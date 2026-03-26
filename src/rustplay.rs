use anyhow::Result;
use crossterm::style::SetBackgroundColor;
use gui::KeyReturn;
use musix::MusicError;
use scripting::Scripting;
use std::fmt::Display;
use std::io::{self, Write as _, stdout};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::{fs, panic, thread::JoinHandle};

use crate::VisualizerPos;
use crate::media_keys::{self, MediaKeyEvent, MediaKeyInfo};
use crate::player::{Cmd, Info, PlayResult, PlayState, Player};
use crate::rustplay::indexer::SongIndexer;
use crate::templ::Template;
use crate::utils::{extract_zip, make_color};
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
mod scripting;
mod song;
mod state;

use crate::term_extra::{MaybeCommand, SetReverse};

use song::{FileInfo, SongArray, SongCollection};

use indexer::RemoteSongIndexer;
use state::{InputMode, State};

/// The RustPlay application
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
    indexer: RemoteSongIndexer,
    menu_component: gui::SongMenu,
    search_component: gui::SearchField,
    fft_component: gui::Fft,
    current_playing: Rc<dyn SongCollection>,
    current_selecting: Rc<dyn SongCollection>,
    current_song: usize,
    scripting: Scripting,
    media_keys_receiver: mpsc::Receiver<MediaKeyEvent>,
    media_sender: mpsc::Sender<MediaKeyInfo>,
    search_result: Rc<dyn SongCollection>,
    favorites_dir: PathBuf,
    favorites: Rc<dyn SongCollection>,
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

        let (media_sender, media_keys_receiver) = media_keys::start();

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

        let data_dir = if let Some(cache_dir) = dirs::cache_dir() {
            let dest_dir = cache_dir.join("oldplay-data");
            if !dest_dir.exists() {
                let data_zip = include_bytes!("oldplay.zip");
                extract_zip(data_zip, &dest_dir)?;
            }
            dest_dir
        } else {
            dirs::home_dir().expect("User must have a home dir")
        };

        musix::init(&data_dir)?;

        let indexer = RemoteSongIndexer::new()?;

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
        let favorites_dir = dirs::home_dir()
            .expect("User should have a home dir")
            .join(".opfavorites");

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
            menu_component: gui::SongMenu::new(use_color, w.into(), h.into()),
            search_component: gui::SearchField::new(th),
            fft_component: gui::Fft {
                data: Vec::new(),
                use_color,
                x,
                y,
                height: args.visualizer_height as i32,
            },
            current_playing: current_list.clone(),
            current_selecting: Rc::new(SongArray::default()),
            current_song: 0,
            scripting,
            media_keys_receiver,
            media_sender,
            search_result: Rc::new(SongArray::default()),
            favorites_dir,
            favorites: Rc::new(SongArray::default()),
            //browsing_favorites: false,
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
        if !self.state.player_started && !self.current_playing.is_empty() {
            let song = self.current_playing.get(0);
            log!("Staring with song {:?}", &song.path);
            self.play_song(&song);
            self.state.player_started = true;
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
            self.menu_component.refresh();
            self.draw_info()?;
        }

        if self.state.mode == InputMode::ResultScreen {
            self.menu_component.draw(&*self.current_selecting)?;
            return Ok(());
        }

        if self.state.mode == InputMode::SearchInput {
            self.search_component.draw()?;
        } else {
            out.queue(cursor::MoveTo(0, self.search_component.ypos as u16 + 1))?
                .queue(self.fg_color(Color::Grey))?
                .queue(Print(
                    "[s] = search, [f] = favorites, [a] = add favorite, [n] = next",
                ))?;
        }
        out.queue(&black_bg)?;

        if self.indexer.working()
            && let Some((x, y)) = self.templ.get_pos("count")
        {
            out.queue(cursor::MoveTo(x, y))?
                .queue(Print(format!("{}", self.indexer.index_count())))?
                .flush()?;
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
        self.search_result = Rc::new(SongArray {
            songs: self.indexer.search(query)?,
        });
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
        self.templ.draw(width as usize, height as usize);
        self.menu_component.resize(width as usize, height as usize);
    }

    pub fn play_pause(&mut self) {
        self.send_cmd(Player::play_pause);
    }

    pub fn handle_keys(&mut self) -> Result<bool> {
        if self.no_term {
            return Ok(false);
        }
        if !event::poll(std::time::Duration::from_millis(40))? {
            return Ok(false);
        }
        match event::read()? {
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
                        KeyCode::Char('y') if ctrl => self.play_pause(),
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
                                KeyCode::Char('i' | 's') => {
                                    self.state.mode = InputMode::SearchInput
                                }
                                KeyCode::Char('n') => self.next(),
                                KeyCode::Char(' ') => self.play_pause(),
                                KeyCode::Char('p') => self.prev(),
                                KeyCode::Char('a') => self.add_to_favorites(),
                                KeyCode::Char('f') => {
                                    self.load_favorites();
                                    if !self.favorites.is_empty() {
                                        self.current_selecting = self.favorites.clone();
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component.start_pos = 0;
                                        self.menu_component.selected = 0;
                                    } else {
                                        self.state.errors.push_back("No favorites yet".into());
                                    }
                                }
                                KeyCode::Char('S') => {
                                    self.current_selecting = self.search_result.clone();
                                    if !self.current_selecting.is_empty() {
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component
                                            .handle_key(&*self.current_selecting, key)?;
                                    }
                                }
                                KeyCode::Right => self.send_cmd(Player::next_song),
                                KeyCode::Left => self.send_cmd(Player::prev_song),
                                KeyCode::PageUp
                                | KeyCode::PageDown
                                | KeyCode::Up
                                | KeyCode::Down => {
                                    if !self.current_selecting.is_empty() {
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component
                                            .handle_key(&*self.current_selecting, key)?;
                                    }
                                }
                                _ => {}
                            }
                        } else if self.state.mode == InputMode::ResultScreen {
                            match self
                                .menu_component
                                .handle_key(&*self.current_selecting, key)?
                            {
                                KeyReturn::PlaySong(song) => {
                                    self.current_playing = self.current_selecting.clone();
                                    self.current_song =
                                        self.current_playing.index_of(&song).unwrap_or(0);
                                    self.play_song(&song);
                                    self.state.changed = true;
                                    self.state.mode = InputMode::Main;
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
                                    if !self.search_result.is_empty() {
                                        self.current_selecting = self.search_result.clone();
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component.start_pos = 0;
                                        self.menu_component.selected = 0;
                                    } else {
                                        log!("Pushing error");
                                        self.state
                                            .errors
                                            .push_back("No results from search".into());
                                    }
                                }
                                KeyReturn::ExitMenu => {
                                    self.state.changed = true;
                                    self.state.mode = InputMode::Main;
                                }
                                KeyReturn::Navigate => {
                                    self.state.changed = true;
                                    if !self.search_result.is_empty() {
                                        self.state.mode = InputMode::ResultScreen;
                                        self.menu_component
                                            .handle_key(&*self.current_playing, key)?;
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
        if n < self.current_playing.len() {
            return Some(self.current_playing.get(n));
        }
        None
    }

    pub(crate) fn play_song(&mut self, song: &FileInfo) {
        self.state.clear_meta();
        for (name, val) in &song.meta_data {
            log!("INDEX-META {name} = {val}");
            if name == "composer"
                && let Value::Text(composer) = val
            {
                let _ = self
                    .media_sender
                    .send(MediaKeyInfo::Author(composer.to_string()));
            }
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
        if !self.current_playing.is_empty() {
            if self.current_song > 1 {
                self.current_song -= 1;
            }
            let song = self.current_playing.get(self.current_song);
            self.play_song(&song);
        }
    }
    pub fn next(&mut self) {
        if !self.current_playing.is_empty() {
            if (self.current_song + 1) < self.current_playing.len() {
                self.current_song += 1;
            }
            let song = self.current_playing.get(self.current_song);
            self.play_song(&song);
        }
    }

    /// Update rustplay, read any meta data from player etc
    pub fn update(&mut self) -> Result<()> {
        if self.state.done {
            self.next();
            self.state.done = false;
        }
        while let Ok((meta, val)) = self.info_consumer.try_recv() {
            if meta != "fft" {
                log!("SONG-META {} = {}", meta, val);
            }
            if meta == "state"
                && let Value::State(n) = val
            {
                log!("state: {:?}", n);
                match n {
                    PlayState::Stopped => self.media_sender.send(MediaKeyInfo::Paused)?,
                    PlayState::Paused => self.media_sender.send(MediaKeyInfo::Paused)?,
                    PlayState::Playing => self.media_sender.send(MediaKeyInfo::Playing)?,
                    _ => (),
                }
            }
            if meta == "title"
                && let Value::Text(title) = &val
            {
                self.media_sender
                    .send(MediaKeyInfo::Title(title.to_string()))?
            }
            if meta == "composer"
                && let Value::Text(composer) = &val
                && !composer.is_empty()
            {
                log!("composer: {composer}");
                self.media_sender
                    .send(MediaKeyInfo::Author(composer.to_string()))?
            }
            self.state.update_meta(&meta, val);
        }

        if let Some(Value::Number(len)) = self.state.meta.get("length") {
            self.state.len_msec = (len * 1000.0) as usize;
        }
        if let Ok(cmd) = self.media_keys_receiver.try_recv() {
            match cmd {
                MediaKeyEvent::Next => self.next(),
                MediaKeyEvent::Previous => self.prev(),
                MediaKeyEvent::Play => self.play_pause(),
                MediaKeyEvent::Pause => self.play_pause(),
                MediaKeyEvent::PlayPause => self.play_pause(),
                _ => (),
            }
        }
        Ok(())
    }

    /// Add a path to the indexer
    pub fn add_path(&mut self, song: &Path) -> Result<()> {
        self.indexer.add_path(song)
    }

    fn load_favorites(&mut self) {
        let mut songs = vec![];
        if let Ok(entries) = fs::read_dir(&self.favorites_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().map(|e| e != "meta").unwrap_or(true) {
                    let file_info = SongIndexer::identify_song(&path);
                    songs.push(file_info);
                }
            }
        }
        self.favorites = Rc::new(SongArray { songs });
    }

    fn add_to_favorites(&mut self) {
        if self.current_song >= self.current_playing.len() {
            return;
        }
        let song = self.current_playing.get(self.current_song);
        let src = song.path();
        let Some(file_name) = src.file_name() else {
            return;
        };
        if let Err(e) = fs::create_dir_all(&self.favorites_dir) {
            self.state
                .errors
                .push_back(format!("Can't create favorites dir: {e}"));
            return;
        }
        let dest = self.favorites_dir.join(file_name);
        match fs::copy(src, &dest) {
            Ok(_) => {
                self.state.errors.push_back("Added to favorites".into());
                let meta_path = dest.with_extension(format!(
                    "{}.meta",
                    dest.extension()
                        .map(|e| e.to_string_lossy())
                        .unwrap_or_default()
                ));
                let mut lines = String::new();
                for (key, value) in &song.meta_data {
                    match value {
                        Value::Text(s) => lines.push_str(&format!("{key}={s}\n")),
                        Value::Number(n) => lines.push_str(&format!("{key}={n}\n")),
                        _ => {}
                    }
                }
                if !lines.is_empty() {
                    let _ = fs::write(&meta_path, &lines);
                }
            }
            Err(e) => self.state.errors.push_back(format!("Failed to copy: {e}")),
        }
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

        // Shutdown media keys listener
        let _ = self.media_sender.send(MediaKeyInfo::Shutdown);

        if let Some(t) = self.player_thread.take()
            && let Err(err) = t.join()
        {
            panic::resume_unwind(err);
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
