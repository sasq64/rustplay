#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use musix::SongInfo;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

use anyhow::Context;
use anyhow::Result;
use tantivy::{Index, IndexWriter, ReloadPolicy, doc};
use tantivy::{IndexReader, schema::*};
use walkdir::WalkDir;

use crate::value;

#[inline]
/// Convert 8 bit unicode to utf8 String
fn slice_to_string(slice: &[u8]) -> String {
    slice
        .iter()
        .take_while(|&&b| b != 0)
        .map(|&b| b as char)
        .collect()
}

pub struct FileInfo {
    path: PathBuf,
    meta_data: HashMap<String, value::Value>,
}

impl FileInfo {
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn title(&self) -> Option<String> {
        if let Some(value::Value::Text(title)) = self.meta_data.get("title") {
            return Some(title.to_owned());
        }
        None
    }
}

pub trait PlayList {
    fn next() -> Option<FileInfo>;
}

pub struct Indexer {
    schema: Schema,
    index: Index,
    index_writer: IndexWriter,
    reader: IndexReader,
    pub result: Vec<String>,

    title_field: Field,
    composer_field: Field,
    path_field: Field,
    file_field: Field,

    song_list: VecDeque<FileInfo>,
    count: AtomicUsize,
}

impl Indexer {
    pub fn new() -> Result<Self> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let composer_field = schema_builder.add_text_field("composer", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", STORED);
        let file_field = schema_builder.add_text_field("file_name", TEXT);
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
            result: Vec::new(),
            title_field,
            composer_field,
            path_field,
            file_field,
            song_list: VecDeque::new(),
            count: 0.into(),
        })
    }

    pub fn add_path(&mut self, song_path: &Path) -> Result<()> {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.index_writer.add_document(doc!(
                self.file_field => song_path.file_name().context("No filename")?.to_str().unwrap(),
                self.path_field => song_path.to_str().unwrap().to_owned()))?;
        if self.song_list.len() < 100 {
            let file_info = FileInfo {
                path: song_path.into(),
                meta_data: HashMap::new(),
            };
            self.song_list.push_back(file_info);
        }
        Ok(())
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

    pub fn add_with_info(&mut self, song_path: &Path, info: &SongInfo) -> Result<()> {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.index_writer.add_document(doc!(
                self.title_field => info.title.clone(),
                self.composer_field => info.composer.clone(),
                self.file_field => song_path.file_name().context("No filename")?
                                    .to_str().unwrap(),
                self.path_field => song_path.to_str().context("Illegal path")?
                                    .to_owned()))?;
        if self.song_list.len() < 100 {
            let file_info = FileInfo {
                path: song_path.into(),
                meta_data: HashMap::from([(
                    "title".into(),
                    value::Value::Text(info.title.clone()),
                )]),
            };
            self.song_list.push_back(file_info);
        }
        Ok(())
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
        let query_parser = QueryParser::for_index(
            &self.index,
            vec![self.title_field, self.composer_field, self.file_field],
        );
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        //println!("Found {}", top_docs.len());
        self.result.clear();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path_val = doc.get_first(self.path_field).context("No path field")?;
            let path = match path_val {
                OwnedValue::Str(name) => name,
                _ => "",
            };
            self.result.push(path.into());
        }
        Ok(())
    }
}

pub struct RemoteIndexer {
    indexer: Arc<Mutex<Indexer>>,
    sender: mpsc::Sender<Cmd>,
    index_thread: Option<JoinHandle<()>>,
}

enum Cmd {
    AddPath(PathBuf),
}

impl RemoteIndexer {
    fn run(indexer: Arc<Mutex<Indexer>>, rx: Receiver<Cmd>) -> Result<()> {
        loop {
            let cmd = rx.recv()?;
            match cmd {
                Cmd::AddPath(path) => {
                    let mut now = Instant::now();
                    for entry in WalkDir::new(path) {
                        let p = entry.unwrap();
                        if p.file_type().is_file() {
                            if let Some(info) = Indexer::identify_song(p.path())? {
                                let mut il = indexer.lock().unwrap();
                                il.add_with_info(p.path(), &info)?;
                            } else {
                                let mut il = indexer.lock().unwrap();
                                il.add_path(p.path())?;
                            }
                        }
                        if now.elapsed() > Duration::from_millis(1000) {
                            indexer.lock().unwrap().commit()?;
                            now += Duration::from_millis(1000);
                        }
                    }
                    indexer.lock().unwrap().commit()?;
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

    pub fn add_path(&self, path: &Path) {
        self.sender.send(Cmd::AddPath(path.to_owned())).unwrap();
    }

    pub fn next(&self) -> Option<FileInfo> {
        return self.indexer.lock().unwrap().next();
    }

    pub fn search(&mut self, query: &str) -> Result<Vec<String>> {
        let mut indexer = self.indexer.lock().unwrap();
        indexer.search(query)?;
        Ok(indexer.result.clone())
    }

    pub fn index_count(&self) -> usize {
        let i = self.indexer.lock().unwrap();
        i.count.load(Ordering::Relaxed)
    }

    pub fn commit(&self) {}
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
    fn search_works() {
        let mut indexer = Indexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music/C64/Ark_Pandora.sid".into();
        let info = musix::identify_song(&path).unwrap();
        indexer.add_with_info(&path, &info).unwrap();
        indexer.commit().unwrap();
        indexer.search("pandora").unwrap();
        assert!(indexer.result.len() == 1);

        let path: PathBuf = "../musicplayer/music/C64".into();
        for entry in WalkDir::new(path) {
            let e = entry.unwrap();
            if e.path().is_file() {
                let info = musix::identify_song(e.path()).unwrap();
                indexer.add_with_info(e.path(), &info).unwrap();
            }
        }
        indexer.commit().unwrap();
        indexer.search("hubbard").unwrap();
        assert!(indexer.result.len() > 3);
        indexer.search("ninja").unwrap();
        assert!(indexer.result.len() >= 3);
        indexer.search("xywizoqp").unwrap();
        assert!(indexer.result.is_empty());
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn threaded_search_works() {
        let mut indexer = RemoteIndexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music/C64".into();
        indexer.add_path(&path);
        std::thread::sleep(std::time::Duration::from_millis(500));
        let result = indexer.search("pandora").unwrap();
        assert!(result.len() == 1);
        let result = indexer.search("ninja").unwrap();
        assert!(result.len() == 3);
    }
}
