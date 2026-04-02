use super::song::{FileInfo, SongArray, SongCollection};
use crate::{log, term_extra::SetReverse};
use anyhow::Result;
use crossterm::{
    QueueableCommand,
    cursor::{self, MoveToNextLine},
    event::{self, KeyCode},
    style::{Color, Print, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::{
    collections::HashMap,
    io::{Write, stdout},
};
use std::rc::Rc;

// SONG MENU

pub enum KeyReturn {
    Quit,
    Nothing,
    PlaySong(FileInfo),
    Search(String),
    ExitMenu,
    Up,
    Navigate,
}

pub enum MenuNav {
    Up,
    Down,
    PageUp,
    PageDown,
}

pub struct SongMenu {
    pub start_pos: usize,
    pub selected: usize,
    pub width: usize,
    pub height: usize,
    pub fader: Vec<u32>,
    pub use_color: bool,
    scrolled: bool,
    moved: bool,
    pub location: String,
    pub info: String,
    songs: Rc<dyn SongCollection>,
    stack: HashMap<String, FileInfo>,
}

impl Default for SongMenu {
    fn default() -> Self {
        Self {
            start_pos: 0,
            selected: 0,
            width: 0,
            height: 0,
            fader: Vec::new(),
            use_color: false,
            scrolled: false,
            moved: false,
            location: String::new(),
            info: String::new(),
            songs: Rc::new(SongArray::default()),
            stack: HashMap::new(),
        }
    }
}

impl SongMenu {
    pub fn set_info(&mut self, info: impl Into<String>) {
        self.info = info.into();
    }

    pub fn set_songs(&mut self, location: impl Into<String>, songs: Rc<dyn SongCollection>) {
        if self.selected < self.songs.len() {
            self.stack
                .insert(self.location.clone(), self.songs.get(self.selected));
        }

        self.selected = 0;
        self.start_pos = 0;
        self.location = location.into();
        self.songs = songs;
        self.scrolled = true;
        if let Some(file_info) = self.stack.get(&self.location)
            && let Some(index) = self.songs.index_of(file_info)
        {
            self.selected = index;
        }

        self.update_scrolling();
    }

    pub fn songs(&self) -> &Rc<dyn SongCollection> {
        &self.songs
    }

    fn fade(&self, i: usize) -> Color {
        let x: u8 = (155 + self.fader[i] * 10) as u8;
        Color::Rgb { r: x, g: x, b: x }
    }

    pub fn draw(&mut self) -> Result<()> {
        if self.fader.len() != self.height {
            self.fader.resize(self.height, 0);
        }
        let mut out = stdout();

        let black = Color::Rgb { r: 0, g: 0, b: 0 };
        let white = Color::Rgb {
            r: 255,
            g: 255,
            b: 255,
        };

        if self.scrolled {
            out.queue(Clear(ClearType::All))?;
        }
        if self.use_color {
            out.queue(SetForegroundColor(white))?
                .queue(SetBackgroundColor(Color::DarkRed))?
                .queue(cursor::MoveTo(0, 0))?
                .queue(Print(format!(
                    "{}{:>width$}",
                    &self.location,
                    &self.info,
                    width = self.width - self.location.len()
                )))?
                .queue(cursor::MoveTo(0, 1))?
                .queue(SetBackgroundColor(black))?;
        } else {
            out.queue(cursor::MoveTo(0, 0))?
                .queue(Print(format!(
                    "{}{:>width$}",
                    &self.location,
                    &self.info,
                    width = self.width - self.location.len()
                )))?
                .queue(cursor::MoveTo(0, 1))?;
        }
        let start = self.start_pos;
        let end = (start + self.height).clamp(start, self.songs.len());
        let songs = &self.songs.get_range(start, end);
        if self.use_color {
            self.fader[self.selected - self.start_pos] = 10;
            for (i, song) in songs.iter().enumerate() {
                let name = song.full_song_name();
                let name = if name.len() > (self.width - 1) {
                    name.chars().take(self.width - 1).collect()
                } else {
                    name
                };
                out.queue(SetForegroundColor(self.fade(i)))?
                    .queue(Print(name))?
                    .queue(MoveToNextLine(1))?;
                self.fader[i] = self.fader[i].saturating_sub(1);
            }
        } else {
            let normal_bg = SetReverse(false);
            let cursor_bg = SetReverse(true);
            for (i, song) in songs.iter().enumerate() {
                out.queue(if i == self.selected - start {
                    &cursor_bg
                } else {
                    &normal_bg
                })?
                .queue(Print(song.full_song_name()))?
                .queue(MoveToNextLine(1))?;
            }
        }
        out.flush()?;
        self.scrolled = false;
        self.moved = false;
        Ok(())
    }

    pub fn new(use_color: bool, width: usize, height: usize) -> Self {
        SongMenu {
            width,
            height: height - 1,
            use_color,
            scrolled: true,
            ..Self::default()
        }
    }

    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height - 1;
        self.scrolled = true;
    }

    pub fn refresh(&mut self) {
        self.scrolled = true;
    }

    pub fn get_current(&self) -> FileInfo {
        self.songs.get(self.selected).clone()
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_nav(&mut self, nav: MenuNav) -> bool {
        let old_selected = self.selected;
        match nav {
            MenuNav::Up => {
                if self.selected > 0 {
                    self.selected -= 1;
                }
            }
            MenuNav::PageUp => {
                if self.selected >= self.height {
                    self.selected -= self.height;
                } else {
                    self.selected = 0;
                }
            }
            MenuNav::PageDown => self.selected += self.height,
            MenuNav::Down => self.selected += 1,
        }
        if self.selected == old_selected {
            return true;
        }
        self.moved = true;
        self.update_scrolling();
        true
    }

    fn update_scrolling(&mut self) {
        let song_len = self.songs.len();

        while self.selected < self.start_pos {
            self.start_pos = self.start_pos.saturating_sub(self.height);
            self.scrolled = true;
        }
        while self.selected >= self.start_pos + self.height {
            self.start_pos += self.height;
            self.scrolled = true;
        }
        if song_len > 0 {
            if self.selected + 1 >= song_len {
                self.selected = song_len - 1;
            }
            if song_len <= self.height {
                self.start_pos = 0;
                self.scrolled = true;
            } else if self.start_pos + self.height > song_len {
                self.start_pos = song_len - self.height;
                self.scrolled = true;
            }
        }
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

    fn len(&self) -> usize {
        self.cmd.len()
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
    pub xpos: u16,
    pub ypos: u16,
    pub prompt_color: Color,
    pub cursor_color: Color,
    pub text_color: Color,
    pub use_color: bool,
}

impl SearchField {
    pub fn new(xpos: u16, ypos: u16, use_color: bool) -> SearchField {
        SearchField {
            shell: Shell::new(),
            xpos,
            ypos,
            prompt_color: Color::Yellow,
            cursor_color: Color::Red,
            text_color: Color::Green,
            use_color,
        }
    }
}

impl SearchField {
    pub fn draw(&self) -> Result<()> {
        let mut out = stdout();

        let (first, cursor, last) = self.shell.command_line();
        if self.use_color {
            out.queue(cursor::MoveTo(self.xpos, self.ypos))?
                .queue(Clear(ClearType::UntilNewLine))?
                .queue(SetForegroundColor(self.prompt_color))?
                .queue(Print("> "))?
                .queue(SetForegroundColor(self.text_color))?
                .queue(Print(first))?
                .queue(SetBackgroundColor(self.cursor_color))?
                .queue(Print(cursor))?
                .queue(SetBackgroundColor(Color::Reset))?
                .queue(Print(last))?;
        } else {
            out.queue(cursor::MoveTo(self.xpos, self.ypos))?
                .queue(Clear(ClearType::UntilNewLine))?
                .queue(Print("> "))?
                .queue(Print(first))?
                .queue(SetReverse(true))?
                .queue(Print(cursor))?
                .queue(SetReverse(false))?
                .queue(Print(last))?;
        }

        Ok(())
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: event::KeyEvent) -> Result<KeyReturn> {
        let mut search = false;
        match key.code {
            KeyCode::Backspace => self.shell.del(),
            KeyCode::Char(x) => self.shell.insert(x),
            KeyCode::Esc => {
                if self.shell.len() == 0 {
                    return Ok(KeyReturn::ExitMenu);
                }
                self.shell.clear();
            }
            KeyCode::Enter => search = true,
            KeyCode::PageUp | KeyCode::PageDown | KeyCode::Up | KeyCode::Down => {
                return Ok(KeyReturn::Navigate);
            }
            _ => {}
        }
        if search {
            let rc = KeyReturn::Search(self.shell.command());
            self.shell.clear();
            Ok(rc)
        } else {
            Ok(KeyReturn::Nothing)
        }
    }
}

#[derive(Default)]
pub struct Fft {
    pub data: Vec<f32>,
    pub height: i32,
    pub use_color: bool,
    pub x: u16,
    pub y: u16,
}

impl Fft {
    fn print_bars(bars: &[f32], target: &mut [char], w: usize, h: usize) {
        const BARS: [char; 9] = ['█', '▇', '▆', '▅', '▄', '▃', '▂', '▁', ' '];
        for x in 0..bars.len() {
            let n = (bars[x] * (h as f32 / 5.0)) as i32;
            for y in 0..h {
                let bar_char = BARS[(((h - y) * 8) as i32 - n).clamp(0, 8) as usize];
                target[x * 3 + y * w] = bar_char;
                target[x * 3 + 1 + y * w] = bar_char;
                target[x * 3 + 2 + y * w] = ' ';
            }
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        if self.data.len() != data.len() {
            self.data.resize(data.len(), 0.0);
            log!("FFT SIZE {}", data.len());
        }
        data.iter().zip(self.data.iter_mut()).for_each(|(a, b)| {
            let d = f32::from(*a);
            *b = if *b < d { d } else { *b * 0.75 + d * 0.25 }
        });
    }

    pub fn draw(&self) -> Result<()> {
        let w = self.data.len() * 3;
        let h = self.height as usize;
        let mut area: Vec<char> = vec![' '; w * h];
        Fft::print_bars(&self.data, &mut area, w, h);
        let mut out = stdout();
        for i in 0..h {
            out.queue(cursor::MoveTo(self.x, self.y + i as u16))?;
            if self.use_color {
                let col: u8 = ((i * 255) / h) as u8;
                out.queue(SetForegroundColor(Color::Rgb {
                    r: 250 - col,
                    g: col,
                    b: 0x40,
                }))?;
            }
            let offset = i * w;
            let line: String = area[offset..(offset + w)].iter().collect();
            out.queue(Print(line))?;
        }
        Ok(())
    }
}
