use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use itertools::Itertools;
use musix::SongInfo;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use tantivy::{Index, IndexWriter, ReloadPolicy, doc};
use tantivy::{IndexReader, schema::*};
use walkdir::WalkDir;

use crate::value::Value;

use super::song::{FileInfo, SongArray, SongCollection};

#[inline]
/// Convert 8 bit unicode to utf8 String
fn slice_to_string(slice: &[u8]) -> String {
    slice
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

pub trait PlayList {
    fn next() -> Option<FileInfo>;
}

pub struct Indexer {
    schema: Schema,
    index: Index,
    index_writer: IndexWriter,
    reader: IndexReader,
    result: Vec<FileInfo>,

    title_field: Field,
    composer_field: Field,
    path_field: Field,

    song_list: VecDeque<FileInfo>,
    count: AtomicUsize,
    working: AtomicBool,
    modland_formats: HashSet<&'static str>,
}

fn get_value(doc: &TantivyDocument, field: Field) -> Option<Value> {
    if let Some(path_val) = doc.get_first(field) {
        return match path_val {
            OwnedValue::Str(name) => Some(Value::Text(name.to_owned())),
            _ => None,
        };
    }
    None
}

fn get_string(doc: &TantivyDocument, field: Field) -> Result<String> {
    if let Some(path_val) = doc.get_first(field) {
        return match path_val {
            OwnedValue::Str(name) => Ok(name.to_owned()),
            _ => Err(anyhow!("")),
        };
    }
    Ok("".into())
}

impl Indexer {
    pub fn new() -> Result<Self> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let composer_field = schema_builder.add_text_field("composer", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", STORED);
        //let file_field = schema_builder.add_text_field("file_name", TEXT);
        let schema = schema_builder.build();

        let index = Index::create_in_ram(schema.clone());

        let index_writer: IndexWriter = index.writer(20_000_000)?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        let modland_formats: HashSet<&str> = include_str!("modland_formats.txt").lines().collect();

        Ok(Self {
            schema,
            index,
            index_writer,
            reader,
            result: Vec::new(),
            title_field,
            composer_field,
            path_field,
            //file_field,
            song_list: VecDeque::new(),
            count: 0.into(),
            working: AtomicBool::new(false),
            modland_formats,
        })
    }

    pub fn add_with_info(&mut self, song_path: &Path, info: &SongInfo) -> Result<()> {
        self.count.fetch_add(1, Ordering::Relaxed);

        let file_name = song_path.file_stem().unwrap_or_default().to_string_lossy();
        let title = if !info.title.is_empty() {
            if !info.game.is_empty() {
                format!("{} ({})", info.game, info.title)
            } else {
                info.title.clone()
            }
        } else if !info.game.is_empty() {
            info.game.clone()
        } else {
            file_name.to_string()
        };

        self.index_writer.add_document(doc!(
                self.title_field => title,
                self.composer_field => info.composer.clone(),
                //self.file_field => song_path.file_name().context("No filename")?
                //                    .to_str().unwrap(),
                self.path_field => song_path.to_str().context("Illegal path")?
                                    .to_owned()))?;
        if self.song_list.len() < 100 {
            let file_info = FileInfo {
                path: song_path.into(),
                meta_data: HashMap::from([
                    ("title".into(), Value::Text(info.title.clone())),
                    ("composer".into(), Value::Text(info.composer.clone())),
                ]),
            };
            self.song_list.push_back(file_info);
        }
        Ok(())
    }

    pub fn add_path(&mut self, song_path: &Path) -> Result<()> {
        if let Some(info) = self.parse_modland_info(song_path) {
            self.add_with_info(song_path, &info)?;
            return Ok(());
        }

        let title = song_path.file_stem().unwrap_or_default().to_string_lossy();
        self.count.fetch_add(1, Ordering::Relaxed);
        self.index_writer.add_document(doc!(
                self.title_field =>  title.to_string(),
                self.path_field => song_path.to_string_lossy().to_string()))?;
        if self.song_list.len() < 100 {
            let file_info = FileInfo {
                path: song_path.into(),
                meta_data: HashMap::new(),
            };
            self.song_list.push_back(file_info);
        }
        Ok(())
    }

    /// If `song_path` looks like a Modland path, parse the information in the
    /// path to a `SongInfo`
    fn parse_modland_info(&self, song_path: &Path) -> Option<SongInfo> {
        let segments = song_path
            .ancestors()
            .filter_map(|a| a.file_name())
            .filter_map(|a| a.to_str())
            .collect_vec();

        let title = song_path.file_stem().unwrap_or_default().to_string_lossy();

        let l = segments.len();
        if l >= 3 && self.modland_formats.contains(&segments[2]) {
            return Some(SongInfo {
                title: title.to_string(),
                composer: segments[1].to_owned(),
                ..SongInfo::default()
            });
        } else if l >= 4 && self.modland_formats.contains(&segments[3]) {
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

    pub fn identify_song(path: &Path) -> Result<Option<SongInfo>> {
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
            }
        }
        Ok(musix::identify_song(path))
    }

    pub fn next(&mut self) -> Option<FileInfo> {
        if self.song_list.is_empty() {
            return None;
        }
        self.song_list.pop_front()
    }

    pub fn commit(&mut self) -> Result<()> {
        self.index_writer.commit()?;
        self.reader.reload()?;
        Ok(())
    }

    pub fn search(&mut self, query: &str) -> Result<()> {
        let searcher = self.reader.searcher();
        let mut query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.composer_field]);
        query_parser.set_conjunction_by_default();
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10000))?;

        self.result.clear();
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

            self.result.push(FileInfo {
                path: path.into(),
                meta_data,
            });
        }
        Ok(())
    }
}

pub struct IndexedSongs {
    indexer: Arc<Mutex<Indexer>>,
}

impl SongCollection for IndexedSongs {
    fn get(&self, index: usize) -> FileInfo {
        let i = self.indexer.lock().unwrap();
        return i.song_list[index].clone();
    }
    fn index_of(&self, song: &FileInfo) -> Option<usize> {
        let i = self.indexer.lock().unwrap();
        for (i, s) in i.song_list.iter().enumerate() {
            if song.path() == s.path() {
                return Some(i);
            }
        }
        None
    }

    fn len(&self) -> usize {
        let i = self.indexer.lock().unwrap();
        i.song_list.len()
    }
}

pub struct RemoteIndexer {
    indexer: Arc<Mutex<Indexer>>,
    sender: mpsc::Sender<Cmd>,
    index_thread: Option<JoinHandle<()>>,
}

#[derive(Debug, Clone, PartialEq)]
enum Cmd {
    AddPath(PathBuf),
}

impl RemoteIndexer {
    #[inline]
    #[allow(clippy::unwrap_used)]
    fn lock(&self) -> MutexGuard<'_, Indexer> {
        self.indexer.lock().unwrap()
    }

    fn run(indexer: Arc<Mutex<Indexer>>, rx: Receiver<Cmd>) -> Result<()> {
        let non_songs: HashSet<String> = [
            "d71", "d81", "dfi", "d64", "1st", "exe", "hvs", "txt", "faq", "md5",
        ]
        .into_iter()
        .map(str::to_string)
        .collect();

        #[allow(clippy::unwrap_used)]
        let lock = || indexer.lock().unwrap();

        loop {
            let cmd = rx.recv()?;
            match cmd {
                Cmd::AddPath(path) => {
                    let mut now = Instant::now();
                    lock().working.store(true, Ordering::Relaxed);
                    for entry in WalkDir::new(path) {
                        let p = entry?;
                        if let Some(ext) = p.path().extension() {
                            let ext = ext.to_string_lossy().to_lowercase();
                            if non_songs.contains(&ext) {
                                continue;
                            }
                        }
                        if p.file_type().is_file() && musix::can_handle(p.path())? {
                            if let Some(info) = Indexer::identify_song(p.path())? {
                                lock().add_with_info(p.path(), &info)?;
                            } else {
                                lock().add_path(p.path())?;
                            }
                        }
                        if now.elapsed() > Duration::from_millis(1000) {
                            lock().commit()?;
                            now += Duration::from_millis(1000);
                        }
                    }
                    lock().commit()?;
                    lock().working.store(false, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn new() -> Result<RemoteIndexer> {
        let indexer = Arc::new(Mutex::new(Indexer::new()?));
        let (sender, rx) = mpsc::channel::<Cmd>();

        let index_thread = Some({
            let indexer = indexer.clone();
            thread::Builder::new()
                .name("index_thread".into())
                .spawn(move || {
                    RemoteIndexer::run(indexer, rx).expect("Fail");
                })?
        });
        Ok(RemoteIndexer {
            indexer,
            sender,
            index_thread,
        })
    }

    pub fn add_path(&self, path: &Path) -> Result<()> {
        self.sender.send(Cmd::AddPath(path.to_owned()))?;
        Ok(())
    }

    pub fn next(&self) -> Option<FileInfo> {
        return self.lock().next();
    }

    pub fn search(&mut self, query: &str) -> Result<()> {
        let mut indexer = self.lock();
        indexer.search(query)?;
        Ok(())
    }

    pub fn index_count(&self) -> usize {
        let i = self.lock();
        i.count.load(Ordering::Relaxed)
    }

    pub fn commit(&self) {}

    pub fn get_songs(&self, start: usize, stop: usize) -> Result<Vec<FileInfo>> {
        let indexer = self.lock();
        let song_len = indexer.result.len();
        if song_len == 0 {
            return Ok(Vec::new());
        }
        if stop > song_len {
            return Ok(indexer.result[start..song_len].to_vec());
        }
        Ok(indexer.result[start..stop].to_vec())
    }

    pub(crate) fn get_song(&self, index: usize) -> Option<FileInfo> {
        let indexer = self.lock();
        indexer.result.get(index).cloned()
    }

    pub fn song_len(&self) -> usize {
        let indexer = self.lock();
        indexer.result.len()
    }

    pub fn get_song_result(&self) -> Option<Box<dyn SongCollection>> {
        let result = self.lock().result.clone();
        Some(Box::new(SongArray { songs: result }))
    }

    pub(crate) fn get_all_songs(&self) -> Option<Box<dyn SongCollection>> {
        Some(Box::new(IndexedSongs {
            indexer: self.indexer.clone(),
        }))
    }

    pub(crate) fn working(&self) -> bool {
        self.indexer.lock().unwrap().working.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use walkdir::WalkDir;

    use crate::rustplay::indexer::RemoteIndexer;

    use super::Indexer;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn identify_works() {
        let path: PathBuf = "../musicplayer/music/C64/Ark_Pandora.sid".into();
        let info = Indexer::identify_song(&path).unwrap().unwrap();
        assert_eq!(info.title, "Ark Pandora");
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn normal_search_works() {
        let mut indexer = Indexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music/C64/Ark_Pandora.sid".into();
        let info = musix::identify_song(&path).unwrap();
        indexer.add_with_info(&path, &info).unwrap();
        indexer.commit().unwrap();
        indexer.search("pandora").unwrap();
        assert!(indexer.result.len() == 1);

        let path: PathBuf =
            "/home/sasq/Music/MODLAND/Fasttracker 2/Purple Motion/sil forever.xm".into();
        indexer.add_path(&path).unwrap();
        indexer.commit().unwrap();
        indexer.search("purple motion").unwrap();
        assert_eq!(indexer.result.len(), 1);

        let path: PathBuf = "../musicplayer/music".into();
        for entry in WalkDir::new(path) {
            let e = entry.unwrap();
            if e.path().is_file() {
                if let Some(info) = musix::identify_song(e.path()) {
                    indexer.add_with_info(e.path(), &info).unwrap();
                } else {
                    indexer.add_path(e.path()).unwrap();
                }
            }
        }
        indexer.commit().unwrap();
        indexer.search("hubbard").unwrap();
        assert!(indexer.result.len() > 3);
        indexer.search("horace").unwrap();
        assert!(indexer.result.len() == 1);
        indexer.search("ninja").unwrap();
        assert!(indexer.result.len() >= 3);
        indexer.search("xywizoqp").unwrap();
        assert!(indexer.result.is_empty());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn threaded_search_works() {
        let mut indexer = RemoteIndexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music".into();
        indexer.add_path(&path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        indexer.search("horace").unwrap();
        assert!(indexer.song_len() == 1);
        indexer.search("ninja").unwrap();
        assert!(indexer.song_len() == 3);
    }
}
