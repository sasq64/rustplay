#![allow(dead_code)]
#![allow(clippy::unwrap_used)]

use std::error::Error;
use std::path::Path;

use musix::SongInfo;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyError, doc};
use tantivy::{IndexReader, schema::*};

pub struct Indexer {
    schema: Schema,
    index: Index,
    index_writer: IndexWriter,
    reader: IndexReader,
    pub result: Vec<String>,

    title_field: Field,
    composer_field: Field,
    path_field: Field,
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
        })
    }

    pub fn add(&mut self, song_path: &Path, info: &SongInfo) {
        self.index_writer
            .add_document(doc!(
                self.title_field => info.title.clone(),
                self.composer_field => info.composer.clone(),
                self.path_field => song_path.to_str().unwrap().to_owned()))
            .unwrap();
    }

    pub fn commit(&mut self) {
        self.index_writer.commit().unwrap();
    }

    pub fn search(&mut self, query: &str) -> Result<(), TantivyError> {
        let searcher = self.reader.searcher();
        let query_parser =
            QueryParser::for_index(&self.index, vec![self.title_field, self.composer_field]);
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        self.result.clear();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let title_val = doc.get_first(self.title_field).unwrap();
            let path_val = doc.get_first(self.path_field).unwrap();
            let name = match title_val {
                OwnedValue::Str(name) => name,
                _ => "",
            };
            let path = match path_val {
                OwnedValue::Str(name) => name,
                _ => "",
            };
            self.result.push(path.into());
        }
        Ok(())
    }
}
