#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::io::Write as _;
use std::io::{self, stdout};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{cell::RefCell, rc::Rc};
use std::{error::Error, path::Path, thread::JoinHandle};

use crossterm::cursor::MoveToNextLine;
use crossterm::style::SetBackgroundColor;
use ringbuf::{HeapRb, StaticRb, traits::*};

use crossterm::{
    ExecutableCommand, QueueableCommand, cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    style::{Color, Print, SetForegroundColor},
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;

use crate::player::{Cmd, Info, PlayResult, Player, Value};
use crate::templ::Template;
use crate::{Settings, VisualizerPos};

use tantivy::{Index, IndexWriter, ReloadPolicy, TantivyError, doc};
use tantivy::{IndexReader, schema::*};

struct Indexer {
    schema: Schema,
    index: Index,
    index_writer: IndexWriter,
    reader: IndexReader,
    result: Vec<String>,
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

#[derive(Default)]
struct State {
    changed: bool,
    meta: HashMap<String, Value>,
    song: i32,
    songs: i32,
    length: i32,
    done: bool,
    show_error: i32,
    errors: VecDeque<String>,
}

impl State {
    pub fn update_meta<IC>(&mut self, info_consumer: &mut IC)
    where
        IC: Consumer<Item = Info>,
    {
        while let Some((meta, val)) = info_consumer.try_pop() {
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
        let mut composer = self.get_meta("composer");
        let full_title: String;
        if game.is_empty() {
            full_title = title.to_string();
        } else if title.is_empty() {
            full_title = game.to_string();
        } else {
            full_title = format!("{} ({})", title, game);
        }
        if composer.is_empty() {
            composer = "??";
        }
        self.set_meta(
            "title_and_composer",
            format!("{} / {}", full_title, composer),
        );
        self.set_meta("full_title", full_title);
    }

    fn get_meta(&self, name: &str) -> &str {
        if let Some(Value::Text(t)) = self.meta.get(name) {
            return t;
        }
        ""
    }

    fn set_meta(&mut self, what: &str, value: String) {
        self.meta.insert(what.into(), Value::Text(value));
    }

    fn clear_meta(&mut self) {
        self.meta.iter_mut().for_each(|(_, val)| match val {
            Value::Text(t) => *t = "".to_string(),
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

    fn command_first(&self) -> String {
        self.cmd[..self.edit_pos].iter().collect()
    }

    fn command_last(&self) -> String {
        self.cmd[self.edit_pos + 1..].iter().collect()
    }

    fn command_cursor(&self) -> char {
        self.cmd[self.edit_pos]
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

pub(crate) struct RustPlay<CP, IC> {
    cmd_producer: CP,
    info_consumer: IC,
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
    indexer: Indexer,
}

impl RustPlay<(), ()> {
    pub fn new(
        settings_ref: Rc<RefCell<Settings>>,
    ) -> Result<RustPlay<impl Producer<Item = Cmd>, impl Consumer<Item = Info>>, Box<dyn Error>>
    {
        // Send commands to player
        let (cmd_producer, cmd_consumer) = HeapRb::<Cmd>::new(5).split();

        let settings = settings_ref.borrow().clone();

        // Receive info from player
        let (info_producer, info_consumer) = StaticRb::<Info, 64>::default().split();
        let msec = Arc::new(AtomicUsize::new(0));

        if !settings.args.no_term {
            Self::setup_term()?;
        }

        // include_str!("../screen.templ"), 72, 10
        let templ = Template::new(&settings.template, 72, 10);

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
            state: Default::default(),
            no_term: settings.args.no_term,
            shell: Shell::new(),
            indexer: Indexer::new()?,
        })
    }

    fn setup_term() -> io::Result<()> {
        enable_raw_mode()?;
        stdout()
            .queue(EnterAlternateScreen)?
            .queue(cursor::Hide)?.flush()
    }

    fn restore_term() -> io::Result<()> {
        disable_raw_mode()?;
        stdout()
            .queue(LeaveAlternateScreen)?
            .queue(cursor::Show)?.flush()
    }
}

impl<CP, IC> RustPlay<CP, IC>
where
    CP: Producer<Item = Cmd>,
    IC: Consumer<Item = Info>,
{
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
        let y = self.templ.height() as u16;

        let i = self.shell.edit_pos;
        let l = self.shell.cmd.len();
        let black_bg = SetBackgroundColor(Color::Reset);
        let cursor_bg = SetBackgroundColor(Color::White);
        let mut out = stdout();
        out.queue(black_bg)?
            .queue(SetForegroundColor(Color::Red))?
            .queue(cursor::MoveTo(0, y + 1))?
            .queue(Print("> "))?
            .queue(Print(self.shell.command_first()))?;

        if i < l {
            out.queue(SetBackgroundColor(Color::White))?
                .queue(Print(&self.shell.command_cursor()))?
                .queue(black_bg)?
                .queue(Print(&self.shell.command_last()))?;
        } else {
            out.queue(cursor_bg)?.queue(Print(" "))?.queue(black_bg)?;
        }
        out.queue(black_bg)?.queue(Print(" "))?;
        if !self.indexer.result.is_empty() {
            out.queue(Clear(ClearType::All))?;
            out.queue(cursor::MoveTo(0, 0))?;
            for val in self.indexer.result.iter() {
                out.queue(Print(val))?.queue(MoveToNextLine(1))?;
            }
            return out.flush();
        }

        if self.fft_pos != VisualizerPos::None {
            let (x, y) = if self.fft_pos == VisualizerPos::Right {
                (73, 0)
            } else {
                (1, 9)
            };
            let use_color = false;
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
                for i in 0..h {
                    out.queue(cursor::MoveTo(x, y + i as u16))?;
                    let u = i * w;
                    let line: String = area[u..(u + w)].iter().collect();
                    if use_color {
                        let col: u8 = ((i * 255) / h) as u8;
                        out.queue(SetForegroundColor(Color::Rgb {
                            r: 250 - col,
                            g: col,
                            b: 0x40,
                        }))?;
                    }
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
                .queue(Print(format!("{:02}:{:02}:{:02}", m, s, c)))?;
        }
        out.flush()?;
        Ok(())
    }

    fn send_cmd(&mut self, f: impl FnOnce(&mut Player) -> PlayResult + Send + 'static) {
        if self.cmd_producer.try_push(Box::new(f)).is_err() {
            panic!("");
        }
    }

    fn search(&mut self) {
        self.indexer.search(&self.shell.command()).unwrap();
        self.shell.clear();
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
                match key.code {
                    KeyCode::Esc => return Ok(true),
                    KeyCode::Char('c') if ctrl => return Ok(true),
                    KeyCode::Char('n') if ctrl => self.next(),
                    KeyCode::Right => self.send_cmd(|p| p.next_song()),
                    KeyCode::Left => self.send_cmd(|p| p.prev_song()),
                    KeyCode::Backspace => self.shell.del(),
                    KeyCode::Char(x) => self.shell.insert(x),
                    KeyCode::Enter => self.search(),
                    _ => {}
                }
            }
        }
        Ok(false)
    }

    pub fn next(&mut self) {
        self.state.clear_meta();
        if let Some(s) = self.song_queue.pop_front() {
            self.send_cmd(move |p| p.load(&s));
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

    fn add_song_(&mut self, song: &Path) -> Result<(), io::Error> {
        if song.is_dir() {
            for p in fs::read_dir(song)? {
                self.add_song_(&p?.path())?;
            }
        } else {
            if let Some(info) = musix::identify_song(song) {
                let title = String::from_utf8_lossy(info.title.to_bytes()).to_string();
                //println!("INFO: {}", title);
                self.indexer.add(&title);
            }

            self.song_queue.push_back(song.to_owned());
        }
        Ok(())
    }
    pub fn add_song(&mut self, song: &Path) -> Result<(), io::Error> {
        self.add_song_(song)?;
        self.indexer.commit();
        Ok(())
    }

    pub fn quit(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.no_term {
            RustPlay::restore_term().unwrap();
        }
        let _ = self.cmd_producer.try_push(Box::new(move |p| p.quit()));
        if let Err(err) = self.player_thread.take().unwrap().join() {
            eprintln!("THREAD ERROR: {:?}", err);
        }
        Ok(())
    }
}

fn print_bars(bars: &[f32], target: &mut [char], w: usize, h: usize) {
    const C: [char; 9] = ['█', '▇', '▆', '▅', '▄', '▃', '▂', '▁', ' '];
    for x in 0..bars.len() {
        let n = (bars[x] * (h as f32 / 5.0)) as i32;
        for y in 0..h {
            let d = ((h - y) * 8) as i32 - n;
            let v = C[d.clamp(0, 8) as usize];
            target[x * 3 + y * w] = v;
            target[x * 3 + 1 + y * w] = v;
            target[x * 3 + 2 + y * w] = ' ';
        }
    }
}
