#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::io::Write as _;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::{error::Error, path::Path};
use strfmt::{strfmt, DisplayStr, FmtError};

use ringbuf::{traits::*, HeapRb, StaticRb};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    style::{Color, Print, SetForegroundColor},
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
    ExecutableCommand, QueueableCommand,
};

use crate::player::{Cmd, Info, PlayResult, Player, Value};

impl DisplayStr for Value {
    fn display_str(&self, f: &mut strfmt::Formatter) -> strfmt::Result<()> {
        match self {
            Value::Text(s) => f.write_str(s.as_str()).unwrap(),
            Value::Number(n) => write!(f, "{:02}", n).unwrap(),
            Value::Error(e) => write!(f, "{}", e).unwrap(),
            Value::Data(_) => (),
        }
        Ok(())
    }
}

pub(crate) struct RustPlay<CP, IC> {
    cmd_producer: CP,
    info_consumer: IC,
    templ: &'static str,
    meta_changed: bool,
    msec: Arc<AtomicUsize>,
    song_meta: HashMap<String, Value>,
    song: i32,
    songs: i32,
    length: i32,
    data: Vec<f32>,
    player_thread: Option<JoinHandle<()>>,
    song_queue: VecDeque<PathBuf>,
}

impl RustPlay<(), ()> {
    pub fn new() -> RustPlay<impl Producer<Item = Cmd>, impl Consumer<Item = Info>> {
        // Send commands to player
        let cmd_buf = HeapRb::<Cmd>::new(5);
        let (cmd_producer, cmd_consumer) = cmd_buf.split();

        // Receive info from player
        let info_buf = StaticRb::<Info, 64>::default();
        let (info_producer, info_consumer) = info_buf.split();
        let msec = Arc::new(AtomicUsize::new(0));

        Self::setup_term().unwrap();

        let mut song_meta = HashMap::<String, Value>::new();

        let templ = "TITLE:    {full_title}\r\n          {sub_title}\r\nCOMPOSER: {composer}\r\nFORMAT:   {format}\r\n\nTIME: 00:00:00 ({len}) SONG: {isong}/{songs}";
        while let Err(e) = strfmt(templ, &song_meta) {
            if let FmtError::KeyError(ke) = e {
                let (_, key) = ke.as_str().rsplit_once(' ').unwrap();
                song_meta.insert(key.to_string(), Value::Text("".to_string()));
            }
        }

        RustPlay {
            cmd_producer,
            info_consumer,
            templ,
            meta_changed: false,
            msec: msec.clone(),
            song_meta,
            song: 0,
            songs: 1,
            length: 0,
            data: Vec::new(),
            player_thread: Some(crate::player::run_player(info_producer, cmd_consumer, msec)),
            song_queue: VecDeque::new(),
        }
    }

    fn setup_term() -> io::Result<()> {
        enable_raw_mode()?;
        stdout()
            .execute(EnterAlternateScreen)?
            .execute(cursor::Hide)?;
        Ok(())
    }

    fn restore_term() -> io::Result<()> {
        disable_raw_mode()?;
        stdout()
            .execute(LeaveAlternateScreen)?
            .execute(cursor::Show)?;
        Ok(())
    }
}

impl<CP, IC> RustPlay<CP, IC>
where
    CP: Producer<Item = Cmd>,
    IC: Consumer<Item = Info>,
{
    pub fn draw_screen(&mut self) -> io::Result<()> {
        if self.meta_changed {
            self.meta_changed = false;
            //let text = self.templ.render_nofail_string(&self.song_meta);
            let text = strfmt(self.templ, &self.song_meta).unwrap();
            let _ = stdout()
                .queue(Clear(ClearType::All))?
                .queue(SetForegroundColor(Color::Cyan))?
                .queue(cursor::MoveTo(0, 0))?
                .queue(Print(text))?;
        }
        let play_time = self.msec.load(Ordering::SeqCst);
        let c = (play_time / 10) % 100;
        let m = play_time / 60000;
        let s = (play_time / 1000) % 60;
        if let Some(Value::Data(data)) = self.song_meta.get("fft") {
            if self.data.len() != data.len() {
                self.data.resize(data.len(), 0.0);
            }
            data.iter().zip(self.data.iter_mut()).for_each(|(a, b)| {
                let d = *a as f32;
                *b = if *b < d { d } else { *b * 0.75 + d * 0.25 }
            });
            let w = data.len();
            let h = 5;
            let mut area: Vec<char> = vec![' '; w * h];
            print_bars(&self.data, &mut area, w, h);
            for i in 0..h {
                let _ = stdout().queue(cursor::MoveTo(40, (1 + i) as u16));
                let u = i * w;
                let line: String = area[u..(u + w)].iter().collect();
                let col: u8 = ((i * 255) / h) as u8;
                let _ = stdout()
                    .queue(SetForegroundColor(Color::Rgb {
                        r: 250 - col,
                        g: col,
                        b: 0x40,
                    }))?
                    .queue(Print(line));
            }
        }
        stdout()
            .queue(cursor::MoveTo(6, 5))?
            .queue(SetForegroundColor(Color::Yellow))?
            .queue(Print(format!("{:02}:{:02}:{:02}", m, s, c)))?
            .flush()?;

        Ok(())
    }

    fn send_cmd(&mut self, f: impl FnOnce(&mut Player) -> PlayResult + Send + 'static) {
        if self.cmd_producer.try_push(Box::new(f)).is_err() {
            panic!("");
        }
    }

    pub fn handle_keys(&mut self) -> Result<bool, io::Error> {
        let ms = std::time::Duration::from_millis(40);
        if !event::poll(ms)? {
            return Ok(false);
        }
        if let Event::Key(key) = event::read()? {
            if key.kind == event::KeyEventKind::Press {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('q') => return Ok(true),
                    KeyCode::Char('c') if ctrl => return Ok(true),
                    KeyCode::Char('n') => self.next(),
                    KeyCode::Right => self.send_cmd(|p| p.next_song()),
                    KeyCode::Left => self.send_cmd(|p| p.prev_song()),
                    _ => {}
                }
            }
        }
        Ok(false)
    }

    pub fn next(&mut self) {
        self.clear_meta();
        if let Some(s) = self.song_queue.pop_front() {
            self.send_cmd(move |p| p.load(&s));
        }
    }

    fn update_title(&self, title: &str, game: &str) -> String {
        if game.is_empty() {
            title.to_string()
        } else if title.is_empty() {
            game.to_string()
        } else {
            format!("{} ({})", title, game)
        }
    }

    fn set_meta(&mut self, what: &str, value: String) {
        self.song_meta.insert(what.to_string(), Value::Text(value));
    }

    pub fn update_meta(&mut self) {
        while let Some((meta, val)) = self.info_consumer.try_pop() {
            match val {
                Value::Number(n) => {
                    self.meta_changed = true;
                    match meta.as_str() {
                        "done" => self.next(),
                        "length" => {
                            self.song_meta.insert(
                                "len".to_owned(),
                                Value::Text(format!("{:02}:{:02}", n / 60, n % 60).to_owned()),
                            );
                        }
                        "song" => {
                            self.song = n;
                            self.song_meta
                                .insert("isong".to_owned(), Value::Number(n + 1));
                        }
                        "songs" => self.songs = n,
                        &_ => {}
                    }
                }
                Value::Text(ref t) => {
                    match meta.as_str() {
                        "title" => {
                            if let Some(Value::Text(ref game)) = self.song_meta.get("game") {
                                self.set_meta("full_title", self.update_title(t, game));
                            } else {
                                self.set_meta("full_title", self.update_title(t, ""));
                            }
                        }
                        "game" => {
                            if let Some(Value::Text(ref title)) = self.song_meta.get("title") {
                                self.set_meta("full_title", self.update_title(title, t));
                            } else {
                                self.set_meta("full_title", self.update_title("", t));
                            }
                        }
                        &_ => {}
                    }
                    self.meta_changed = true;
                }
                Value::Error(_) => {}
                Value::Data(_) => {}
            }
            self.song_meta.insert(meta, val);
        }
    }

    fn clear_meta(&mut self) {
        self.song_meta.iter_mut().for_each(|(_, val)| match val {
            Value::Text(t) => *t = "".to_string(),
            Value::Number(n) => *n = 0,
            _ => (),
        });
    }

    pub fn add_song(&mut self, song: &Path) {
        if song.is_dir() {
            for p in fs::read_dir(song).unwrap() {
                self.add_song(&p.unwrap().path());
            }
        } else {
            self.song_queue.push_back(song.to_owned());
        }
    }

    pub fn quit(&mut self) -> Result<(), Box<dyn Error>> {
        RustPlay::restore_term().unwrap();
        let _ = self.cmd_producer.try_push(Box::new(move |p| p.quit()));
        if let Err(err) = self.player_thread.take().unwrap().join() {
            eprintln!("THREAD ERROR: {:?}", err);
        }
        Ok(())
    }
}
fn print_bars(bars: &[f32], target: &mut [char], w: usize, h: usize) {
    let c = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    for x in 0..bars.len() {
        let n = bars[x];
        for y in 0..h {
            let fy = ((h - y) as f32) * 8.0;
            let d = fy - n;
            target[x + y * w] = if d < 0.0 {
                '█'
            } else if d >= 8.0 {
                ' '
            } else {
                c[7 - (d as usize)]
            };
        }
    }
}
