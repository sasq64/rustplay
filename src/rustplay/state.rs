use std::collections::{HashMap, VecDeque};

use crate::value::Value;

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum InputMode {
    #[default]
    Main,
    SearchInput,
    ResultScreen,
}

pub(crate) enum Msg {
    Err(String),
    Info(String),
}

#[derive(Default)]
pub struct State {
    pub changed: bool,
    pub meta: HashMap<String, Value>,
    pub song: i32,
    pub songs: i32,
    pub len_msec: usize,
    pub done: bool,
    pub show_error: i32,
    pub mode: InputMode,
    pub last_mode: InputMode,
    pub quit: bool,
    pub use_color: bool,
    pub messages: VecDeque<Msg>,
    pub player_started: bool,
    pub width: i32,
    pub height: i32,
}

impl State {
    pub fn info(&mut self, text: impl Into<String>) {
        self.messages.push_back(Msg::Info(text.into()));
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.messages.push_back(Msg::Err(text.into()));
    }

    pub fn update_meta(&mut self, meta: &str, val: Value) {
        match val {
            Value::Number(n) => {
                self.changed = true;
                let i = n as i32;
                match meta {
                    "done" => self.done = true,
                    "length" => {
                        self.meta.insert(
                            "len".to_owned(),
                            if i > 0 {
                                Value::Text(format!("{:02}:{:02}", i / 60, i % 60).to_owned())
                            } else {
                                Value::Text("??:??".to_owned())
                            },
                        );
                    }
                    "song" => {
                        self.song = i;
                        self.meta.insert("isong".into(), (i + 1).into());
                    }
                    "songs" => self.songs = i,
                    &_ => {}
                }
            }
            Value::Text(ref t) => {
                if t.is_empty() {
                    return;
                }
                self.changed = true;
            }
            Value::Error(ref e) => {
                self.messages.push_back(Msg::Err((*e).to_string()));
            }
            Value::State(_) | Value::Data(_) | Value::Instant(_) | Value::Unknown => {}
        }

        self.meta.insert(meta.to_owned(), val);
    }

    pub fn get_meta(&self, name: &str) -> &str {
        if let Some(Value::Text(t)) = self.meta.get(name) {
            return t;
        }
        ""
    }

    pub fn get_meta_or<'a>(&'a self, name: &str, def: &'a str) -> &'a str {
        if let Some(Value::Text(t)) = self.meta.get(name) {
            return t;
        }
        def
    }

    pub fn set_meta(&mut self, what: &str, value: String) {
        self.meta.insert(what.into(), Value::Text(value));
    }

    pub fn clear_meta(&mut self) {
        self.meta.iter_mut().for_each(|(_, val)| match val {
            Value::Text(t) => *t = String::new(),
            Value::Number(n) => *n = 0.0,
            _ => (),
        });
    }
}
