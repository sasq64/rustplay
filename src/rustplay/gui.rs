use super::indexer::{FileInfo, RemoteIndexer};
use crate::term_extra::{self, SetReverse, TextComponent};
use anyhow::Result;
use crossterm::{
    Command, QueueableCommand,
    cursor::{self, MoveToNextLine},
    event::{self, KeyCode, KeyModifiers},
    style::{Color, Print, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{Write, stdout};

// SONG MENU

pub enum KeyReturn {
    Quit,
    Nothing,
    PlaySong(FileInfo),
    Search(String),
    ExitMenu,
}

#[derive(Default)]
pub struct SongMenu {
    pub start_pos: usize,
    pub selected: usize,
    pub height: usize,
}

impl term_extra::TextComponent for SongMenu {
    type UiState = RemoteIndexer;
    type Return = KeyReturn;

    fn draw(&self, indexer: &mut RemoteIndexer) -> Result<()> {
        let mut out = stdout();
        let normal_bg = SetReverse(false);
        let cursor_bg = SetReverse(true);
        out.queue(Clear(ClearType::All))?;
        out.queue(cursor::MoveTo(0, 0))?;
        let start = self.start_pos;
        let songs = indexer.get_songs(start, start + self.height)?;
        for (i, song) in songs.into_iter().enumerate() {
            out.queue(if i == self.selected - start {
                &cursor_bg
            } else {
                &normal_bg
            })?
            .queue(Print(song.full_song_name()))?
            .queue(MoveToNextLine(1))?;
        }
        out.flush()?;
        Ok(())
    }

    fn handle_key(
        &mut self,
        indexer: &mut RemoteIndexer,
        key: event::KeyEvent,
    ) -> Result<KeyReturn> {
        let song_len = indexer.song_len();
        let mut exit = false;
        let mut song: Option<FileInfo> = None;
        match key.code {
            KeyCode::Esc => exit = true,
            KeyCode::Char(_) => exit = true,
            KeyCode::Up => self.selected -= if self.selected > 0 { 1 } else { 0 },
            KeyCode::PageUp => {
                self.selected = if self.selected >= self.height {
                    self.selected - self.height
                } else {
                    0
                }
            }
            KeyCode::PageDown => self.selected += self.height,
            KeyCode::Down => self.selected += 1,
            KeyCode::Enter => {
                if let Some(s) = indexer.get_song(self.selected) {
                    song = Some(s);
                }
            }
            _ => {}
        }

        if exit {
            return Ok(KeyReturn::ExitMenu);
        } else if let Some(song) = song {
            return Ok(KeyReturn::PlaySong(song));
        }

        if self.selected < self.start_pos {
            self.start_pos = if self.start_pos >= self.height {
                self.start_pos - self.height
            } else {
                0
            }
        } else if self.selected >= self.start_pos + self.height {
            self.start_pos += self.height
        }

        if self.selected + 1 >= song_len {
            self.selected = song_len - 1;
        }
        if song_len <= self.height {
            self.start_pos = 0;
        } else if self.start_pos + self.height > song_len {
            self.start_pos = song_len - self.height;
        }
        Ok(KeyReturn::Nothing)
    }
}

pub struct Shell {
    cmd: Vec<char>,
    edit_pos: usize,
}

impl Shell {
    fn new() -> Self {
        Self {
            cmd: Vec::new(),
            edit_pos: 0,
        }
    }

    fn command(&self) -> String {
        self.cmd.iter().collect()
    }

    fn command_line(&self) -> (String, char, String) {
        let at_end = self.edit_pos == self.cmd.len();
        (
            self.cmd[..self.edit_pos].iter().collect(),
            if at_end { ' ' } else { self.cmd[self.edit_pos] },
            if at_end {
                String::new()
            } else {
                self.cmd[self.edit_pos + 1..].iter().collect()
            },
        )
    }

    fn insert(&mut self, c: char) {
        self.cmd.insert(self.edit_pos, c);
        self.edit_pos += 1;
    }

    fn del(&mut self) {
        if self.edit_pos == 0 {
            return;
        }
        self.edit_pos -= 1;
        self.cmd.remove(self.edit_pos);
    }

    fn go(&mut self, delta: isize) {
        let mut p = self.edit_pos as isize;
        p += delta;
        if p >= 0 && p < self.cmd.len() as isize {
            self.edit_pos = p as usize;
        }
    }

    fn clear(&mut self) {
        self.cmd.clear();
        self.edit_pos = 0;
    }
}

// Search field

pub struct SearchField {
    pub shell: Shell,
    pub ypos: usize,
    pub prompt_color: Color,
    pub cursor_color: Color,
    pub text_color: Color,
}

impl SearchField {
    pub fn new(ypos: usize) -> SearchField {
        SearchField {
            shell: Shell::new(),
            ypos,
            prompt_color: Color::Yellow,
            cursor_color: Color::Red,
            text_color: Color::White,
        }
    }
}

impl TextComponent for SearchField {
    type UiState = RemoteIndexer;
    type Return = KeyReturn;

    fn draw(&self, _: &mut RemoteIndexer) -> Result<()> {
        let mut out = stdout();

        let (first, cursor, last) = self.shell.command_line();

        out.queue(cursor::MoveTo(0, self.ypos as u16 + 1))?
            .queue(Clear(ClearType::UntilNewLine))?
            .queue(SetForegroundColor(self.prompt_color))?
            .queue(Print("> "))?
            .queue(SetForegroundColor(self.text_color))?
            .queue(Print(first))?
            .queue(SetBackgroundColor(self.cursor_color))?
            .queue(Print(cursor))?
            .queue(SetBackgroundColor(Color::Reset))?
            .queue(Print(last))?;

        Ok(())
    }

    fn handle_key(&mut self, _: &mut RemoteIndexer, key: event::KeyEvent) -> Result<KeyReturn> {
        let mut search = false;
        match key.code {
            KeyCode::Backspace => self.shell.del(),
            KeyCode::Char(x) => self.shell.insert(x),
            KeyCode::Esc => self.shell.clear(),
            KeyCode::Enter => search = true,
            _ => {}
        };
        if search {
            self.shell.clear();
            Ok(KeyReturn::Search(self.shell.command()))
        } else {
            Ok(KeyReturn::Nothing)
        }
    }
}
