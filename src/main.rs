extern crate minifb;

use minifb::{Key, Window, WindowOptions};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;

use std::sync::mpsc::{channel, Sender, Receiver};
use std::{thread, time};

mod musix;
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
}

enum PlayerInfo {
    Title(String),
    Composer(String),
}

struct MusicPlayer {
    #[allow(dead_code)]
    player_thread : std::thread::JoinHandle<i32>,
    cmd_sender : Sender<Command>,
    info_receiver : Receiver<PlayerInfo>
}

impl MusicPlayer {
    pub fn create(data_path: &str) -> MusicPlayer {
        let (cmd_sender, cmd_receiver) = channel::<Command>();
        let (info_sender, info_receiver) = channel::<PlayerInfo>();
        musix::init(data_path);
        let player_thread = thread::spawn(move || {

            let mut audio_player = create_audio_player();
            let mut player = musix::ChipPlayer::new();
            let mut samples: [i16; 1024 * 8] = [0; 1024 * 8];
            loop {
                if let Result::Ok(cmd) = cmd_receiver.try_recv() {
                    match cmd {
                        Command::Play(name) => {
                            player = musix::play_song(&name);
                            let title = player.get_meta("title");
                            info_sender
                                .send(PlayerInfo::Title(title))
                                .expect("Could not send");
                            let composer = player.get_meta("composer");
                            info_sender
                                .send(PlayerInfo::Composer(composer))
                                .expect("Could not send");
                        }
                        Command::SetSong(n) => player.seek(n, 0),
                    }
                }

                let count = player.get_samples(&mut samples);
                audio_player.write(&samples[0..count]);
            }
        });

        MusicPlayer { player_thread, cmd_sender, info_receiver }

    }

    pub fn play(&mut self, file_name: &str) {
        self.cmd_sender.send(Command::Play(file_name.to_string()))
            .expect("Could not send");
    }

    pub fn set_song(&mut self, song: i32) {
        self.cmd_sender.send(Command::SetSong(song)).expect("Could not send");
    }

    pub fn get_info(&mut self) -> Option<PlayerInfo> {
        match self.info_receiver.try_recv() {
            Result::Ok(ok) => Some(ok),
            Result::Err(_) => None
        }
    }
}


fn main() {

    let args: Vec<String> = std::env::args().collect();

    let mut music_player = MusicPlayer::create("musicplayer/data");
    music_player.play(&args[1]);
    music_player.set_song(0);

    let mut window = Window::new( "Audioplayer",
        WIDTH, HEIGHT,
        WindowOptions::default(),
    ) .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    while window.is_open() && !window.is_key_down(Key::Escape) {

        if let Some(PlayerInfo::Title(title)) = music_player.get_info() {
            println!("TITLE {}", title);
        }
        if let Some(PlayerInfo::Composer(composer)) = music_player.get_info() {
            println!("COMPOSER {}", composer);
        }

        window.update();
    }
}
