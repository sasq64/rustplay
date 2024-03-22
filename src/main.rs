
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;

use std::io::stdout;

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::*, widgets::*};


use musicplayer::musix;

mod fifo;
mod audio_player;

#[cfg(target_os = "linux")]
mod linux_player;

#[cfg(target_os = "macos")]
mod mac_player;

#[cfg(target_os = "windows")]
mod win_player;

use audio_player::AudioPlayer;

#[cfg(target_os = "macos")]
pub fn create_audio_player() -> impl AudioPlayer {
    mac_player::MacPlayer::new(44100)
}

#[cfg(target_os = "linux")]
pub fn create_audio_player() -> impl AudioPlayer {
    linux_player::LinuxPlayer::new(44100)
}

#[cfg(target_os = "windows")]
pub fn create_audio_player() -> impl AudioPlayer {
    win_player::WinPlayer::new(44100)
}
enum Command {
    Play(String),
    SetSong(i32),
    PrevSong(),
    NextSong(),
}


#[derive(Debug)]
enum PlayerInfo {
    Title(String),
    Composer(String),
    Game(String),
    Subtitle(String),
    Length(i32),
    Format(String),
    Song(i32),
    Songs(i32),
    Seconds(f32),
    Custom(String),
    Nothing()
}

struct MusicPlayer {
    #[allow(dead_code)]
    player_thread : std::thread::JoinHandle<i32>,
    cmd_sender : Sender<Command>,
    info_receiver : Receiver<PlayerInfo>,
}

impl MusicPlayer {

    pub fn create(data_path: &str) -> MusicPlayer {
        let (cmd_sender, cmd_receiver) = channel::<Command>();
        let (info_sender, info_receiver) = channel::<PlayerInfo>();
        musix::init(data_path);
        let player_thread = thread::spawn(move || {

            let mut audio_player = create_audio_player();
            let mut player = musix::ChipPlayer::new();
            let mut samples: [i16; 1024 * 4] = [0; 1024 * 4];
            let mut songn = 0;
            let mut seconds: f32 = 0.0;
            loop {
                if let Ok(cmd) = cmd_receiver.try_recv() {
                    match cmd {
                        Command::Play(name) => { player = musix::play_song(&name); }
                        Command::SetSong(n) => { songn = n ; player.seek(n, 0) },
                        Command::NextSong() => { songn = songn + 1 ; player.seek(songn, 0) },
                        Command::PrevSong() => { songn = songn - 1 ; player.seek(songn, 0) },
                    }
                }

                let count = player.get_samples(&mut samples);
                audio_player.write(&samples[0..count]);
                seconds += (count as f32)/(2.0*44100.0);

                info_sender.send(PlayerInfo::Seconds(seconds)).expect("Could not send");

                while let Some(what) = player.get_changed_meta() {
                    let val = player.get_meta(what.as_str());
                    //println!("{} = '{}'\n", what, val);
                    let info = match what.as_str() {
                        "composer" => PlayerInfo::Composer(val),
                        "title" => PlayerInfo::Title(val),
                        "game" => PlayerInfo::Game(val),
                        "sub_title" => PlayerInfo::Subtitle(val),
                        "length" => PlayerInfo::Length(val.parse::<i32>().unwrap()),
                        "songs" => PlayerInfo::Songs(val.parse::<i32>().unwrap()),
                        "song" => PlayerInfo::Song(val.parse::<i32>().unwrap()),
                        "format" => PlayerInfo::Format(val),
                        _ => PlayerInfo::Custom(what)
                    };
                    info_sender.send(info).expect("Could not send");
                }
            }
        });

        MusicPlayer { player_thread, cmd_sender, info_receiver  }

    }

    pub fn play(&mut self, file_name: &str) {
        self.cmd_sender.send(Command::Play(file_name.to_string()))
            .expect("Could not send");
    }

    pub fn set_song(&mut self, song: i32) {
        self.cmd_sender.send(Command::SetSong(song)).expect("Could not send");
    }

    pub fn next_song(&mut self) {
        self.cmd_sender.send(Command::NextSong()).expect("Could not send");
    }

    pub fn prev_song(&mut self) {
        self.cmd_sender.send(Command::PrevSong()).expect("Could not send");
    }

    pub fn get_info(&mut self) -> PlayerInfo {
        self.info_receiver.try_recv().unwrap_or_else(|_| PlayerInfo::Nothing())
    }
}

fn main() {

    let args: Vec<String> = std::env::args().collect();


    let mut music_player = MusicPlayer::create("data");
    music_player.play(&args[1]);
    music_player.set_song(0);
    // let mp = Rc::new(RefCell::new(music_player));

    enable_raw_mode().unwrap();
    stdout().execute(EnterAlternateScreen).unwrap();
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout())).unwrap();

    let ms = std::time::Duration::from_millis(50);

    let mut sub_title: String = "".to_string();
    let mut title: String = "Unknown".to_string();
    let mut song_composer: String = "Unknown".to_string();
    let mut length = 0;
    let mut song = 0;
    let mut songs = 1;
    let mut seconds: f32 = 0.0;

    loop {
        let mut changed = true;
        match music_player.get_info() {
            PlayerInfo::Title(s) => { title = s; },
            PlayerInfo::Game(s) => { title = s; },
            PlayerInfo::Seconds(f) => { seconds = f; },
            PlayerInfo::Subtitle(s) => {
                if s == "" { sub_title = s; } else { sub_title = format!(" ({})", s); } },
            PlayerInfo::Composer(composer) => { song_composer = composer; },
            PlayerInfo::Length(i) => { length = i; }
            PlayerInfo::Song(i) => { song = i; }
            PlayerInfo::Songs(i) => { songs = i; }
            _ => { changed = false; }
        }
        terminal.draw(|frame | {
            let secs = seconds as i32;
            let t = format!("TITLE   : {}{}\nCOMPOSER: {}\nLENGTH  : {:02}:{:02} / {:02}:{:02}\nSONG    : {:02}/{:02}",
                            title, sub_title, song_composer, secs/60, secs%60, length/60, length % 60, song+1, songs);
            frame.render_widget(
                Paragraph::new(t).block(Block::default().title("Play Music").borders(Borders::ALL)),
                frame.size(),
            )
        }).unwrap();
        if event::poll(ms).unwrap() {
            if let Event::Key(key) = event::read().unwrap() {
                if key.kind == event::KeyEventKind::Press {
                    if key.code == KeyCode::Char('q') {
                        break;
                    }
                    if key.code == KeyCode::Char('n') || key.code == KeyCode::Right {
                        music_player.next_song();
                    }
                    if key.code == KeyCode::Char('p') || key.code == KeyCode::Left {
                        music_player.prev_song();
                    }
                }
            }
        }
    }

    disable_raw_mode().unwrap();
    stdout().execute(LeaveAlternateScreen).unwrap();
}