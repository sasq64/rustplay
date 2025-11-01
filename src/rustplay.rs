use anyhow::Result;
use crossterm::style::SetBackgroundColor;
use gui::KeyReturn;
use musix::MusicError;
use rhai::FnPtr;
use scripting::Scripting;
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::io::{self, Cursor, Write as _, stdout};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::{fs, panic};
use std::{path::Path, thread::JoinHandle};

use crate::VisualizerPos;
use crate::player::{Cmd, Info, PlayResult, Player};
use crate::templ::Template;
use crate::value::Value;
use crate::{Args, log};
use crossterm::{
    QueueableCommand, cursor,
    event::{self, Event, KeyCode, KeyModifiers},
    style::{Color, Print, SetForegroundColor},
    terminal,
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

mod gui;
mod indexer;
mod song;
mod scripting;

use crate::term_extra::{MaybeCommand, SetReverse};

use song::{FileInfo, SongCollection};

use indexer::RemoteIndexer;

#[derive(Clone, Debug, Default)]
pub struct TemplateVar {
    color: Option<u32>,
    alias: Option<String>,
    func: Option<FnPtr>,
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
enum InputMode {
    #[default]
    Main,
    SearchInput,
    ResultScreen,
}

#[derive(Default)]
struct State {
    changed: bool,
    meta: HashMap<String, Value>,
    song: i32,
    songs: i32,
    len_msec: usize,
    done: bool,
    show_error: i32,
    mode: InputMode,
    last_mode: InputMode,
    quit: bool,
    use_color: bool,
    errors: VecDeque<String>,
    player_started: bool,
    width: i32,
    height: i32,
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
            Value::Data(_) | Value::Unknown => {}
        }

        self.meta.insert(meta.to_owned(), val);
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

fn extract_zip(data_zip: &[u8], dd: &Path) -> Result<()> {
    let cursor = Cursor::new(data_zip);
    let mut archive = zip::ZipArchive::new(cursor)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => dd.join(path),
            None => continue,
        };
        if file.is_dir() {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(p)?;
                }
            }
            let mut outfile = fs::File::create(&outpath)?;
            io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

fn make_color(color: u32) -> Color {
    let r = (color >> 16) as u8;
    let g = ((color >> 8) & 0xff) as u8;
    let b = (color & 0xff) as u8;
    Color::Rgb { r, g, b }
}

pub struct RustPlay {
    cmd_producer: mpsc::Sender<Cmd>,
    info_consumer: mpsc::Receiver<(String, Value)>,
    templ: Template,
    msec: Arc<AtomicUsize>,
    player_thread: Option<JoinHandle<()>>,
    fft_pos: VisualizerPos,
    state: State,
    height: usize,
    no_term: bool,
    indexer: RemoteIndexer,
    menu_component: gui::SongMenu,
    search_component: gui::SearchField,
    fft_component: gui::Fft,
    current_list: Option<Box<dyn SongCollection>>,
    current_song: usize,
    scripting: Scripting
}
impl RustPlay {
    /// Create a new instance of `RustPlay` using parsed command line arguments in `args`.
    ///
    /// # Errors
    ///
    pub fn new(args: Args) -> Result<RustPlay> {
        // Send commands to player
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));

        if !args.no_term {
            Self::setup_term()?;
        }

        let (w, h) = terminal::size()?;
        let scripting = Scripting::new().unwrap();

        let templ = Template::new(&scripting.get_template(), w as usize, 10)?;
        let use_color = !args.no_color;

        let th = templ.height();
        let (x, y) = if args.visualizer == VisualizerPos::Right {
            (73, 0)
        } else {
            (1, 9)
        };

        let data_zip = include_bytes!("oldplay.zip");
        let data_dir = if let Some(cache_dir) = dirs::cache_dir() {
            let dest_dir = cache_dir.join("oldplay-data");
            if !dest_dir.exists() {
                extract_zip(data_zip, &dest_dir)?;
            }
            dest_dir
        } else {
            dirs::home_dir().expect("User must have a home dir")
        };

        musix::init(&data_dir)?;

        let indexer = RemoteIndexer::new()?;

        if args.songs.is_empty() {
            let test_song: PathBuf = "music.mod".into();
            if test_song.is_file() {
                indexer.add_path(&test_song)?;
            }
        } else {
            for song in &args.songs {
                indexer.add_path(song)?;
            }
        }

        let current_list = indexer.get_all_songs();

        Ok(RustPlay {
            cmd_producer,
            info_consumer,
            templ,
            msec: msec.clone(),
            player_thread: Some(crate::player::run_player(
                &args,
                info_producer,
                cmd_consumer,
                msec,
                crate::player::CpalBackend,
            )?),
            fft_pos: args.visualizer,
            state: State {
                changed: true,
                use_color: !args.no_color,
                width: i32::from(w),
                height: th as i32,
                ..State::default()
            },
            height: h.into(),
            no_term: args.no_term,
            indexer,
            menu_component: gui::SongMenu {
                height: h.into(),
                use_color,
                ..gui::SongMenu::default()
            },
            search_component: gui::SearchField::new(th),
            fft_component: gui::Fft {
                data: Vec::new(),
                use_color,
                x,
                y,
                height: args.visualizer_height as i32,
            },
            current_list,
            current_song: 0,
            scripting
        })
    }
    fn setup_term() -> io::Result<()> {
        terminal::enable_raw_mode()?;
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
        terminal::disable_raw_mode()
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

    fn write_field(&self, key: &str, val: impl Display) -> Result<()> {
        if let Some(ph) = self.templ.get_placeholder(key) {
            let text = format!("{val}");
            let l = usize::min(text.len(), ph.len);
            stdout()
                .queue(cursor::MoveTo(ph.col as u16, ph.line as u16))?
                .queue(Print(&text[..l]))?;
        }
        Ok(())
    }

    /// Draw the info panel with all song metadata
    fn draw_info(&self) -> Result<()> {
        let mut out = stdout();
        out.queue(Clear(ClearType::All))?
            .queue(self.fg_color(Color::Cyan))?;
        for (i, line) in self.templ.lines().iter().enumerate() {
            out.queue(cursor::MoveTo(0, i as u16))?.queue(Print(line))?;
        }

        let overrides = self.scripting.get_overrides(&self.state.meta).unwrap();

        // TODO: Consider Rc<RefCell> to avoid full map clones below

        //let rhai_map = RustPlay::to_rhai_map(&self.state.meta);
        for (name, ph) in self.templ.place_holders() {
            let mut color: u32 = ph.color;
            let mut val: Option<Value> = None;

            if let Some(o) = overrides.get(name) {
                if o.value != Value::Unknown {
                    val = Some(o.value.clone());
                }
                color = o.color.unwrap_or(color);
            }
            if val.is_none() {
                val = self.state.meta.get(name).cloned();
            }
            if let Some(v) = val {
                let text = format!("{v}");
                let l = usize::min(text.len(), ph.len);
                if self.state.use_color {
                    stdout().queue(SetForegroundColor(make_color(color)))?;
                }
                stdout()
                    .queue(cursor::MoveTo(ph.col as u16, ph.line as u16))?
                    .queue(Print(&text[..l]))?;
            }
        }
        Ok(())
    }

    pub fn draw_screen(&mut self) -> Result<()> {
        let play_time = self.msec.load(Ordering::SeqCst);
        if !self.state.player_started {
            if let Some(cl) = &self.current_list {
                if cl.len() > 0 {
                    let song = cl.get(0);
                    log!("Staring with song {:?}", &song.path);
                    self.play_song(&song);
                    self.state.player_started = true;
                }
            }
        }
        // TODO: Separate update() function for things like this
        if self.state.len_msec > 0 && play_time > self.state.len_msec {
            self.next();
        }

        if self.no_term {
            return Ok(());
        }

        let black_bg = self.bg_color(Color::Rgb { r: 0, g: 0, b: 0 });
        let normal_bg = SetReverse(false);

        let mut out = stdout();

        out.queue(normal_bg)?.queue(&black_bg)?.flush()?;
        if self.state.changed {
            self.state.changed = false;
            self.draw_info()?;
        }

        if self.state.mode == InputMode::ResultScreen {
            self.menu_component.draw(&mut self.indexer)?;
            return Ok(());
        }

        if self.state.mode == InputMode::SearchInput {
            self.search_component.draw()?;
        } else {
            out.queue(cursor::MoveTo(0, self.search_component.ypos as u16 + 1))?
                .queue(self.fg_color(Color::Grey))?
                .queue(Print("[s] = search, [Ctrl-C] = quit, [n] = next"))?;
        }
        out.queue(&black_bg)?;

        if self.indexer.working() {
            if let Some((x, y)) = self.templ.get_pos("count") {
                out.queue(cursor::MoveTo(x, y))?
                    .queue(Print(format!("{}", self.indexer.index_count())))?
                    .flush()?;
            }
        }

        if self.fft_pos != VisualizerPos::None {
            if let Some(Value::Data(data)) = self.state.meta.get("fft") {
                self.fft_component.update(data);
            }
            self.fft_component.draw()?;
        }

        if self.state.show_error > 0 {
            self.state.show_error -= 1;
            let err: &str = self.state.errors.front().map_or("", |s| s.as_str());
            let x = self.state.width - err.len() as i32 - 2;
            let y = self.state.height - 1;

            out.queue(cursor::MoveTo(x as u16, y as u16))?
                .queue(self.fg_color(Color::Red))?
                .queue(Print(err))?;
            if self.state.show_error == 0 {
                self.state.errors.pop_front();
                self.state.changed = true;
            }
        } else if !self.state.errors.is_empty() {
            let l = self.state.errors.len();
            self.state.show_error = match l {
                5.. => 1,
                2..5 => 10,
                _ => 50,
            };
            log!("Error for {} frames", self.state.show_error);
        }

        let play_time = self.msec.load(Ordering::SeqCst);
        let c = (play_time / 10) % 100;
        let m = play_time / 60000;
        let s = (play_time / 1000) % 60;
        self.write_field("time", format!("{m:02}:{s:02}:{c:02}"))?;
        out.flush()?;
        Ok(())
    }

    // The passed function is sent to the player thread for execution, so must be `Send`,
    // and also `'static` since we have not tied it to the lifetime of the player.
    fn send_cmd(&mut self, f: impl (FnOnce(&mut Player) -> PlayResult) + Send + 'static) {
        self.cmd_producer
            .send(Box::new(f))
            .expect("Only fails when other end has quit");
    }

    fn search(&mut self, query: &str) -> Result<()> {
        log!("Searching for {}", query);
        self.indexer.search(query)?;
        Ok(())
    }

    fn set_song(&mut self, mut song: u32) {
        if song == 0 {
            song = 10;
        }
        self.send_cmd(move |player| player.set_song(song as i32));
    }

    fn handle_resize(&mut self, width: u16, height: u16) {
        self.state.width = width as i32;
        self.state.height = height as i32;
        self.state.changed = true;
        self.height = height as usize;

        // Template will be redrawn on next render with new size
    }

    pub fn handle_keys(&mut self) -> Result<bool> {
        if self.no_term {
            return Ok(false);
        }
        let ms = std::time::Duration::from_millis(40);
        if !event::poll(ms)? {
            return Ok(false);
        }
        let e = event::read()?;
        match e {
            Event::Resize(width, height) => {
                self.handle_resize(width, height);
            }
            Event::Key(key) => {
                if key.kind == event::KeyEventKind::Press {
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    let mut handled = true;
                    match key.code {
                        KeyCode::Char('c') if ctrl => self.state.quit = true,
                        KeyCode::Char('n') if ctrl => self.next(),
                        KeyCode::Char('p') if ctrl => self.prev(),
                        KeyCode::Char('y') if ctrl => self.send_cmd(Player::play_pause),
                        KeyCode::Right => self.send_cmd(Player::next_song),
                        KeyCode::Left => self.send_cmd(Player::prev_song),
                        _ => handled = false,
                    }
                    if !handled {
                        if self.state.mode == InputMode::Main {
                            self.state.last_mode = InputMode::Main;
                            match key.code {
                                KeyCode::Char(d) if d.is_ascii_digit() => {
                                    self.set_song(d.to_digit(10).unwrap());
                                }
                                KeyCode::Char('i' | 's') => self.state.mode = InputMode::SearchInput,
                                KeyCode::Char('n') => self.next(),
                                KeyCode::Char(' ') => self.send_cmd(Player::play_pause),
                                KeyCode::Char('p') => self.prev(),
                                KeyCode::Char('f') => self.send_cmd(|player| player.ff(10000)),
                                KeyCode::Right => self.send_cmd(Player::next_song),
                                KeyCode::Left => self.send_cmd(Player::prev_song),
                                KeyCode::PageUp | KeyCode::PageDown | KeyCode::Up | KeyCode::Down => {
                                    if self.indexer.result_len() > 0 {
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component.handle_key(&mut self.indexer, key)?;
                                    }
                                }
                                _ => {}
                            }
                        } else if self.state.mode == InputMode::ResultScreen {
                            match self.menu_component.handle_key(&mut self.indexer, key)? {
                                KeyReturn::PlaySong(song) => {
                                    self.current_list = self.indexer.get_song_result();
                                    if let Some(cl) = &self.current_list {
                                        self.current_song = cl.index_of(&song).unwrap_or(0);
                                    }
                                    self.play_song(&song);
                                    self.state.changed = true;
                                    self.state.mode = self.state.last_mode;
                                }
                                KeyReturn::ExitMenu => {
                                    self.state.changed = true;
                                    self.state.mode = self.state.last_mode;
                                }
                                KeyReturn::Navigate => {
                                    self.state.changed = true;
                                    self.state.mode = InputMode::SearchInput;
                                    self.search_component.handle_key(key)?;
                                }
                                _ => {}
                            }
                        } else if self.state.mode == InputMode::SearchInput {
                            self.state.last_mode = InputMode::SearchInput;
                            match self.search_component.handle_key(key)? {
                                KeyReturn::Search(query) => {
                                    self.search(&query)?;
                                    if self.indexer.result_len() > 0 {
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component.start_pos = 0;
                                        self.menu_component.selected = 0;
                                    } else {
                                        log!("Pushing error");
                                        self.state.errors.push_back("No results from search".into());
                                    }
                                }
                                KeyReturn::ExitMenu => {
                                    self.state.changed = true;
                                    self.state.mode = InputMode::Main;
                                }
                                KeyReturn::Navigate => {
                                    self.state.changed = true;
                                    if self.indexer.result_len() > 0 {
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component.handle_key(&mut self.indexer, key)?;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(self.state.quit)
    }

    fn get_song(&self, n: usize) -> Option<FileInfo> {
        if let Some(cl) = &self.current_list {
            if n < cl.len() {
                return Some(cl.get(n));
            }
        }
        None
    }

    pub(crate) fn play_song(&mut self, song: &FileInfo) {
        self.state.clear_meta();
        for (name, val) in &song.meta_data {
            log!("INDEX-META {name} = {val}");
            self.state.update_meta(name, val.clone());
        }
        if let Some(fname) = song.path().file_stem() {
            let s = fname.to_string_lossy().to_string();
            self.state.update_meta("file_name", Value::Text(s));
        }
        if let Some(next_song) = self.get_song(self.current_song + 1) {
            self.state
                .update_meta("next_song", Value::Text(next_song.full_song_name()));
        }

        let path = song.path().to_owned();
        self.send_cmd(move |player| player.load(&path));
    }

    pub fn prev(&mut self) {
        if let Some(cl) = &self.current_list {
            if self.current_song > 1 {
                self.current_song -= 1;
            }
            let song = cl.get(self.current_song);
            self.play_song(&song);
        }
    }
    pub fn next(&mut self) {
        if let Some(cl) = &self.current_list {
            if (self.current_song + 1) < cl.len() {
                self.current_song += 1;
            }
            let song = cl.get(self.current_song);
            self.play_song(&song);
        }
    }

    /// Update rustplay, read any meta data from player etc
    pub fn update(&mut self) {
        if self.state.done {
            self.next();
            self.state.done = false;
        }
        while let Ok((meta, val)) = self.info_consumer.try_recv() {
            if meta != "fft" {
                log!("SONG-META {} = {}", meta, val);
            }
            self.state.update_meta(&meta, val);
        }

        if let Some(Value::Number(len)) = self.state.meta.get("length") {
            self.state.len_msec = (len * 1000.0) as usize;
        }

    }

    /// Add a path to the indexer
    pub fn add_path(&mut self, song: &Path) -> Result<()> {
        self.indexer.add_path(song)?;
        Ok(())
    }

    /// Quit rustplay.
    ///
    /// # Panic
    ///
    /// Will panic if the player thread could not be joined.
    pub fn quit(&mut self) -> Result<()> {
        if !self.no_term {
            RustPlay::restore_term()?;
        }
        if self.cmd_producer.send(Box::new(Player::quit)).is_err() {
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

#[cfg(test)]
mod tests {

    use super::RustPlay;
    use crate::Args;

    #[test]
    fn rustplay_starts() {
        let args = Args {
            no_term: true,
            ..Args::default()
        };
        let mut rp = RustPlay::new(args).unwrap();
        rp.quit().unwrap();
    }
}
