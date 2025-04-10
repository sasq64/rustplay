#![allow(dead_code)]
#![allow(clippy::unwrap_used)]

use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use musix::SongInfo;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyError, doc};
use tantivy::{IndexReader, schema::*};
use walkdir::WalkDir;

use crate::value;

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

    song_list: VecDeque<FileInfo>,
}

impl Indexer {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let mut schema_builder = Schema::builder();
        let title_field = schema_builder.add_text_field("title", TEXT | STORED);
        let composer_field = schema_builder.add_text_field("composer", TEXT | STORED);
        let path_field = schema_builder.add_text_field("path", STORED);
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
            song_list: VecDeque::new(),
        })
    }

    pub fn add_with_info(&mut self, song_path: &Path, info: &SongInfo) {
        //println!("INFO: {} {}", info.title, info.composer);
        self.index_writer
            .add_document(doc!(
                self.title_field => info.title.clone(),
                self.composer_field => info.composer.clone(),
                self.path_field => song_path.to_str().unwrap().to_owned()))
            .unwrap();
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
    }

    pub fn next(&mut self) -> Option<FileInfo> {
        if self.song_list.is_empty() {
            return None;
        }
        self.song_list.pop_front()
    }

    pub fn commit(&mut self) {
        self.index_writer.commit().unwrap();
        self.reader.reload().unwrap();
    }

    pub fn search(&mut self, query: &str) -> Result<(), TantivyError> {
        let searcher = self.reader.searcher();
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.composer_field]);
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        //println!("Found {}", top_docs.len());
        self.result.clear();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let path_val = doc.get_first(self.path_field).unwrap();
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
    pub fn new() -> Result<RemoteIndexer, Box<dyn Error>> {
        let indexer = Arc::new(Mutex::new(Indexer::new()?));
        let (sender, rx) = mpsc::channel::<Cmd>();

        let index_thread = Some({
            let indexer = indexer.clone();
            thread::spawn(move || {
                loop {
                    let cmd = rx.recv().unwrap();
                    match cmd {
                        Cmd::AddPath(path) => {
                            let mut now = Instant::now();
                            for entry in WalkDir::new(path) {
                                let p = entry.unwrap();
                                if p.path().is_file() {
                                    if let Some(info) = musix::identify_song(p.path()) {
                                        let mut il = indexer.lock().unwrap();
                                        il.add_with_info(p.path(), &info);
                                    }
                                }
                                if Instant::now() - now > Duration::from_millis(500) {
                                    indexer.lock().unwrap().commit();
                                    now += Duration::from_millis(500);
                                }
                            }
                            indexer.lock().unwrap().commit();
                        }
                    }
                }
            })
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

    pub fn search(&mut self, query: &str) -> Result<Vec<String>, TantivyError> {
        let mut indexer = self.indexer.lock()?;
        indexer.search(query)?;
        Ok(indexer.result.to_owned())
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
    fn search_works() {
        let mut indexer = Indexer::new().unwrap();
        let path: PathBuf = "../musicplayer/music/C64/Ark_Pandora.sid".into();
        let info = musix::identify_song(&path).unwrap();
        indexer.add_with_info(&path, &info);
        indexer.commit();
        indexer.search("pandora").unwrap();
        assert!(indexer.result.len() == 1);

        let path: PathBuf = "../musicplayer/music/C64".into();
        for entry in WalkDir::new(path) {
            let e = entry.unwrap();
            if e.path().is_file() {
                let info = musix::identify_song(e.path()).unwrap();
                indexer.add_with_info(e.path(), &info);
            }
        }
        indexer.commit();
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
