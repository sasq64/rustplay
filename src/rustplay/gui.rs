use super::song::{FileInfo, SongArray, SongCollection};
use crate::log;
use crossterm::event::{self, KeyCode};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
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
        Color::Rgb(x, x, x)
    }

    pub fn render(&mut self, buf: &mut Buffer, _area: Rect) {
        if self.fader.len() != self.height {
            self.fader.resize(self.height, 0);
        }

        let black = Color::Rgb(0, 0, 0);
        let white = Color::Rgb(255, 255, 255);

        let header_text = format!(
            "{}{:>width$}",
            &self.location,
            &self.info,
            width = self.width.saturating_sub(self.location.len())
        );
        let header_style = if self.use_color {
            Style::default().fg(white).bg(Color::Red)
        } else {
            Style::default()
        };
        buf.set_stringn(0, 0, &header_text, self.width, header_style);

        let start = self.start_pos;
        let end = (start + self.height).clamp(start, self.songs.len());
        let songs = &self.songs.get_range(start, end);

        if self.use_color {
            // Mark selected row in the fader so it stays brightest
            self.fader[self.selected - self.start_pos] = 10;
            for (i, song) in songs.iter().enumerate() {
                let name = song.full_song_name();
                let style = Style::default().fg(self.fade(i)).bg(black);
                buf.set_stringn(0, (i + 1) as u16, &name, self.width, style);
                self.fader[i] = self.fader[i].saturating_sub(1);
            }
        } else {
            for (i, song) in songs.iter().enumerate() {
                let style = if i == self.selected - start {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                buf.set_stringn(0, (i + 1) as u16, song.full_song_name(), self.width, style);
            }
        }
        // Pad any rows below the song list so previous content does not bleed through.
        let drawn = songs.len();
        let blank: String = " ".repeat(self.width);
        for i in drawn..self.height {
            buf.set_stringn(0, (i + 1) as u16, &blank, self.width, Style::default());
        }
        self.scrolled = false;
        self.moved = false;
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
    pub fn render(&self, buf: &mut Buffer, area: Rect) {
        let (first, cursor, last) = self.shell.command_line();
        let cursor_str = cursor.to_string();

        // Clear the line up to the right edge so previous content doesn't bleed through.
        let line_width = (area.width.saturating_sub(self.xpos)) as usize;
        let blank: String = " ".repeat(line_width);
        buf.set_stringn(self.xpos, self.ypos, &blank, line_width, Style::default());

        let mut x = self.xpos;
        let prompt = "> ";
        let prompt_style = if self.use_color {
            Style::default().fg(self.prompt_color)
        } else {
            Style::default()
        };
        buf.set_string(x, self.ypos, prompt, prompt_style);
        x += prompt.chars().count() as u16;

        let text_style = if self.use_color {
            Style::default().fg(self.text_color)
        } else {
            Style::default()
        };
        buf.set_string(x, self.ypos, &first, text_style);
        x += first.chars().count() as u16;

        let cursor_style = if self.use_color {
            Style::default().fg(self.text_color).bg(self.cursor_color)
        } else {
            Style::default().add_modifier(Modifier::REVERSED)
        };
        buf.set_string(x, self.ypos, &cursor_str, cursor_style);
        x += 1;

        buf.set_string(x, self.ypos, &last, text_style);
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn handle_key(&mut self, key: event::KeyEvent) -> anyhow::Result<KeyReturn> {
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

// Create a target_count colors from source, by evenly distributing the source
// colors in the new array an then interpolating the values in between.
pub fn interpolate_colors(source: &[u32], target_count: usize) -> Vec<u32> {
    if target_count == 0 || source.is_empty() {
        return vec![];
    }
    if source.len() == 1 || target_count == 1 {
        return vec![source[0]; target_count];
    }
    let mut result = vec![0u32; target_count];
    for (i, res) in result.iter_mut().enumerate() {
        let pos = i as f64 * (source.len() - 1) as f64 / (target_count - 1) as f64;
        let lo = pos as usize;
        let hi = (lo + 1).min(source.len() - 1);
        let t = pos - lo as f64;
        let lerp = |shift: u32| -> u32 {
            let a = (source[lo] >> shift) & 0xFF;
            let b = (source[hi] >> shift) & 0xFF;
            ((a as f64 * (1.0 - t) + b as f64 * t) + 0.5) as u32
        };
        *res = (lerp(16) << 16) | (lerp(8) << 8) | lerp(0);
    }
    result
}

#[derive(Default)]
pub struct Fft {
    pub data: Vec<f32>,
    pub height: i32,
    pub use_color: bool,
    pub x: u16,
    pub y: u16,
    pub bar_width: usize,
    pub gap: usize,
    pub colors: Vec<u32>,
}

impl Fft {
    fn print_bars(&self, target: &mut [char]) {
        let gb = self.bar_width + self.gap;
        let w = self.data.len() * gb;
        let h = self.height as usize;
        const BARS: [char; 9] = ['█', '▇', '▆', '▅', '▄', '▃', '▂', '▁', ' '];
        for x in 0..self.data.len() {
            let n = (self.data[x] * (h as f32 / 25.0)) as i32;
            for y in 0..h {
                let bar_char = BARS[(((h - y) * 8) as i32 - n).clamp(0, 8) as usize];
                let xx = x * gb;
                for j in xx..(xx + self.bar_width) {
                    target[j + y * w] = bar_char;
                }
                for j in (xx + self.bar_width)..(xx + self.bar_width + self.gap) {
                    target[j + y * w] = ' ';
                }
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

    pub fn render(&self, buf: &mut Buffer, area: Rect) {
        if self.data.is_empty() {
            return;
        }
        let gb = self.bar_width + self.gap;
        let w = self.data.len() * gb;
        let h = self.height as usize;
        let mut area_chars: Vec<char> = vec![' '; w * h];
        self.print_bars(&mut area_chars);
        for i in 0..h {
            let style = if self.use_color {
                let r = (self.colors[i] >> 16) as u8;
                let g = ((self.colors[i] >> 8) & 0xff) as u8;
                let b = (self.colors[i] & 0xff) as u8;
                Style::default().fg(Color::Rgb(r, g, b))
            } else {
                Style::default()
            };
            let offset = i * w;
            let line: String = area_chars[offset..(offset + w)].iter().collect();
            buf.set_string(area.x, area.y + i as u16, &line, style);
        }
    }
}
