use std::{
    collections::HashMap,
    ops::Index,
    path::{Path, PathBuf},
};

use super::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    pub path: PathBuf,
    pub meta_data: HashMap<String, Value>,
}

impl FileInfo {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, what: &str) -> &Value {
        self.meta_data.get(what).unwrap_or(&Value::Unknown())
    }

    pub fn title_and_composer(&self) -> String {
        let title = self.get("title");
        let composer = self.get("composer");
        format!("{title} / {composer}")
    }

    pub fn full_song_name(&self) -> String {
        let title = self.get("title");
        let composer = self.get("composer");
        let file_name = self.path.file_name().map(|s| s.to_string_lossy());
        if composer != &Value::Unknown() {
            if let Some(ext) = self.path.extension() {
                return format!("{title} / {composer} [{}]", ext.to_string_lossy());
            }
            return format!("{title} / {composer}");
        }
        if let Some(file_name) = file_name {
            return file_name.to_string();
        }
        "???".into()
    }

    pub fn title(&self) -> Option<&str> {
        if let Some(Value::Text(title)) = self.meta_data.get("title") {
            return Some(title);
        }
        None
    }
}

pub struct SongArray {
    pub songs: Vec<FileInfo>,
}

pub trait SongCollection {
    fn get(&self, index: usize) -> FileInfo;
    fn index_of(&self, song: &FileInfo) -> Option<usize>;
    fn len(&self) -> usize;
}

impl SongCollection for SongArray {
    fn get(&self, index: usize) -> FileInfo {
        self.songs.index(index).clone()
    }

    fn len(&self) -> usize {
        self.songs.len()
    }

    fn index_of(&self, song: &FileInfo) -> Option<usize> {
        for (i, s) in self.songs.iter().enumerate() {
            if song.path() == s.path() {
                return Some(i);
            }
        }
        None
    }
}
