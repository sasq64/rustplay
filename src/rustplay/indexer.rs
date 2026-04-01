use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use id3::{Tag, TagLike};
use itertools::Itertools;
use musix::SongInfo;
use std::ops::Bound;
use tantivy::Term;
use tantivy::collector::TopDocs;
use tantivy::query::{AllQuery, QueryParser, RangeQuery, TermQuery};
use tantivy::schema::IndexRecordOption;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use tantivy::{Index, IndexWriter, ReloadPolicy, doc};
use tantivy::{
    IndexReader,
    schema::{Field, OwnedValue, STORED, STRING, Schema, TEXT, TantivyDocument},
};
use walkdir::WalkDir;

use crate::log;
use crate::value::Value;

use super::song::{FileInfo, FileType, SongCollection};

#[inline]
/// Convert ISO-8859-1 slice to utf8 String (For text in SID header)
fn slice_to_string(slice: &[u8]) -> String {
    slice
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

const INITIAL_SONG_COUNT: usize = 100;

static MODLAND_FORMATS: LazyLock<HashSet<&'static str>> =
    LazyLock::new(|| include_str!("modland_formats.txt").lines().collect());

// --- Directory cache types and utilities ---

#[derive(Serialize, Deserialize, Clone)]
enum CachedValue {
    Text(String),
    Number(f64),
}

#[derive(Serialize, Deserialize, Clone)]
struct CachedFileInfo {
    path: PathBuf,
    title: String,
    composer: String,
    meta_data: Vec<(String, CachedValue)>,
}

impl CachedFileInfo {
    fn from_file_info(fi: &FileInfo) -> Self {
        let title = fi.get_title().to_owned();
        let composer = fi.get("composer").to_string();
        let meta_data = fi
            .meta_data
            .iter()
            .map(|(k, v)| {
                let cv = match v {
                    Value::Text(s) => CachedValue::Text(s.clone()),
                    Value::Number(n) => CachedValue::Number(*n),
                    _ => CachedValue::Text(String::new()),
                };
                (k.clone(), cv)
            })
            .collect();
        CachedFileInfo {
            path: fi.path.clone(),
            title,
            composer,
            meta_data,
        }
    }

    fn into_file_info(self) -> FileInfo {
        let meta_data = self
            .meta_data
            .into_iter()
            .map(|(k, v)| {
                let val = match v {
                    CachedValue::Text(s) => Value::Text(s),
                    CachedValue::Number(n) => Value::Number(n),
                };
                (k, val)
            })
            .collect();
        FileInfo {
            path: self.path,
            meta_data,
            ..Default::default()
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DirCache {
    mtime_secs: u64,
    mtime_nanos: u32,
    files: Vec<CachedFileInfo>,
}

fn cache_base_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir()?.join("oldplay-data").join("index-cache");
    std::fs::create_dir_all(&base).ok()?;
    Some(base)
}

fn cache_file_for_dir(base: &Path, dir: &Path) -> PathBuf {
    let dir_str = dir.to_string_lossy();
    // Simple hash: sum of bytes with mixing to avoid collisions
    let hash: u64 = dir_str.bytes().fold(0u64, |h, b| {
        h.wrapping_mul(31).wrapping_add(b as u64)
    });
    base.join(format!("{hash:016x}.bin"))
}

fn dir_mtime(dir: &Path) -> Option<(u64, u32)> {
    let md = std::fs::metadata(dir).ok()?;
    let mtime = md.modified().ok()?;
    let dur = mtime.duration_since(UNIX_EPOCH).ok()?;
    Some((dur.as_secs(), dur.subsec_nanos()))
}

fn load_cache(base: &Path, dir: &Path) -> Option<DirCache> {
    let path = cache_file_for_dir(base, dir);
    let data = std::fs::read(path).ok()?;
    let cache: DirCache = bincode::deserialize(&data).ok()?;
    let (secs, nanos) = dir_mtime(dir)?;
    if cache.mtime_secs == secs && cache.mtime_nanos == nanos {
        Some(cache)
    } else {
        None
    }
}

fn save_cache(base: &Path, dir: &Path, cache: &DirCache) {
    let path = cache_file_for_dir(base, dir);
    if let Ok(data) = bincode::serialize(cache) {
        let _ = std::fs::write(path, data);
    }
}

// --- End cache utilities ---

/// A Tantivy indexer that indexes song files.
pub struct SongIndexer {
    schema: Schema,
    index: Index,
    index_writer: IndexWriter,
    reader: IndexReader,
    title_field: Field,
    composer_field: Field,
    path_field: Field,
    parent_field: Field,
    index_field: Field,

    initial_songs: VecDeque<FileInfo>,
    count: AtomicUsize,
}

fn get_value(doc: &TantivyDocument, field: Field) -> Option<Value> {
    if let Some(path_val) = doc.get_first(field) {
        let owned: OwnedValue = path_val.into();
        return match owned {
            OwnedValue::Str(name) => Some(Value::Text(name)),
            _ => None,
        };
    }
    None
}

fn get_string(doc: &TantivyDocument, field: Field) -> Result<String> {
    if let Some(path_val) = doc.get_first(field) {
        let owned: OwnedValue = path_val.into();
        return match owned {
            OwnedValue::Str(name) => Ok(name),
            _ => Err(anyhow!("")),
        };
    }
    Ok(String::new())
}

impl SongIndexer {
    pub fn new() -> Result<Self> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let composer_field = schema_builder.add_text_field("composer", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", STORED);
        let parent_field = schema_builder.add_text_field("parent", STRING | STORED);
        let index_field = schema_builder.add_u64_field("index", STORED);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema.clone());

        let index_writer: IndexWriter = index.writer(20_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            schema,
            index,
            index_writer,
            reader,
            title_field,
            composer_field,
            path_field,
            parent_field,
            index_field,
            initial_songs: VecDeque::new(),
            count: 0.into(),
        })
    }

    pub fn add_dir(&mut self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .unwrap_or(Path::new(""))
            .to_str()
            .context("Illegal parent path")?
            .to_owned();
        self.index_writer.add_document(doc!(
                self.path_field => path.to_str().context("Illegal path")?
                                    .to_owned(),
                self.parent_field => parent))?;
        Ok(())
    }

    pub fn add_song(&mut self, file_info: &FileInfo) -> Result<()> {
        let count = self.count.fetch_add(1, Ordering::Relaxed);
        let title = file_info.get_title();
        let composer = file_info.get("composer");
        let parent = file_info
            .path
            .parent()
            .unwrap_or(Path::new(""))
            .to_str()
            .context("Illegal parent path")?
            .to_owned();

        self.index_writer.add_document(doc!(
                self.title_field => title,
                self.index_field => count as u64,
                self.composer_field => composer.to_string(),
                self.path_field => file_info.path.to_str().context("Illegal path")?
                                    .to_owned(),
                self.parent_field => parent))?;
        if self.initial_songs.len() < INITIAL_SONG_COUNT {
            self.initial_songs.push_back(file_info.clone());
        }
        Ok(())
    }

    fn add_cached_song(&mut self, cached: &CachedFileInfo) -> Result<()> {
        let count = self.count.fetch_add(1, Ordering::Relaxed);
        let parent = cached
            .path
            .parent()
            .unwrap_or(Path::new(""))
            .to_str()
            .context("Illegal parent path")?
            .to_owned();

        self.index_writer.add_document(doc!(
                self.title_field => cached.title.as_str(),
                self.index_field => count as u64,
                self.composer_field => cached.composer.as_str(),
                self.path_field => cached.path.to_str().context("Illegal path")?
                                    .to_owned(),
                self.parent_field => parent))?;
        if self.initial_songs.len() < INITIAL_SONG_COUNT {
            self.initial_songs.push_back(cached.clone().into_file_info());
        }
        Ok(())
    }

    pub fn add_path(&mut self, song_path: &Path) -> Result<()> {
        // TODO: We can do this less generic but faster, avoiding the hashtable
        let file_info = SongIndexer::identify_song(song_path);
        self.add_song(&file_info)
    }

    /// If `song_path` looks like a Modland path, parse the information in the
    /// path to a `SongInfo`
    fn parse_modland_info(song_path: &Path) -> Option<SongInfo> {
        let segments = song_path
            .ancestors()
            .filter_map(|a| a.file_name())
            .filter_map(|a| a.to_str())
            .collect_vec();

        let title = song_path.file_stem().unwrap_or_default().to_string_lossy();

        let l = segments.len();
        if l >= 3 && MODLAND_FORMATS.contains(&segments[2]) {
            return Some(SongInfo {
                title: title.to_string(),
                composer: segments[1].to_owned(),
                ..SongInfo::default()
            });
        } else if l >= 4 && MODLAND_FORMATS.contains(&segments[3]) {
            if segments[1].starts_with("coop-") {
                let coop = &segments[1][5..];
                let composer = segments[2].to_owned();
                return Some(SongInfo {
                    title: title.to_string(),
                    composer: format!("{composer} + {coop}"),
                    ..SongInfo::default()
                });
            }
            return Some(SongInfo {
                title: title.to_string(),
                game: segments[1].to_owned(),
                composer: segments[2].to_owned(),
                ..SongInfo::default()
            });
        }
        None
    }

    fn identify_song_internal(path: &Path) -> Result<Option<SongInfo>> {
        if let Some(ext) = path.extension() {
            if ext == "sid" {
                let mut buf: [u8; 0x60] = [0; 0x60];
                File::open(path)?.read_exact(&mut buf)?;
                let title = slice_to_string(&buf[0x16..0x36]);
                let composer = slice_to_string(&buf[0x36..0x56]);
                return Ok(Some(SongInfo {
                    title,
                    composer,
                    ..SongInfo::default()
                }));
            } else if ext == "mp3" {
                let tag = Tag::read_from_path(path)?;
                let mut info = SongInfo {
                    format: "MP3".into(),
                    ..SongInfo::default()
                };
                if let Some(artist) = tag.artist() {
                    info.composer = artist.into();
                }
                if let Some(title) = tag.title() {
                    info.title = title.into();
                }
                return Ok(Some(info));
            }
        }
        let res = musix::identify_song(path);
        {
            log!("Checking {:?} => {:?}", path, res);
        }

        Ok(res)
    }

    pub fn identify_song(path: &Path) -> FileInfo {
        let mut meta_data: HashMap<String, Value> = HashMap::new();
        let info = SongIndexer::parse_modland_info(path)
            .or_else(|| SongIndexer::identify_song_internal(path).ok().flatten());

        if let Some(info) = info {
            meta_data.insert("title".into(), info.title.into());
            meta_data.insert("composer".into(), info.composer.into());
            meta_data.insert("game".into(), info.game.into());
            meta_data.insert("format".into(), info.format.into());
        } else {
            let title = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            meta_data.insert("title".into(), title.into());
        }
        if let Ok(md) = std::fs::metadata(path) {
            meta_data.insert("size".into(), Value::Number(md.len() as f64));
        }
        let meta_path = path.with_extension(format!(
            "{}.meta",
            path.extension()
                .map(|e| e.to_string_lossy())
                .unwrap_or_default()
        ));
        if let Ok(file) = File::open(&meta_path) {
            for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
                if let Some((key, value)) = line.split_once('=') {
                    if value.starts_with("\"") {
                        let value = value.trim_start_matches('"').trim_end_matches('"');
                        meta_data.insert(key.to_string(), Value::Text(value.into()));
                    } else {
                        meta_data.insert(
                            key.to_string(),
                            Value::Number(value.parse::<f64>().unwrap()),
                        );
                    }
                }
            }
        }

        FileInfo {
            path: path.to_owned(),
            meta_data,
            ..Default::default()
        }
    }

    pub fn commit(&mut self) -> Result<()> {
        self.index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<FileInfo>> {
        let searcher = self.reader.searcher();
        let mut query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.composer_field]);
        query_parser.set_conjunction_by_default();
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(100_000))?;

        let mut result = Vec::new();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path = get_string(&doc, self.path_field)?;
            let title = get_value(&doc, self.title_field);
            let composer = get_value(&doc, self.composer_field);

            let mut meta_data = HashMap::new();
            if let Some(title) = title {
                meta_data.insert("title".to_owned(), title);
            }
            if let Some(composer) = composer {
                meta_data.insert("composer".to_owned(), composer);
            }

            result.push(FileInfo {
                path: path.into(),
                meta_data,
                ..Default::default()
            });
        }
        Ok(result)
    }

    pub fn search_by_index_range(&mut self, start: u64, end: u64) -> Result<()> {
        let searcher = self.reader.searcher();
        let lower = Term::from_field_u64(self.index_field, start);
        let upper = Term::from_field_u64(self.index_field, end);
        let query = RangeQuery::new(Bound::Included(lower), Bound::Included(upper));
        let top_docs = searcher.search(&query, &TopDocs::with_limit(100_000))?;

        let mut result = Vec::new();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path = get_string(&doc, self.path_field)?;
            let title = get_value(&doc, self.title_field);
            let composer = get_value(&doc, self.composer_field);

            let mut meta_data = HashMap::new();
            if let Some(title) = title {
                meta_data.insert("title".to_owned(), title);
            }
            if let Some(composer) = composer {
                meta_data.insert("composer".to_owned(), composer);
            }

            result.push(FileInfo {
                path: path.into(),
                meta_data,
                ..Default::default()
            });
        }
        Ok(())
    }

    pub fn browse(&self, dir: &Path) -> Result<Vec<FileInfo>> {
        let searcher = self.reader.searcher();
        let dir_str = dir.to_str().context("Illegal dir path")?;
        let query = TermQuery::new(
            Term::from_field_text(self.parent_field, dir_str),
            IndexRecordOption::Basic,
        );
        let top_docs = searcher.search(&query, &TopDocs::with_limit(1_000_000))?;

        let mut dirs: Vec<FileInfo> = Vec::new();
        let mut songs: Vec<FileInfo> = Vec::new();

        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path: PathBuf = get_string(&doc, self.path_field)?.into();
            let title = get_value(&doc, self.title_field);
            let composer = get_value(&doc, self.composer_field);

            let has_title = title.is_some();
            let mut meta_data = HashMap::new();
            if let Some(title) = title {
                meta_data.insert("title".to_owned(), title);
            }
            if let Some(composer) = composer {
                meta_data.insert("composer".to_owned(), composer);
            }

            if has_title {
                songs.push(FileInfo {
                    path,
                    meta_data,
                    ..Default::default()
                });
            } else {
                dirs.push(FileInfo {
                    path,
                    file_type: FileType::Dir,
                    ..Default::default()
                });
            }
        }

        dirs.sort_by(|a, b| a.path.cmp(&b.path));
        songs.sort_by(|a, b| a.path.cmp(&b.path));
        dirs.append(&mut songs);
        Ok(dirs)
    }
}

pub struct IndexedSongs {
    indexer: Arc<Mutex<SongIndexer>>,
}

impl SongCollection for IndexedSongs {
    fn get(&self, index: usize) -> FileInfo {
        let mut i = self.indexer.lock().unwrap();
        let _ = i.search_by_index_range(0, 100);
        i.initial_songs[index].clone()
    }
    fn index_of(&self, song: &FileInfo) -> Option<usize> {
        let i = self.indexer.lock().unwrap();
        for (i, s) in i.initial_songs.iter().enumerate() {
            if song.path() == s.path() {
                return Some(i);
            }
        }
        None
    }

    fn len(&self) -> usize {
        let i = self.indexer.lock().unwrap();
        //i.initial_songs.len()
        i.count.load(Ordering::Relaxed)
    }
}

pub struct RemoteSongIndexer {
    indexer: Arc<Mutex<SongIndexer>>,
    sender: mpsc::Sender<Cmd>,
    index_thread: Option<JoinHandle<()>>,
    is_working: Arc<AtomicBool>,
}

#[derive(Debug, Clone, PartialEq)]
enum Cmd {
    AddPath(PathBuf),
    Quit,
}

impl Drop for RemoteSongIndexer {
    fn drop(&mut self) {
        self.is_working.store(false, Ordering::Relaxed);
        let _ = self.sender.send(Cmd::Quit {});
        if let Some(t) = self.index_thread.take() {
            let _ = t.join();
        }
    }
}

impl RemoteSongIndexer {
    #[inline]
    #[allow(clippy::unwrap_used)]
    fn lock(&self) -> MutexGuard<'_, SongIndexer> {
        self.indexer.lock().unwrap()
    }

    fn run(
        indexer: &Arc<Mutex<SongIndexer>>,
        working: &Arc<AtomicBool>,
        rx: &Receiver<Cmd>,
    ) -> Result<()> {
        let non_songs: HashSet<&str> = [
            "d71", "d81", "dfi", "d64", "1st", "exe", "hvs", "txt", "faq", "md5",
        ]
        .into();

        #[allow(clippy::unwrap_used)]
        let lock = || indexer.lock().unwrap();

        loop {
            let cmd = rx.recv()?;
            match cmd {
                Cmd::Quit => {
                    break Ok(());
                }
                Cmd::AddPath(path) => {
                    let mut now = Instant::now();
                    let cache_base = cache_base_dir();
                    working.store(true, Ordering::Relaxed);

                    // With contents_first(true), all files in a directory are
                    // yielded before the directory entry itself.
                    let mut pending_files: Vec<walkdir::DirEntry> = Vec::new();

                    for entry in WalkDir::new(path).contents_first(true) {
                        if !working.load(Ordering::Relaxed) {
                            break;
                        }
                        let p = entry?;

                        if p.file_type().is_dir() {
                            if let Some(ref base) = cache_base
                                && let Some(cached) = load_cache(base, p.path())
                            {
                                // Cache hit: add directly to Tantivy
                                for cf in &cached.files {
                                    lock().add_cached_song(cf)?;
                                }
                            } else {
                                // Cache miss: identify songs and save cache
                                let mut cache_entries = Vec::new();
                                for file_entry in pending_files.drain(..) {
                                    if let Some(ext) = file_entry.path().extension() {
                                        let ext = ext.to_string_lossy().to_lowercase();
                                        if non_songs.contains(ext.as_str()) {
                                            continue;
                                        }
                                    }
                                    if file_entry.file_type().is_file()
                                        && musix::can_handle(file_entry.path())?
                                    {
                                        let file_info =
                                            SongIndexer::identify_song(file_entry.path());
                                        lock().add_song(&file_info)?;
                                        cache_entries
                                            .push(CachedFileInfo::from_file_info(&file_info));
                                    }
                                }
                                if let Some(ref base) = cache_base
                                    && let Some((secs, nanos)) = dir_mtime(p.path())
                                {
                                    let cache = DirCache {
                                        mtime_secs: secs,
                                        mtime_nanos: nanos,
                                        files: cache_entries,
                                    };
                                    save_cache(base, p.path(), &cache);
                                }
                            }
                            pending_files.clear();
                            lock().add_dir(p.path())?;
                        } else {
                            pending_files.push(p);
                        }

                        if now.elapsed() > Duration::from_millis(1000) {
                            lock().commit()?;
                            now += Duration::from_millis(1000);
                        }
                    }
                    lock().commit()?;
                    working.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn new() -> Result<RemoteSongIndexer> {
        let indexer = Arc::new(Mutex::new(SongIndexer::new()?));
        let (sender, rx) = mpsc::channel::<Cmd>();

        let working = Arc::new(AtomicBool::new(false));
        let index_thread = Some({
            let indexer = indexer.clone();
            let working = working.clone();
            thread::Builder::new()
                .name("index_thread".into())
                .spawn(move || {
                    RemoteSongIndexer::run(&indexer, &working, &rx).expect("Fail");
                })?
        });
        Ok(RemoteSongIndexer {
            indexer,
            sender,
            index_thread,
            is_working: working,
        })
    }

    pub fn add_path(&self, path: &Path) -> Result<()> {
        self.is_working.store(true, Ordering::Relaxed);
        self.sender.send(Cmd::AddPath(path.to_owned()))?;
        Ok(())
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<FileInfo>> {
        let mut indexer = self.lock();
        indexer.search(query)
    }

    pub fn search_by_index_range(&mut self, start: u64, end: u64) -> Result<()> {
        let mut indexer = self.lock();
        indexer.search_by_index_range(start, end)?;
        Ok(())
    }

    pub fn browse(&self, dir: &Path) -> Result<Vec<FileInfo>> {
        let indexer = self.lock();
        indexer.browse(dir)
    }

    pub fn index_count(&self) -> usize {
        let i = self.lock();
        i.count.load(Ordering::Relaxed)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub(crate) fn get_all_songs(&self) -> Rc<dyn SongCollection> {
        Rc::new(IndexedSongs {
            indexer: self.indexer.clone(),
        })
    }

    pub(crate) fn working(&self) -> bool {
        self.is_working.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::{Path, PathBuf};

    use walkdir::WalkDir;

    use crate::rustplay::indexer::RemoteSongIndexer;
    use crate::rustplay::song::{FileInfo, FileType};

    use super::SongIndexer;

    #[test]
    fn identify_works() {
        let path: PathBuf = "music/C64/Ark_Pandora.sid".into();
        let info = SongIndexer::identify_song_internal(&path).unwrap().unwrap();
        assert_eq!(info.title, "Ark Pandora");
    }

    #[test]
    fn normal_search_works() {
        let mut indexer = SongIndexer::new().unwrap();
        let path: PathBuf = "music/C64/Ark_Pandora.sid".into();
        indexer.add_path(&path).unwrap();
        indexer.commit().unwrap();
        let result = indexer.search("pandora").unwrap();
        assert!(result.len() == 1);

        let path: PathBuf = "/MODLAND/Fasttracker 2/Purple Motion/sil forever.xm".into();
        indexer.add_path(&path).unwrap();
        indexer.commit().unwrap();
        let result = indexer.search("purple motion").unwrap();
        assert_eq!(result.len(), 1);

        let path: PathBuf = "music".into();
        for entry in WalkDir::new(path) {
            let e = entry.unwrap();
            if e.path().is_file() {
                indexer.add_path(e.path()).unwrap();
            }
        }
        indexer.commit().unwrap();
        let result = indexer.search("hubbard").unwrap();
        assert!(result.len() > 3);
        let result = indexer.search("horace").unwrap();
        assert!(result.len() == 1);
        let result = indexer.search("ninja").unwrap();
        assert!(result.len() >= 3);
        let result = indexer.search("xywizoqp").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn browse_works() {
        let mut indexer = SongIndexer::new().unwrap();
        for entry in WalkDir::new("music") {
            let e = entry.unwrap();
            if e.path().is_dir() {
                indexer.add_dir(e.path()).unwrap();
            } else if e.path().is_file() {
                indexer.add_path(e.path()).unwrap();
            }
        }
        indexer.commit().unwrap();

        // Browse root "music" dir
        let entries = indexer.browse(Path::new("music")).unwrap();
        let dirs: Vec<&FileInfo> = entries
            .iter()
            .filter(|e| e.file_type == FileType::Dir)
            .collect();
        let songs: Vec<&FileInfo> = entries
            .iter()
            .filter(|e| e.file_type == FileType::Song)
            .collect();
        assert!(dirs.iter().any(|d| &d.path == "music/C64"));
        assert!(dirs.iter().any(|d| &d.path == "music/MODS"));
        assert!(!songs.is_empty());
        assert!(songs.iter().all(|s| !s.path().starts_with("music/C64")));

        // Browse music/C64
        let entries = indexer.browse(Path::new("music/C64")).unwrap();
        let dirs: Vec<&FileInfo> = entries
            .iter()
            .filter(|e| e.file_type == FileType::Dir)
            .collect();
        let songs: Vec<&FileInfo> = entries
            .iter()
            .filter(|e| e.file_type == FileType::Song)
            .collect();
        assert!(dirs.is_empty());
        assert!(songs.len() > 40);
    }

    #[test]
    fn threaded_search_works() {
        let data = Path::new("data");
        assert!(data.is_dir());
        musix::init(data).unwrap();
        let mut indexer = RemoteSongIndexer::new().unwrap();
        let path: PathBuf = "music".into();
        indexer.add_path(&path).unwrap();
        while indexer.working() {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let result = indexer.search("hymn").unwrap();
        assert_eq!(result.len(), 1);
        let result = indexer.search("ninja").unwrap();
        assert_eq!(result.len(), 3);
    }
}
