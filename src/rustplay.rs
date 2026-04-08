use anyhow::Result;
use crossterm::event::KeyEvent;
use crossterm::style::SetBackgroundColor;
use gui::KeyReturn;
use scripting::Scripting;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Display;
use std::io::{self, Write as _, stdout};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;
use std::{fs, panic, thread::JoinHandle};

use crate::media_keys::{self, MediaKeyEvent, MediaKeyInfo};
use crate::player::{Cmd, Info, PlayResult, PlayState, Player, init_music};
use crate::rustplay::gui::MenuNav;
use crate::rustplay::indexer::SongIndexer;
use crate::rustplay::state::Msg;
use crate::templ::Template;
use crate::utils::make_color;
use crate::value::Value;
use crate::{Args, CONFIG_LUA, log};
use crossterm::{
    QueueableCommand, cursor,
    event::{self, Event, KeyCode},
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

use song::{FileInfo, FileType, SongArray, SongCollection};

use indexer::RemoteSongIndexer;
use state::{InputMode, State};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MenuId {
    Search,
    Favorites,
    Dir,
}

/// The RustPlay application
pub struct RustPlay {
    cmd_producer: mpsc::Sender<Cmd>,
    info_consumer: mpsc::Receiver<(String, Value)>,
    templ: Template,
    msec: Arc<AtomicUsize>,
    player_thread: Option<JoinHandle<Result<()>>>,
    state: State,
    height: usize,
    no_term: bool,
    indexer: RemoteSongIndexer,
    menus: HashMap<MenuId, gui::SongMenu>,
    current_menu: MenuId,
    search_component: gui::SearchField,
    fft_component: gui::Fft,
    fft_queue: VecDeque<(Instant, Vec<u8>)>,
    current_playlist: Rc<dyn SongCollection>,
    current_song: usize,
    scripting: Option<Scripting>,
    media_keys_receiver: mpsc::Receiver<MediaKeyEvent>,
    media_sender: mpsc::Sender<MediaKeyInfo>,
    favorites_dir: PathBuf,
    start_dir: PathBuf,
    current_dir: PathBuf,
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

        let script_path = dirs::config_dir()
            .map(|d| d.join("oldplay"))
            .unwrap_or_default()
            .join("config.lua");

        let script = if script_path.is_file() {
            std::fs::read_to_string(&script_path)?
        } else {
            CONFIG_LUA.to_string()
        };
        let scripting = Scripting::new(script)?;

        let settings = scripting.get_settings();

        let templ = Template::new(&scripting.get_template(), w as usize, 10)?;
        let scripting = Some(scripting);
        let use_color = !args.no_color;

        let th = templ.height();

        let (x, y) = templ.get_pos("fft").unwrap_or((1, 9));

        init_music();

        let indexer = RemoteSongIndexer::new()?;
        indexer.ignore_cache(args.ignore_cache)?;

        let home_dir = dirs::home_dir().expect("User should have a home dir");
        let mut start_dir = home_dir.clone();

        if args.songs.is_empty() {
            let test_song: PathBuf = "music.mod".into();
            if test_song.is_file() {
                indexer.add_path(&test_song)?;
            }
        } else {
            for song in &args.songs {
                if song.is_file() {
                    start_dir = song.parent().unwrap_or(Path::new("")).into();
                } else {
                    start_dir = song.into();
                }
                indexer.add_path(song)?;
            }
        }

        let current_list = indexer.get_all_songs();
        let favorites_dir = home_dir.join(".opfavorites");
        let audio_delay_us = Arc::new(AtomicUsize::new(0));

        let songs = Self::load_favorites(&favorites_dir);
        let mut fav_menu = gui::SongMenu::new(use_color, w.into(), h.into());
        fav_menu.set_songs("Favorites", Rc::new(SongArray { songs }));

        let (sx, sy) = templ.get_pos("search").unwrap_or((1, (th + 1) as u16));

        let height = settings.fft.visualizer_height as i32;
        let fft_component = gui::Fft {
            data: Vec::new(),
            use_color,
            x,
            y,
            height,
            bar_width: settings.fft.bar_width,
            gap: settings.fft.bar_gap,
            colors: gui::interpolate_colors(&settings.fft.colors, height as usize),
        };

        Ok(RustPlay {
            cmd_producer,
            info_consumer,
            templ,
            msec: msec.clone(),
            player_thread: Some(crate::player::run_player(
                &settings,
                info_producer,
                cmd_consumer,
                msec,
                audio_delay_us,
                crate::player::CpalBackend,
            )?),
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
            menus: HashMap::from([
                (
                    MenuId::Search,
                    gui::SongMenu::new(use_color, w.into(), h.into()),
                ),
                (MenuId::Favorites, fav_menu),
                (
                    MenuId::Dir,
                    gui::SongMenu::new(use_color, w.into(), h.into()),
                ),
            ]),
            current_menu: MenuId::Dir,
            search_component: gui::SearchField::new(sx, sy, use_color),
            fft_component,
            fft_queue: VecDeque::new(),
            current_playlist: current_list.clone(),
            current_song: 0,
            scripting,
            media_keys_receiver,
            media_sender,
            favorites_dir,
            current_dir: start_dir.clone(),
            start_dir,
        })
    }
    fn current_menu(&mut self) -> &mut gui::SongMenu {
        self.menus
            .get_mut(&self.current_menu)
            .expect("Menu should exist")
    }

    fn get_menu(&mut self, id: &MenuId) -> &mut gui::SongMenu {
        self.menus.get_mut(id).expect("All menus should exist")
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
    fn draw_info(&mut self) -> Result<()> {
        let mut out = stdout();
        out.queue(Clear(ClearType::All))?
            .queue(self.fg_color(Color::Cyan))?;
        for (i, line) in self.templ.lines().iter().enumerate() {
            out.queue(cursor::MoveTo(0, i as u16))?.queue(Print(line))?;
        }

        let overrides = self
            .scripting
            .as_mut()
            .expect("scripting should exist")
            .get_overrides(&self.state.meta)?;

        // TODO: Consider Rc<RefCell> to avoid full map clones below

        //let rhai_map = RustPlay::to_rhai_map(&self.state.meta);
        for (name, ph) in self.templ.place_holders() {
            let mut color: u32 = ph.color;
            let mut val: Option<Value> = None;

            let mut name = name.clone();
            if let Some(o) = overrides.get(&name) {
                if o.value != Value::Unknown {
                    val = Some(o.value.clone());
                }
                color = o.color.unwrap_or(color);
                if let Some(alias) = &o.alias {
                    name = alias.clone();
                }
            }
            if val.is_none() {
                val = self.state.meta.get(&name).cloned();
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
        if !self.state.player_started && !self.current_playlist.is_empty() {
            let song = self.current_playlist.get(0);
            log!("Staring with song {:?}", &song.path);
            self.play_song(&song);
            self.state.player_started = true;
        }
        // TODO: Separate update() function for things like this
        if self.state.len_msec > 0 && play_time > self.state.len_msec {
            self.next_song();
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
            self.current_menu().refresh();
            self.draw_info()?;
        }

        if self.state.mode == InputMode::ResultScreen {
            self.current_menu().draw()?;
            return Ok(());
        }

        if self.state.mode == InputMode::SearchInput {
            self.search_component.draw()?;
        } else {
            let scripting = self.scripting.as_ref().expect("scripting should exist");

            let info: String = scripting.info.clone().unwrap_or(
                "[s] = search, [f] = favorites, [a] = add favorite, [n] = next".to_string(),
            );
            out.queue(cursor::MoveTo(
                self.search_component.xpos,
                self.search_component.ypos,
            ))?
            .queue(self.fg_color(Color::Grey))?
            .queue(Print(info))?;
        }
        out.queue(&black_bg)?;

        if self.indexer.working()
            && let Some((x, y)) = self.templ.get_pos("count")
        {
            out.queue(cursor::MoveTo(x, y))?
                .queue(Print(format!("{}", self.indexer.index_count())))?
                .flush()?;
        }

        let play_time = self.msec.load(Ordering::SeqCst);
        self.write_field(
            "time",
            format!(
                "{:02}:{:02}:{:02}",
                play_time / 60000,
                (play_time / 1000) % 60,
                (play_time / 10) % 100,
            ),
        )?;

        // Pop delayed FFT frames whose display time has arrived
        let now = Instant::now();
        while let Some((display_at, _)) = self.fft_queue.front() {
            if *display_at <= now {
                let (_, data) = self.fft_queue.pop_front().unwrap();
                self.fft_component.update(&data);
            } else {
                break;
            }
        }
        self.fft_component.draw()?;

        if self.state.show_error > 0 {
            self.state.show_error -= 1;
            let Some(err) = self.state.messages.front() else {
                return Ok(());
            };
            let (text, color) = match err {
                Msg::Err(t) => (t, Color::Red),
                Msg::Info(t) => (t, Color::Yellow),
            };
            let x = self.state.width - text.len() as i32 - 2;
            let y = self.state.height - 1;

            out.queue(cursor::MoveTo(x as u16, y as u16))?
                .queue(self.fg_color(color))?
                .queue(Print(text))?;
            if self.state.show_error == 0 {
                self.state.messages.pop_front();
                self.state.changed = true;
            }
        } else if !self.state.messages.is_empty() {
            let l = self.state.messages.len();
            self.state.show_error = match l {
                5.. => 1,
                2..5 => 10,
                _ => 50,
            };
            log!("Error for {} frames", self.state.show_error);
        }

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
        let songs = self.indexer.search(query)?;
        self.get_menu(&MenuId::Search)
            .set_songs(query, Rc::new(SongArray { songs }));
        if self.show_search_result() {
            self.current_menu = MenuId::Search;
            self.state.mode = InputMode::ResultScreen;
        } else {
            log!("Pushing error");
            self.state
                .messages
                .push_back(Msg::Err("No results from search".into()));
        }
        Ok(())
    }

    fn set_song(&mut self, mut song: u32) {
        if song == 0 {
            song = 10;
        }
        self.send_cmd(move |player| player.set_song(song as i32));
    }

    fn next_subtune(&mut self) {
        self.send_cmd(Player::next_song);
    }

    fn prev_subtune(&mut self) {
        self.send_cmd(Player::prev_song);
    }

    fn handle_resize(&mut self, width: u16, height: u16) {
        self.state.width = width as i32;
        self.state.height = height as i32;
        self.state.changed = true;
        self.height = height as usize;
        self.templ.draw(width as usize, height as usize);
        let (x, y) = self.templ.get_pos("fft").unwrap_or((1, 9));
        self.fft_component.x = x;
        self.fft_component.y = y;
        for m in self.menus.values_mut() {
            m.resize(width as usize, height as usize);
        }
    }

    pub fn play_pause(&mut self) {
        self.send_cmd(Player::play_pause);
    }

    fn show_main(&mut self) {
        self.state.changed = true;
        self.state.mode = InputMode::Main;
    }

    fn show_directory(&mut self) -> Result<()> {
        let songs = self.indexer.browse(&self.current_dir)?;
        let dir_menu = self.menus.get_mut(&MenuId::Dir).unwrap();
        let name = self
            .current_dir
            //.strip_prefix(&self.start_dir)?
            .to_string_lossy();
        dir_menu.set_songs(name, Rc::new(SongArray { songs }));
        //dir_menu.set_info(format!("\"{name}\""), "[/] = Parent");
        self.current_menu = MenuId::Dir;
        self.state.mode = InputMode::ResultScreen;
        Ok(())
    }

    pub fn quit(&mut self) {
        self.state.quit = true;
    }

    pub fn show_current(&mut self) -> Result<()> {
        if self.current_menu == MenuId::Dir {
            self.show_directory()?;
        }
        self.state.mode = InputMode::ResultScreen;
        Ok(())
    }

    pub fn show_favorites(&mut self) {
        if !self.menus[&MenuId::Favorites].songs().is_empty() {
            self.current_menu = MenuId::Favorites;
            self.state.mode = InputMode::ResultScreen;
        } else {
            self.state.info("No favorites yet");
        }
    }

    pub fn show_search_result(&mut self) -> bool {
        if !self.menus[&MenuId::Search].songs().is_empty() {
            self.current_menu = MenuId::Search;
            self.state.mode = InputMode::ResultScreen;
            true
        } else {
            false
        }
    }

    fn goto_parent(&mut self) -> Result<()> {
        if self.current_dir != self.start_dir
            && let Some(parent) = self.current_dir.parent()
        {
            self.current_dir = parent.into();
            self.show_directory()?;
        }
        Ok(())
    }

    pub fn focus_search_edit(&mut self) {
        self.state.mode = InputMode::SearchInput;
    }
    pub fn input_mode(&self) -> InputMode {
        if self.state.mode == InputMode::ResultScreen {
            match self.current_menu {
                MenuId::Search => InputMode::SearchScreen,
                MenuId::Favorites => InputMode::FavScreen,
                MenuId::Dir => InputMode::DirScreen,
            }
        } else {
            self.state.mode
        }
    }

    fn enter_or_play_selected(&mut self) -> Result<()> {
        let menu = self.current_menu();
        let song = menu.get_current();
        if song.file_type == FileType::Dir {
            self.current_dir = song.path;
            self.show_directory()?;
        } else {
            self.current_playlist = self.current_menu().songs().clone();
            self.current_song = self.current_playlist.index_of(&song).unwrap_or(0);
            self.play_song(&song);
            self.state.changed = true;
            self.state.mode = InputMode::Main;
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        let mut scripting = self.scripting.take().unwrap();
        let mode = self.input_mode();
        let handled_by_script = scripting.handle_key(self, key.code, key.modifiers, mode)?;
        self.scripting = Some(scripting);
        if handled_by_script {
            return Ok(self.state.quit);
        }
        if self.state.mode == InputMode::ResultScreen {
            let menu = self.current_menu();
            match key.code {
                KeyCode::Up => menu.handle_nav(MenuNav::Up),
                KeyCode::Down => menu.handle_nav(MenuNav::Down),
                KeyCode::PageUp => menu.handle_nav(MenuNav::PageUp),
                KeyCode::PageDown => menu.handle_nav(MenuNav::PageDown),
                _ => false,
            };
        } else if self.state.mode == InputMode::SearchInput {
            self.state.last_mode = InputMode::SearchInput;
            match self.search_component.handle_key(key)? {
                KeyReturn::Search(query) => {
                    self.search(&query)?;
                }
                KeyReturn::ExitMenu => {
                    self.state.changed = true;
                    self.state.mode = InputMode::Main;
                }
                _ => {}
            }
        }
        Ok(false)
    }

    pub fn update(&mut self) -> Result<()> {
        if self.state.done {
            self.next_song();
            self.state.done = false;
        }
        let mut next_fft_at = None;
        while let Ok((meta, val)) = self.info_consumer.try_recv() {
            if meta != "fft" && meta != "fft_at" {
                log!("SONG-META {} = {}", meta, val);
            }

            if meta == "quit" {
                self.state.quit = true;
                continue;
            } else if meta == "song_files"
                && let Value::Files(files) = &val
            {
                self.state.song_files = files.clone();
            } else if meta == "fft_at"
                && let Value::Instant(at) = val
            {
                next_fft_at = Some(at);
                continue;
            } else if meta == "fft"
                && let Value::Data(data) = val
            {
                let display_at = next_fft_at.take().unwrap_or_else(Instant::now);
                self.fft_queue.push_back((display_at, data));
                continue;
            } else if meta == "state"
                && let Value::State(n) = val
            {
                log!("state: {:?}", n);
                match n {
                    PlayState::Stopped => self.media_sender.send(MediaKeyInfo::Paused)?,
                    PlayState::Paused => self.media_sender.send(MediaKeyInfo::Paused)?,
                    PlayState::Playing => self.media_sender.send(MediaKeyInfo::Playing)?,
                    _ => (),
                }
            } else if meta == "title"
                && let Value::Text(title) = &val
            {
                self.media_sender
                    .send(MediaKeyInfo::Title(title.to_string()))?
            } else if meta == "composer"
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
                MediaKeyEvent::Next => self.next_song(),
                MediaKeyEvent::Previous => self.prev_song(),
                MediaKeyEvent::Play => self.play_pause(),
                MediaKeyEvent::Pause => self.play_pause(),
                MediaKeyEvent::PlayPause => self.play_pause(),
                _ => (),
            }
        }
        Ok(())
    }

    pub fn handle_events(&mut self) -> Result<bool> {
        if self.no_term {
            return Ok(false);
        }
        if self.state.quit {
            return Ok(true);
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
                    self.handle_key(key)?;
                }
            }
            _ => {}
        }
        Ok(self.state.quit)
    }

    fn get_song(&self, n: usize) -> Option<FileInfo> {
        if n < self.current_playlist.len() {
            return Some(self.current_playlist.get(n));
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

    pub(crate) fn play_file(&mut self, file_name: String) {
        self.play_song(&FileInfo {
            path: PathBuf::from(file_name),
            ..Default::default()
        });
    }

    pub fn prev_song(&mut self) {
        if !self.current_playlist.is_empty() {
            if self.current_song > 0 {
                self.current_song -= 1;
            }
            let song = self.current_playlist.get(self.current_song);
            self.play_song(&song);
        }
    }
    pub fn next_song(&mut self) {
        if !self.current_playlist.is_empty() {
            if (self.current_song + 1) < self.current_playlist.len() {
                self.current_song += 1;
            }
            let song = self.current_playlist.get(self.current_song);
            self.play_song(&song);
        }
    }

    /// Update rustplay, read any meta data from player etc
    /// Add a path to the indexer
    pub fn add_path(&mut self, song: &Path) -> Result<()> {
        self.indexer.add_path(song)
    }

    fn load_favorites(favorites_dir: &Path) -> Vec<FileInfo> {
        let mut songs = vec![];
        if let Ok(entries) = fs::read_dir(favorites_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                // TODO: HACK! we need to know which files are secondary for real
                if path.is_file()
                    && path
                        .extension()
                        .map(|e| e != "meta" && e != "smpl")
                        .unwrap_or(true)
                {
                    log!("{path:?}");
                    let file_info = SongIndexer::identify_song(&path);
                    songs.push(file_info);
                }
            }
        }
        songs
    }

    fn get_playing_song(&self) -> Option<FileInfo> {
        if self.current_song >= self.current_playlist.len() {
            return None;
        }
        let mut song = self.current_playlist.get(self.current_song);
        let skip_tags: HashSet<&str> = [
            // "message",
            "startSong",
            "len",
            "new",
            "isong",
            "next_song",
            // "file_name",
        ]
        .into();
        for (key, value) in &self.state.meta {
            if skip_tags.contains(key.as_str()) {
                continue;
            }
            song.meta_data.insert(key.clone(), value.clone());
        }
        Some(song)
    }

    fn get_selected_song(&self) -> Option<FileInfo> {
        let menu = self.menus.get(&self.current_menu).unwrap();
        Some(menu.get_current())
    }

    fn add_selected_to_favorites(&mut self) {
        let song = self.current_menu().get_current();
        if song.file_type == FileType::Dir {
            return;
        }
        self.add_favorite(song.clone());
    }

    fn add_playing_to_favorites(&mut self) {
        if self.current_song >= self.current_playlist.len() {
            return;
        }
        let song = self.current_playlist.get(self.current_song);
        self.add_favorite(song);
    }

    fn add_char(&mut self, ke: KeyEvent) -> Result<()> {
        if self.state.mode == InputMode::SearchInput {
            let _ = self.search_component.handle_key(ke)?;
        } else if self.state.mode == InputMode::ResultScreen {
            let menu = self.current_menu();
            let _ = match ke.code {
                KeyCode::Up => menu.handle_nav(MenuNav::Up),
                KeyCode::Down => menu.handle_nav(MenuNav::Down),
                KeyCode::PageUp => menu.handle_nav(MenuNav::PageUp),
                KeyCode::PageDown => menu.handle_nav(MenuNav::PageDown),
                _ => true,
            };
        }
        Ok(())
    }

    fn add_favorite(&mut self, song: FileInfo) {
        let src = song.path();
        let Some(file_name) = src.file_name() else {
            return;
        };
        if let Err(e) = fs::create_dir_all(&self.favorites_dir) {
            self.state
                .messages
                .push_back(Msg::Err(format!("Can't create favorites dir: {e}")));
            return;
        }

        log!("FILES: {:?}", self.state.song_files);
        let res = self.state.song_files.iter().try_for_each(|src| {
            let dest = self.favorites_dir.join(src.file_name().unwrap_or_default());
            fs::copy(src, &dest).map(|_| ())
        });

        let dest = self.favorites_dir.join(file_name);
        match res {
            Ok(_) => {
                self.state.info("Added to favorites");
                let meta_path = dest.with_extension(format!(
                    "{}.meta",
                    dest.extension()
                        .map(|e| e.to_string_lossy())
                        .unwrap_or_default()
                ));
                let mut table = toml::map::Map::new();
                for (key, value) in &song.meta_data {
                    match value {
                        Value::Text(s) if !s.is_empty() => {
                            table.insert(key.clone(), toml::Value::String(s.clone()));
                        }
                        Value::Number(n) => {
                            let int = *n as i64;
                            if *n == int as f64 {
                                table.insert(key.clone(), toml::Value::Integer(int));
                            } else {
                                table.insert(key.clone(), toml::Value::Float(*n));
                            }
                        }
                        _ => {}
                    }
                }
                if !table.is_empty()
                    && let Ok(toml_str) = toml::to_string(&table)
                {
                    let _ = fs::write(&meta_path, &toml_str);
                }
            }
            Err(e) => self.state.error(format!("Failed to copy: {e}")),
        }
        let songs = Self::load_favorites(&self.favorites_dir);
        self.get_menu(&MenuId::Favorites)
            .set_songs("Favorites", Rc::new(SongArray { songs }));
    }

    /// Quit rustplay.
    ///
    /// # Panic
    ///
    /// Will panic if the player thread could not be joined.
    pub fn destroy(&mut self) -> Result<()> {
        if !self.no_term {
            RustPlay::restore_term()?;
        }
        // Send quit command; if it fails, the thread already exited (possibly with an error)
        let _ = self.cmd_producer.send(Box::new(Player::quit));

        // Shutdown media keys listener
        let _ = self.media_sender.send(MediaKeyInfo::Shutdown);

        if let Some(t) = self.player_thread.take() {
            match t.join() {
                Ok(result) => result?,
                Err(err) => panic::resume_unwind(err),
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
        rp.destroy().unwrap();
    }
}
