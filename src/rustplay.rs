#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::io::{self, stdout};
use std::panic;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::{error::Error, path::Path, thread::JoinHandle};

use crossterm::cursor::MoveToNextLine;
use crossterm::style::SetBackgroundColor;
use musix::MusicError;

use crate::player::{Cmd, Info, PlayResult, Player};
use crate::templ::Template;
use crate::value::*;
use crate::{Settings, VisualizerPos};
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

mod indexer;

use indexer::RemoteIndexer;

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
    errors: VecDeque<String>,
    result: Vec<String>,
}

impl State {
    pub fn update_meta(&mut self, info_consumer: &mut mpsc::Receiver<Info>) {
        while let Ok((meta, val)) = info_consumer.try_recv() {
            if meta != "fft" {
                //    println!("{} = {}", meta, val);
            }
            match val {
                Value::Number(n) => {
                    self.changed = true;
                    match meta.as_str() {
                        "done" => self.done = true,
                        "length" => {
                            self.meta.insert(
                                "len".to_owned(),
                                Value::Text(format!("{:02}:{:02}", n / 60, n % 60).to_owned()),
                            );
                        }
                        "song" => {
                            self.song = n;
                            self.meta.insert("isong".into(), (n + 1).into());
                        }
                        "songs" => self.songs = n,
                        &_ => {}
                    }
                }
                Value::Text(_) => {
                    self.changed = true;
                }
                Value::Error(ref e) => {
                    self.errors.push_back((*e).to_string());
                }
                Value::Data(_) => {}
            }
            self.meta.insert(meta, val);
        }
        if self.changed {
            self.update_title();
        }
    }

    fn update_title(&mut self) {
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
            Value::Number(n) => *n = 0,
            _ => (),
        });
    }
}

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

pub(crate) struct RustPlay {
    cmd_producer: mpsc::Sender<Cmd>,
    info_consumer: mpsc::Receiver<(String, Value)>,
    templ: Template,
    msec: Arc<AtomicUsize>,
    data: Vec<f32>,
    player_thread: Option<JoinHandle<()>>,
    song_queue: VecDeque<PathBuf>,
    fft_pos: VisualizerPos,
    fft_height: usize,
    errors: VecDeque<String>,
    state: State,
    no_term: bool,
    shell: Shell,
    indexer: RemoteIndexer,
}

impl RustPlay {
    pub fn new(settings: Settings) -> Result<RustPlay, Box<dyn Error>> {
        // Send commands to player
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        //let (info_producer, info_consumer) = StaticRb::<Info, 64>::default().split();
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));

        if !settings.args.no_term {
            Self::setup_term()?;
        }

        let (w, _) = terminal::size()?;

        // include_str!("../screen.templ"), 72, 10
        let templ = Template::new(&settings.template, w as usize, 10);

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
            song_queue: VecDeque::new(),
            fft_pos: settings.args.visualizer,
            fft_height: settings.args.visualizer_height,
            errors: VecDeque::new(),
            state: State::default(),
            no_term: settings.args.no_term,
            shell: Shell::new(),
            indexer: RemoteIndexer::new()?,
        })
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

    pub fn draw_screen(&mut self) -> io::Result<()> {
        if self.no_term {
            return Ok(());
        }
        if self.state.changed {
            self.state.changed = false;
            stdout()
                .queue(Clear(ClearType::All))?
                .queue(SetForegroundColor(Color::Cyan))?;
            self.templ.write(&self.state.meta, 0, 0)?;
        }

        let black_bg = SetBackgroundColor(Color::Reset);
        let cursor_bg = SetBackgroundColor(Color::White);
        let mut out = stdout();

        let (first, cursor, last) = self.shell.command_line();

        out.queue(black_bg)?
            .queue(SetForegroundColor(Color::Red))?
            .queue(cursor::MoveTo(0, self.templ.height() as u16 + 1))?
            .queue(Print("> "))?
            .queue(Print(first))?
            .queue(cursor_bg)?
            .queue(Print(cursor))?
            .queue(black_bg)?
            .queue(Print(last))?
            .queue(black_bg)?
            .queue(Print(" "))?;
        if self.state.select_mode {
            out.queue(Clear(ClearType::All))?;
            out.queue(cursor::MoveTo(0, 0))?;
            for (i, val) in self.state.result.iter().enumerate() {
                out.queue(if i == self.state.selected {
                    cursor_bg
                } else {
                    black_bg
                })?
                .queue(Print(val))?
                .queue(MoveToNextLine(1))?;
            }
            return out.flush();
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
                out.queue(SetForegroundColor(Color::DarkBlue))?;
                for i in 0..h {
                    out.queue(cursor::MoveTo(x, y + i as u16))?;
                    if use_color {
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
            }
        }

        if self.state.show_error > 0 {
            self.state.show_error -= 1;
            let empty = "".to_string();
            let err = self.state.errors.front().unwrap_or(&empty);
            out.queue(cursor::MoveTo(2, 1))?
                .queue(SetForegroundColor(Color::Red))?
                .queue(Print(err))?;
            if self.state.show_error == 0 {
                self.state.errors.pop_front();
                self.state.changed = true;
            }
        } else if !self.state.errors.is_empty() {
            self.state.show_error = 50;
        }

        if let Some((x, y)) = self.templ.get_pos("time") {
            let play_time = self.msec.load(Ordering::SeqCst);
            let c = (play_time / 10) % 100;
            let m = play_time / 60000;
            let s = (play_time / 1000) % 60;
            out.queue(cursor::MoveTo(x, y))?
                .queue(SetForegroundColor(Color::Yellow))?
                .queue(Print(format!("{m:02}:{s:02}:{c:02}")))?;
        }
        out.flush()?;
        Ok(())
    }

    fn send_cmd(&mut self, f: impl FnOnce(&mut Player) -> PlayResult + Send + 'static) {
        if self.cmd_producer.send(Box::new(f)).is_err() {
            panic!("");
        }
    }

    fn search(&mut self) {
        self.state.result = self.indexer.search(&self.shell.command()).unwrap();
        self.shell.clear();
        self.state.select_mode = true;
        self.state.selected = 0;
    }

    pub fn handle_keys(&mut self) -> Result<bool, io::Error> {
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

                if self.state.select_mode {
                    match key.code {
                        KeyCode::Esc => self.state.select_mode = false,
                        KeyCode::Char('c') if ctrl => return Ok(true),
                        KeyCode::Char('n') if ctrl => self.next(),
                        KeyCode::Up => {
                            if self.state.selected > 0 {
                                self.state.selected -= 1;
                            }
                        }
                        KeyCode::Down => {
                            if self.state.selected < self.state.result.len() - 1 {
                                self.state.selected += 1;
                            }
                        }
                        KeyCode::Left => self.send_cmd(Player::prev_song),
                        KeyCode::Enter => {
                            let path = self.state.result[self.state.selected].clone();
                            self.song_queue.push_front(path.into());
                            self.next();
                            self.state.select_mode = false
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Esc => return Ok(true),
                        KeyCode::Char('c') if ctrl => return Ok(true),
                        KeyCode::Char('n') if ctrl => self.next(),
                        KeyCode::Right => self.send_cmd(Player::next_song),
                        KeyCode::Left => self.send_cmd(Player::prev_song),
                        KeyCode::Backspace => self.shell.del(),
                        KeyCode::Char(x) => self.shell.insert(x),
                        KeyCode::Enter => self.search(),
                        _ => {}
                    }
                }
            }
        }
        Ok(false)
    }

    pub fn next(&mut self) {
        self.state.clear_meta();
        if let Some(s) = self.song_queue.pop_front() {
            self.send_cmd(move |p| p.load(&s));
        } else if let Some(s) = self.indexer.next() {
            self.send_cmd(move |p| p.load(s.path()));
            if let Some(next) = self.song_queue.front() {
                self.state
                    .meta
                    .insert("next_song".into(), Value::Text(next.display().to_string()));
            }
        }
    }

    pub fn update_meta(&mut self) {
        if self.state.done {
            self.next();
            self.state.done = false;
        }
        self.state.update_meta(&mut self.info_consumer);
    }

    pub fn add_song(&mut self, song: &Path) -> Result<(), io::Error> {
        self.indexer.add_path(song);
        Ok(())
    }

    pub fn quit(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.no_term {
            RustPlay::restore_term()?;
        }
        if self.cmd_producer.send(Box::new(move |p| p.quit())).is_err() {
            return Err(Box::new(MusicError {
                msg: "Quit failed".into(),
            }));
        }

        if let Err(err) = self.player_thread.take().unwrap().join() {
            panic::resume_unwind(err);
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
                ..Default::default()
            },
            template: "".into(),
            width: 10,
        };
        let mut rp = RustPlay::new(settings).unwrap();
        rp.quit().unwrap();
    }
}
