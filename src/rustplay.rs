use std::collections::{HashMap, VecDeque};
use std::io::{self, Cursor, Write as _, stdout};
use std::panic;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::{path::Path, thread::JoinHandle};

use anyhow::Result;
use crossterm::style::SetBackgroundColor;
use gui::KeyReturn;
use musix::MusicError;

use crate::player::{Cmd, Info, PlayResult, Player};
use crate::templ::Template;
use crate::{Settings, VisualizerPos};
use crate::{log, value::*};
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

use crate::term_extra::{MaybeCommand, SetReverse};

use song::{FileInfo, SongCollection};

use indexer::RemoteIndexer;

#[derive(Default)]
struct State {
    changed: bool,
    meta: HashMap<String, Value>,
    song: i32,
    songs: i32,
    len_msec: usize,
    done: bool,
    show_error: i32,
    select_mode: bool,
    quit: bool,
    use_color: bool,
    errors: VecDeque<String>,
    player_started: bool,
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

pub(crate) struct RustPlay {
    cmd_producer: mpsc::Sender<Cmd>,
    info_consumer: mpsc::Receiver<(String, Value)>,
    templ: Template,
    msec: Arc<AtomicUsize>,
    player_thread: Option<JoinHandle<()>>,
    fft_pos: VisualizerPos,
    errors: VecDeque<String>,
    state: State,
    height: usize,
    no_term: bool,
    indexer: RemoteIndexer,
    menu_component: gui::SongMenu,
    search_component: gui::SearchField,
    fft_component: gui::Fft,
    current_list: Option<Box<dyn SongCollection>>,
    current_song: usize,
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
        templ.set_vars(settings.variables);
        let use_color = !settings.args.no_color;
        templ.set_use_color(use_color);

        let th = templ.height();
        let (x, y) = if settings.args.visualizer == VisualizerPos::Right {
            (73, 0)
        } else {
            (1, 9)
        };

        let data_zip = include_bytes!("oldplay.zip");
        let data_dir = if let Some(cache_dir) = dirs::cache_dir() {
            let dd = cache_dir.join("oldplay-data");
            if !dd.exists() {
                zip_extract::extract(Cursor::new(data_zip), &dd, false)?;
            }
            dd
        } else {
            dirs::home_dir().unwrap()
        };

        let indexer = RemoteIndexer::new()?;
        let current_list = indexer.get_all_songs();

        Ok(RustPlay {
            cmd_producer,
            info_consumer,
            templ,
            msec: msec.clone(),
            player_thread: Some(crate::player::run_player(
                &settings.args,
                info_producer,
                cmd_consumer,
                msec,
                data_dir,
            )?),
            fft_pos: settings.args.visualizer,
            errors: VecDeque::new(),
            state: State {
                changed: true,
                use_color: !settings.args.no_color,
                ..State::default()
            },
            height: h.into(),
            no_term: settings.args.no_term,
            indexer,
            menu_component: gui::SongMenu {
                height: h.into(),
                ..gui::SongMenu::default()
            },
            search_component: gui::SearchField::new(th),
            fft_component: gui::Fft {
                data: Vec::new(),
                use_color,
                x,
                y,
                height: settings.args.visualizer_height as i32,
            },
            current_list,
            current_song: 0,
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

    pub fn draw_screen(&mut self) -> Result<()> {
        let play_time = self.msec.load(Ordering::SeqCst);
        if !self.state.player_started {
            if let Some(cl) = &self.current_list {
                if cl.len() > 0 {
                    let song = cl.get(0);
                    log!("Staring with song {:?}", &song.path);
                    self.play_song(&song);
                    self.state.player_started = true
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
            stdout()
                .queue(Clear(ClearType::All))?
                .queue(self.fg_color(Color::Cyan))?;
            self.templ.write(&self.state.meta, 0, 0)?;
        }

        if self.state.select_mode {
            self.menu_component.draw(&mut self.indexer)?;
            return Ok(());
        }

        self.search_component.draw()?;
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

    // The passed function is sent to the player thread for execution, so must be Send,
    // and also 'static since we have not tied it to the lifetime of the player.
    fn send_cmd(&mut self, f: impl FnOnce(&mut Player) -> PlayResult + Send + 'static) {
        if self.cmd_producer.send(Box::new(f)).is_err() {
            panic!("");
        }
    }

    fn search(&mut self, query: &str) -> Result<()> {
        log!("Searching for {}", query);
        self.indexer.search(query)?;
        self.state.select_mode = true;
        Ok(())
    }

    fn set_song(&mut self, song: u32) {
        self.send_cmd(move |player| player.set_song(song as i32));
    }

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
                let mut handled = true;
                match key.code {
                    KeyCode::Char(d) if d.is_ascii_digit() && ctrl => {
                        self.set_song(d.to_digit(10).unwrap())
                    }
                    KeyCode::Char('c') if ctrl => self.state.quit = true,
                    KeyCode::Char('n') if ctrl => self.next(),
                    KeyCode::Char('p') if ctrl => self.prev(),
                    KeyCode::Char('f') => self.send_cmd(|player| player.ff(10000)),
                    KeyCode::Right => self.send_cmd(Player::next_song),
                    KeyCode::Left => self.send_cmd(Player::prev_song),
                    _ => handled = false,
                }
                if !handled {
                    if self.state.select_mode {
                        match self.menu_component.handle_key(&mut self.indexer, key)? {
                            KeyReturn::PlaySong(song) => {
                                self.current_list = self.indexer.get_song_result();
                                if let Some(cl) = &self.current_list {
                                    self.current_song = cl.index_of(&song).unwrap_or(0);
                                }
                                self.play_song(&song);
                                self.state.changed = true;
                                self.state.select_mode = false;
                            }
                            KeyReturn::ExitMenu => {
                                self.state.changed = true;
                                self.state.select_mode = false;
                            }
                            KeyReturn::Navigate => {
                                self.state.changed = true;
                                self.state.select_mode = false;
                                self.search_component.handle_key(key)?;
                            }
                            _ => {}
                        }
                    } else {
                        match self.search_component.handle_key(key)? {
                            KeyReturn::Search(query) => {
                                self.search(&query)?;
                                self.state.select_mode = true;
                                self.menu_component.start_pos = 0;
                                self.menu_component.selected = 0;
                            }
                            KeyReturn::ExitMenu => {
                                self.state.changed = true;
                                self.state.select_mode = true;
                            }
                            KeyReturn::Navigate => {
                                self.state.changed = true;
                                self.state.select_mode = true;
                                self.menu_component.handle_key(&mut self.indexer, key)?;
                            }
                            _ => {}
                        }
                    }
                }
            }
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

    pub fn play_song(&mut self, song: &FileInfo) {
        self.state.clear_meta();
        for (name, val) in song.meta_data.iter() {
            log!("INDEX-META {name} = {val}");
            self.state.update_meta(name, val.clone());
        }
        if let Some(next_song) = self.get_song(self.current_song + 1) {
            self.state
                .update_meta("next_song", Value::Text(next_song.full_song_name()));
        }

        self.state.update_title();
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

    pub fn update_meta(&mut self) {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

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
            variables: HashMap::new(),
        };
        let mut rp = RustPlay::new(settings).unwrap();
        rp.quit().unwrap();
    }
}
