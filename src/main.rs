extern crate minifb;

use minifb::{Key, Window, WindowOptions};

const WIDTH: usize = 640;
const HEIGHT: usize = 360;

use std::ffi::CStr;
// use std::collections::VecDeque;
use std::ffi::CString;
use std::os::raw::c_char;
use std::os::raw::c_void;
use std::sync::mpsc::channel;
// use std::sync::{Arc, Mutex};
use std::{thread, time};
// extern crate coreaudio;
// use coreaudio::audio_unit::{AudioUnit, IOType, SampleFormat};
// use coreaudio::audio_unit::render_callback::{self, data};

mod audio_player;

#[cfg(target_os = "linux")]
mod linux_player;
#[cfg(target_os = "macos")]
mod mac_player;

use crate::audio_player::AudioPlayer;

#[cfg(target_os = "macos")]
pub fn create_audio_player() -> impl AudioPlayer {
    mac_player::MacPlayer::new(44100)
}

#[cfg(target_os = "linux")]
pub fn create_audio_player() -> impl AudioPlayer {
    linux_player::LinuxPlayer::new(44100)
}

// #[link(name = "musix")]

extern "C" {
    pub fn free(__ptr: *mut ::std::os::raw::c_void);
}

extern "C" {
    fn musix_create(dataDir: *const c_char) -> i32;
    fn musix_find_plugin(musicFile: *const c_char) -> *mut c_void;
    fn musix_plugin_create_player(plugin: *mut c_void, musicFile: *const c_char) -> *mut c_void;
    fn musix_player_get_meta(player: *const c_void, what: *const c_char) -> *const c_char;
    fn musix_player_get_samples(player: *const c_void, target: *mut i16, size: i32) -> i32;

    fn musix_player_seek(player: *const c_void, song: i32, seconds: i32);

    fn musix_player_destroy(player: *const c_void);
}

pub struct ChipPlayer {
    player: *mut c_void,
}

impl ChipPlayer {
    fn get_meta(&mut self, what: &str) -> String {
        unsafe {
            let cptr = musix_player_get_meta(self.player, CString::new(what).unwrap().as_ptr());
            let meta = CStr::from_ptr(cptr).to_string_lossy().into_owned();
            free(cptr as *mut c_void);
            meta
        }
    }

    fn get_samples(&mut self, target: &mut [i16], size: usize) -> usize {
        unsafe {
            if self.player.is_null() {
                0
            } else {
                musix_player_get_samples(self.player, target.as_mut_ptr(), size as i32) as usize
            }
        }
    }
    fn seek(&mut self, song: i32, seconds: i32) {
        unsafe {
            musix_player_seek(self.player, song, seconds);
        }
    }

    fn new() -> ChipPlayer {
        ChipPlayer {
            player: std::ptr::null_mut(),
        }
    }
}

impl Drop for ChipPlayer {
    fn drop(&mut self) {
        if !self.player.is_null() {
            unsafe { musix_player_destroy(self.player) }
        }
    }
}

unsafe impl Send for ChipPlayer {}

pub fn play_song(song_file: &str) -> ChipPlayer {
    let music_file = CString::new(song_file).unwrap();
    unsafe {
        let plugin = musix_find_plugin(music_file.as_ptr());
        let player = musix_plugin_create_player(plugin, music_file.as_ptr());
        ChipPlayer { player }
    }
}

enum Command {
    Play(String),
    SetSong(i32),
}

enum PlayerInfo {
    Title(String),
}

fn main() {

    let (cmd_sender, cmd_receiver) = channel::<Command>();
    let (info_sender, info_receiver) = channel::<PlayerInfo>();
    thread::spawn(move || {
        unsafe {
            let data_dir = CString::new("musicplayer/data").unwrap();
            musix_create(data_dir.as_ptr());
        }

        let mut audio_player = create_audio_player();
        let mut player = ChipPlayer::new();
        let mut samples: [i16; 1024 * 8] = [0; 1024 * 8];
        let len = samples.len();
        loop {
            let result = cmd_receiver.try_recv();
            if let Result::Ok(cmd) = result {
                match cmd {
                    Command::Play(name) => {
                        println!("Play {}", name);
                        player = play_song(&name);
                        let title = player.get_meta("game");
                        info_sender
                            .send(PlayerInfo::Title(title))
                            .expect("Could not send");
                    }
                    Command::SetSong(n) => player.seek(n, 0),
                }
            }

            let count: usize = player.get_samples(&mut samples, len);
            audio_player.write(&samples[0..count]);
        }
    });

   //std::thread::sleep(time::Duration::from_millis(1000));
    cmd_sender
        .send(Command::Play(
            "musicplayer/music/Jugi - onward (party version).xm".to_string(),
        ))
        .expect("Could not send");

    match info_receiver.recv().unwrap() {
        PlayerInfo::Title(title) => println!("TITLE {}", title),
    }

    std::thread::sleep(time::Duration::from_millis(1000));
    cmd_sender.send(Command::SetSong(19)).expect("Could not send");

    let mut window = Window::new( "Audioplayer",
        WIDTH, HEIGHT,
        WindowOptions::default(),
    ) .unwrap_or_else(|e| {
        panic!("{}", e);
    });

    while window.is_open() && !window.is_key_down(Key::Escape) {
        window.update();
    }
}
