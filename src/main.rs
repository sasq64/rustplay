use std::ffi::CStr;

use std::collections::VecDeque;
use std::ffi::CString;
use std::os::raw::c_char;
use std::os::raw::c_void;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::{thread, time};

extern crate coreaudio;

use coreaudio::audio_unit::{AudioUnit, IOType, SampleFormat};
use coreaudio::audio_unit::render_callback::{self, data};

#[link(name = "musix")]

extern "C" {
    pub fn free(__ptr: *mut ::std::os::raw::c_void);
}

extern "C" {
    fn musix_create(dataDir: *const c_char) -> i32;
    fn musix_find_plugin(musicFile: *const c_char) -> *mut c_void;
    fn musix_plugin_create_player(
        plugin: *mut c_void,
        musicFile: *const c_char,
    ) -> *mut c_void;
    fn musix_player_get_meta(
        player: *const c_void,
        what: *const c_char,
    ) -> *const c_char;
    fn musix_player_get_samples(
        player: *const c_void,
        target: *mut i16,
        size: i32,
    ) -> i32;

    fn musix_player_seek(player: *const c_void, song: i32, seconds: i32);

    fn musix_player_destroy(player: *const c_void);
}

pub struct ChipPlayer {
    player: *mut c_void,
}

impl ChipPlayer {
    fn get_meta(&mut self, what: &str) -> String {
        unsafe {
            let cptr = musix_player_get_meta(
                self.player,
                CString::new(what).unwrap().as_ptr(),
            );
            let meta = CStr::from_ptr(cptr).to_string_lossy().into_owned();
            free(cptr as *mut c_void);
            meta
        }
    }

    fn get_samples(&mut self, target: &mut [i16], size: usize) -> usize {
        unsafe {
            musix_player_get_samples(
                self.player,
                target.as_mut_ptr(),
                size as i32,
            ) as usize
        }
    }
    fn seek(&mut self, song: i32, seconds: i32) {
        unsafe {
            musix_player_seek(self.player, song, seconds);
        }
    }
}

unsafe impl Send for ChipPlayer {}

pub fn play_song(song_file: &str) -> ChipPlayer {
    let music_file = CString::new(song_file).unwrap();
    //let data_dir = CString::new("data").unwrap();
    unsafe {
        let plugin = musix_find_plugin(music_file.as_ptr());
        let player = musix_plugin_create_player(plugin, music_file.as_ptr());
        ChipPlayer { player }
    }
}

pub trait AudioPlayer {
    fn new(hz: u32) -> Self;
    fn write(&mut self, samples: &[i16]);
    fn play(&mut self, callback: fn(&mut [i16]));
}

pub struct MacPlayer {
    hz: u32,
}

impl MacPlayer {
    fn create(&mut self) {
        let mut audio_unit = AudioUnit::new(IOType::DefaultOutput);
    }
}

impl AudioPlayer for MacPlayer {
    fn new(hz: u32) -> MacPlayer {
        let mut player = MacPlayer { hz };
        player.create();
        player
    }

    fn write(&mut self, samples: &[i16]) {
    }

    fn play(&mut self, callback: fn(&mut [i16])) {}
}




pub fn create_audio_player() -> MacPlayer {
    MacPlayer::new(44100)
}

fn main() {
    let mut samples: [i16; 1024 * 8] = [0; 1024 * 8];

    unsafe {
        let data_dir = CString::new("data").unwrap();
        musix_create(data_dir.as_ptr());
    }

    let mut audioPlayer = create_audio_player();
    let mut player = play_song("music/Starbuck - Tennis.mod");

    println!("TITLE:{}", player.get_meta("game"));
    //    player.seek(1, 0);
    //let fifo = Arc::new(Mutex::new(VecDeque::<i16>::new()));
    ////let (sender, receiver) = channel::<[i16; 1024]>();
    //let fifo_clone = fifo.clone();
    thread::spawn(move || {
        let len = samples.len();
        loop {
            let count: usize = player.get_samples(&mut samples, len);
            audioPlayer.write(&samples[0..count]);
        }
    });

    std::thread::sleep(time::Duration::from_millis(10000));
}
