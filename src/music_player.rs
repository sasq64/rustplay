use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;

use musicplayer::musix;

use crate::audio_player::AudioPlayer;
use crate::mac_player;

#[cfg(target_os = "macos")]
pub fn create_audio_player() -> impl AudioPlayer {
    mac_player::MacPlayer::new(44100)
}

#[cfg(target_os = "linux")]
pub fn create_audio_player() -> impl AudioPlayer {
    crate::linux_player::LinuxPlayer::new(44100)
}

#[cfg(target_os = "windows")]
pub fn create_audio_player() -> impl AudioPlayer {
    crate::win_player::WinPlayer::new(44100)
}

enum Command {
    Play(String),
    SetSong(i32),
    PrevSong(),
    NextSong(),
}

#[derive(Debug)]
pub enum PlayerInfo {
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
    Nothing(),
}

pub(crate) struct MusicPlayer {
    #[allow(dead_code)]
    player_thread: std::thread::JoinHandle<i32>,
    cmd_sender: Sender<Command>,
    info_receiver: Receiver<PlayerInfo>,
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
                        Command::Play(name) => {
                            player = musix::play_song(&name);
                        }
                        Command::SetSong(n) => {
                            songn = n;
                            player.seek(n, 0)
                        }
                        Command::NextSong() => {
                            songn = songn + 1;
                            player.seek(songn, 0)
                        }
                        Command::PrevSong() => {
                            songn = songn - 1;
                            player.seek(songn, 0)
                        }
                    }
                }

                let count = player.get_samples(&mut samples);
                audio_player.write(&samples[0..count]);
                seconds += (count as f32) / (2.0 * 44100.0);

                info_sender
                    .send(PlayerInfo::Seconds(seconds))
                    .expect("Could not send");

                while let Some(what) = player.get_changed_meta() {
                    let val = player.get_meta(what.as_str());
                    let info = match what.as_str() {
                        "composer" => PlayerInfo::Composer(val),
                        "title" => PlayerInfo::Title(val),
                        "game" => PlayerInfo::Game(val),
                        "sub_title" => PlayerInfo::Subtitle(val),
                        "length" => PlayerInfo::Length(val.parse::<i32>().unwrap()),
                        "songs" => PlayerInfo::Songs(val.parse::<i32>().unwrap()),
                        "song" => PlayerInfo::Song(val.parse::<i32>().unwrap()),
                        "format" => PlayerInfo::Format(val),
                        _ => PlayerInfo::Custom(what),
                    };
                    info_sender.send(info).expect("Could not send");
                }
            }
        });

        MusicPlayer {
            player_thread,
            cmd_sender,
            info_receiver,
        }
    }

    pub fn play(&mut self, file_name: &str) {
        self.cmd_sender
            .send(Command::Play(file_name.to_string()))
            .expect("Could not send");
    }

    pub fn set_song(&mut self, song: i32) {
        self.cmd_sender
            .send(Command::SetSong(song))
            .expect("Could not send");
    }

    pub fn next_song(&mut self) {
        self.cmd_sender
            .send(Command::NextSong())
            .expect("Could not send");
    }

    pub fn prev_song(&mut self) {
        self.cmd_sender
            .send(Command::PrevSong())
            .expect("Could not send");
    }

    pub fn get_info(&mut self) -> PlayerInfo {
        self.info_receiver
            .try_recv()
            .unwrap_or_else(|_| PlayerInfo::Nothing())
    }
}
