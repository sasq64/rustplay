use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::Write as _;
use std::io::{self, stdout};
use std::panic;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::{path::Path, thread::JoinHandle};

use anyhow::Result;
use crossterm::cursor::MoveToNextLine;
use crossterm::style::SetBackgroundColor;
use gui::KeyReturn;
use musix::MusicError;

use crate::player::{Cmd, Info, PlayResult, Player};
use crate::templ::Template;
use crate::{Settings, VisualizerPos};
use crate::{term_extra, value::*};
use crossterm::{
    QueueableCommand, cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    style::{Color, Print, SetForegroundColor},
    terminal,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};

mod gui;
mod indexer;

use crate::term_extra::{MaybeCommand, SetReverse, TextComponent};

use indexer::{FileInfo, RemoteIndexer};

#[derive(Default)]
struct State {
    changed: bool,
    meta: HashMap<String, Value>,
    song: i32,
    songs: i32,
    length: i32,
    done: bool,
    show_error: i32,
    select_mode: bool,
    selected: usize,
    start_pos: usize,
    quit: bool,
    use_color: bool,
    errors: VecDeque<String>,
}

impl State {
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
                            Value::Text(format!("{:02}:{:02}", i / 60, i % 60).to_owned()),
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
                self.errors.push_back((*e).to_string());
            }
            Value::Data(_) | Value::Unknown() => {}
        }

        self.meta.insert(meta.to_owned(), val);
    }

    fn update_title(&mut self) {
        if self.changed {
            let game = self.get_meta("game");
            let title = self.get_meta("title");
            let composer = self.get_meta_or("composer", "??");
            let full_title = if game.is_empty() {
                title.to_string()
            } else if title.is_empty() {
                game.to_string()
            } else {
                format!("{title} ({game})")
            };
            self.set_meta("title_and_composer", format!("{full_title} / {composer}"));
            self.set_meta("full_title", full_title);
        }
    }

    fn get_meta(&self, name: &str) -> &str {
        if let Some(Value::Text(t)) = self.meta.get(name) {
            return t;
        }
        ""
    }

    fn get_meta_or<'a>(&'a self, name: &str, def: &'a str) -> &'a str {
        if let Some(Value::Text(t)) = self.meta.get(name) {
            return t;
        }
        def
    }

    fn set_meta(&mut self, what: &str, value: String) {
        self.meta.insert(what.into(), Value::Text(value));
    }

    fn clear_meta(&mut self) {
        self.meta.iter_mut().for_each(|(_, val)| match val {
            Value::Text(t) => *t = String::new(),
            Value::Number(n) => *n = 0.0,
            _ => (),
        });
    }
}

/*
struct Shell {
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
*/

pub(crate) struct RustPlay {
    cmd_producer: mpsc::Sender<Cmd>,
    info_consumer: mpsc::Receiver<(String, Value)>,
    templ: Template,
    msec: Arc<AtomicUsize>,
    data: Vec<f32>,
    player_thread: Option<JoinHandle<()>>,
    fft_pos: VisualizerPos,
    fft_height: usize,
    errors: VecDeque<String>,
    state: State,
    height: usize,
    log_file: File,
    no_term: bool,
    //shell: Shell,
    indexer: RemoteIndexer,
    menu_component: gui::SongMenu,
    search_component: gui::SearchField,
}
impl RustPlay {
    pub fn new(settings: Settings) -> Result<RustPlay> {
        // Send commands to player
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));

        if !settings.args.no_term {
            Self::setup_term()?;
        }

        let (w, h) = terminal::size()?;

        let mut templ = Template::new(&settings.template, w as usize, 10)?;
        let color = !settings.args.no_color;
        templ.set_use_color(color);

        let prompt_color = if color { Color::Yellow } else { Color::Reset };
        let cursor_color = if color { Color::Yellow } else { Color::Reset };
        let th = templ.height();

        Ok(RustPlay {
            cmd_producer,
            info_consumer,
            templ,
            msec: msec.clone(),
            data: Vec::new(),
            player_thread: Some(crate::player::run_player(
                &settings.args,
                info_producer,
                cmd_consumer,
                msec,
            )?),
            fft_pos: settings.args.visualizer,
            fft_height: settings.args.visualizer_height,
            errors: VecDeque::new(),
            state: State {
                changed: true,
                use_color: !settings.args.no_color,
                ..State::default()
            },
            height: h.into(),
            log_file: File::create(".rustplay.log")?,
            no_term: settings.args.no_term,
            //shell: Shell::new(),
            indexer: RemoteIndexer::new()?,
            menu_component: gui::SongMenu {
                height: h.into(),
                ..gui::SongMenu::default()
            },
            search_component: gui::SearchField::new(th),
        })
    }

    fn log(&mut self, text: &str) -> io::Result<()> {
        writeln!(self.log_file, "{}", text)?;
        self.log_file.flush()
    }

    fn setup_term() -> io::Result<()> {
        enable_raw_mode()?;
        stdout()
            .queue(EnterAlternateScreen)?
            .queue(cursor::Hide)?
            .flush()
    }

    pub fn restore_term() -> io::Result<()> {
        stdout()
            .queue(LeaveAlternateScreen)?
            .queue(cursor::Show)?
            .flush()?;
        disable_raw_mode()
    }

    fn bg_color(&self, color: Color) -> MaybeCommand<SetBackgroundColor> {
        if self.state.use_color {
            MaybeCommand::Set(SetBackgroundColor(color))
        } else {
            MaybeCommand::None
        }
    }

    fn fg_color(&self, color: Color) -> MaybeCommand<SetForegroundColor> {
        if self.state.use_color {
            MaybeCommand::Set(SetForegroundColor(color))
        } else {
            MaybeCommand::None
        }
    }

    pub fn draw_screen(&mut self) -> Result<()> {
        if self.no_term {
            return Ok(());
        }

        let black_bg = self.bg_color(Color::Rgb { r: 0, g: 0, b: 0 });
        let normal_bg = SetReverse(false);
        let cursor_bg = SetReverse(true);

        stdout().queue(normal_bg)?.queue(black_bg)?.flush()?;
        if self.state.changed {
            self.state.changed = false;
            stdout()
                .queue(Clear(ClearType::All))?
                .queue(self.fg_color(Color::Cyan))?;
            self.templ.write(&self.state.meta, 0, 0)?;
        }

        let mut out = stdout();

        self.search_component.draw(&mut self.indexer)?;

        if self.state.select_mode {
            self.menu_component.draw(&mut self.indexer)?;
            return Ok(());
        }

        if let Some((x, y)) = self.templ.get_pos("count") {
            out.queue(cursor::MoveTo(x, y))?
                .queue(Print(format!("{}", self.indexer.index_count())))?
                .flush()?;
        }

        if self.fft_pos != VisualizerPos::None {
            let (x, y) = if self.fft_pos == VisualizerPos::Right {
                (73, 0)
            } else {
                (1, 9)
            };
            let use_color = true;
            if let Some(Value::Data(data)) = self.state.meta.get("fft") {
                if self.data.len() != data.len() {
                    self.data.resize(data.len(), 0.0);
                }
                data.iter().zip(self.data.iter_mut()).for_each(|(a, b)| {
                    let d = *a as f32;
                    *b = if *b < d { d } else { *b * 0.75 + d * 0.25 }
                });
                let w = data.len() * 3;
                let h = self.fft_height;
                let mut area: Vec<char> = vec![' '; w * h];
                print_bars(&self.data, &mut area, w, h);
                out.queue(self.fg_color(Color::DarkBlue))?;
                for i in 0..h {
                    out.queue(cursor::MoveTo(x, y + i as u16))?;
                    if use_color {
                        let col: u8 = ((i * 255) / h) as u8;
                        out.queue(self.fg_color(Color::Rgb {
                            r: 250 - col,
                            g: col,
                            b: 0x40,
                        }))?;
                    }
                    let offset = i * w;
                    let line: String = area[offset..(offset + w)].iter().collect();
                    out.queue(Print(line))?;
                }
            }
        }

        if self.state.show_error > 0 {
            self.state.show_error -= 1;
            let empty = "".to_string();
            let err = self.state.errors.front().unwrap_or(&empty);
            out.queue(cursor::MoveTo(2, 1))?
                .queue(self.fg_color(Color::Red))?
                .queue(Print(err))?;
            if self.state.show_error == 0 {
                self.state.errors.pop_front();
                self.state.changed = true;
            }
        } else if !self.state.errors.is_empty() {
            self.state.show_error = 50;
        }

        let play_time = self.msec.load(Ordering::SeqCst);
        let c = (play_time / 10) % 100;
        let m = play_time / 60000;
        let s = (play_time / 1000) % 60;
        self.templ
            .write_field(0, 0, "time", &format!("{m:02}:{s:02}:{c:02}"))?;
        out.flush()?;
        Ok(())
    }

    fn send_cmd(&mut self, f: impl FnOnce(&mut Player) -> PlayResult + Send + 'static) {
        if self.cmd_producer.send(Box::new(f)).is_err() {
            panic!("");
        }
    }

    fn search(&mut self, query: &str) -> Result<()> {
        self.indexer.search(query)?;
        //self.shell.clear();
        self.state.select_mode = true;
        self.state.selected = 0;
        self.state.start_pos = 0;
        Ok(())
    }

    fn set_song(&mut self, song: u32) {
        self.send_cmd(move |p| p.set_song(song as i32));
    }
    /*
        fn handle_player_key(&mut self, key: event::KeyEvent) -> Result<()> {
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Char(d) if d.is_ascii_digit() && ctrl => {
                    self.set_song(d.to_digit(10).unwrap())
                }
                KeyCode::Up | KeyCode::Down => {
                    self.state.select_mode = true;
                    self.handle_select_key(key)?;
                }
                KeyCode::Char('c') if ctrl => self.state.quit = true,
                KeyCode::Char('n') if ctrl => self.next(),
                KeyCode::Right => self.send_cmd(Player::next_song),
                KeyCode::Left => self.send_cmd(Player::prev_song),
                KeyCode::Backspace => self.shell.del(),
                KeyCode::Char(x) => self.shell.insert(x),
                KeyCode::Esc => self.shell.clear(),
                KeyCode::Enter => self.search(&self.shell.command())?,
                _ => {}
            };
            Ok(())
        }

        fn handle_select_key(&mut self, key: event::KeyEvent) -> Result<()> {
            let song_len = self.indexer.song_len();
            let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
            match key.code {
                KeyCode::Esc => {
                    self.state.select_mode = false;
                    self.state.changed = true;
                }

                KeyCode::Char('c') if ctrl => self.state.quit = true,
                KeyCode::Char('n') if ctrl => self.next(),
                KeyCode::Char(_) => {
                    self.state.select_mode = false;
                    self.state.changed = true;
                    self.handle_player_key(key)?;
                }
                KeyCode::Up => self.state.selected -= if self.state.selected > 0 { 1 } else { 0 },
                KeyCode::PageUp => {
                    self.state.selected = if self.state.selected >= self.height {
                        self.state.selected - self.height
                    } else {
                        0
                    }
                }
                KeyCode::PageDown => self.state.selected += self.height,
                KeyCode::Down => self.state.selected += 1,
                KeyCode::Left => self.send_cmd(Player::prev_song),
                KeyCode::Enter => {
                    if let Some(song) = self.indexer.get_song(self.state.selected) {
                        self.play_song(&song);
                    }
                    self.state.changed = true;
                    self.state.select_mode = false;
                }
                _ => {}
            }

            if self.state.selected < self.state.start_pos {
                self.state.start_pos = if self.state.start_pos >= self.height {
                    self.state.start_pos - self.height
                } else {
                    0
                }
            } else if self.state.selected >= self.state.start_pos + self.height {
                self.state.start_pos += self.height
            }

            if self.state.selected + 1 >= song_len {
                self.state.selected = song_len - 1;
            }
            if song_len <= self.height {
                self.state.start_pos = 0;
            } else if self.state.start_pos + self.height > song_len {
                self.state.start_pos = song_len - self.height;
            }
            Ok(())
        }
    */
    pub fn handle_keys(&mut self) -> Result<bool> {
        if self.no_term {
            return Ok(false);
        }
        let ms = std::time::Duration::from_millis(40);
        if !event::poll(ms)? {
            return Ok(false);
        }
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char(d) if d.is_ascii_digit() && ctrl => {
                        self.set_song(d.to_digit(10).unwrap())
                    }
                    // KeyCode::Up | KeyCode::Down => {
                    //     self.state.select_mode = true;
                    //     self.handle_select_key(key)?;
                    // }
                    KeyCode::Char('c') if ctrl => self.state.quit = true,
                    KeyCode::Char('n') if ctrl => self.next(),
                    KeyCode::Right => self.send_cmd(Player::next_song),
                    KeyCode::Left => self.send_cmd(Player::prev_song),
                    _ => {}
                }
                if self.state.select_mode {
                    match self.menu_component.handle_key(&mut self.indexer, key)? {
                        KeyReturn::PlaySong(song) => {
                            self.play_song(&song);
                            self.state.changed = true;
                            self.state.select_mode = false;
                        }
                        KeyReturn::ExitMenu => {
                            self.state.changed = true;
                            self.state.select_mode = false;
                        }
                        _ => {}
                    }

                    //self.handle_select_key(key)?;
                } else {
                    match self.search_component.handle_key(&mut self.indexer, key)? {
                        KeyReturn::Search(query) => {
                            self.search(&query)?;
                            self.state.select_mode = true;
                        }
                        KeyReturn::ExitMenu => {
                            self.state.changed = true;
                            self.state.select_mode = false;
                        }
                        _ => {}
                    }
                    //self.handle_player_key(key)?;
                }
            }
        }
        Ok(self.state.quit)
    }

    pub fn play_song(&mut self, song: &FileInfo) {
        self.state.clear_meta();
        for (name, val) in song.meta_data.iter() {
            self.log(&format!("INDEX-META {name} = {val}")).unwrap();
            self.state.update_meta(name, val.clone());
        }
        self.state.update_title();
        let path = song.path().to_owned();
        self.send_cmd(move |p| p.load(&path));
    }

    pub fn next(&mut self) {
        if let Some(song) = self.indexer.next() {
            self.play_song(&song);
        }
    }

    pub fn update_meta(&mut self) {
        if self.state.done {
            self.next();
            self.state.done = false;
        }
        while let Ok((meta, val)) = self.info_consumer.try_recv() {
            if meta != "fft" {
                self.log(&format!("SONG-META {} = {}", meta, val)).unwrap();
            }
            self.state.update_meta(&meta, val);
        }
        self.state.update_title();
    }

    pub fn add_path(&mut self, song: &Path) -> Result<()> {
        self.indexer.add_path(song)?;
        Ok(())
    }

    pub fn quit(&mut self) -> Result<()> {
        if !self.no_term {
            RustPlay::restore_term()?;
        }
        if self.cmd_producer.send(Box::new(move |p| p.quit())).is_err() {
            return Err(MusicError {
                msg: "Quit failed".into(),
            }
            .into());
        }

        if let Some(t) = self.player_thread.take() {
            if let Err(err) = t.join() {
                panic::resume_unwind(err);
            }
        }
        Ok(())
    }
}

fn print_bars(bars: &[f32], target: &mut [char], w: usize, h: usize) {
    const C: [char; 9] = ['█', '▇', '▆', '▅', '▄', '▃', '▂', '▁', ' '];
    for x in 0..bars.len() {
        let n = (bars[x] * (h as f32 / 5.0)) as i32;
        for y in 0..h {
            let bar_char = C[(((h - y) * 8) as i32 - n).clamp(0, 8) as usize];
            target[x * 3 + y * w] = bar_char;
            target[x * 3 + 1 + y * w] = bar_char;
            target[x * 3 + 2 + y * w] = ' ';
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RustPlay;
    use crate::Args;
    use crate::Settings;

    #[test]
    fn rustplay_starts() {
        let settings = Settings {
            args: Args {
                no_term: true,
                ..Args::default()
            },
            template: "".into(),
            width: 10,
        };
        let mut rp = RustPlay::new(settings).unwrap();
        rp.quit().unwrap();
    }
}
