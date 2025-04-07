#![allow(dead_code)]

use std::error::Error;

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
}

impl Indexer {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let mut schema_builder = Schema::builder();
        schema_builder.add_text_field("title", TEXT | STORED);
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
        })
    }

    pub fn add(&mut self, song_title: &str) {
        let title = self.schema.get_field("title").unwrap();
        self.index_writer
            .add_document(doc!(title => song_title))
            .unwrap();
    }

    pub fn commit(&mut self) {
        self.index_writer.commit().unwrap();
    }

    pub fn search(&mut self, query: &str) -> Result<i32, TantivyError> {
        let searcher = self.reader.searcher();
        let title = self.schema.get_field("title")?;
        let query_parser = QueryParser::for_index(&self.index, vec![title]);
        let query = query_parser.parse_query(query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(10))?;
        self.result.clear();
        for (_score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address)?;
            let x = doc.get_first(title).unwrap();
            let name = match x {
                OwnedValue::Str(name) => name,
                _ => "",
            };
            self.result.push(name.into());
        }
        Ok(0)
    }
}
