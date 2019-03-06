#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

include!("asoundlib.rs");

use std::ffi::CStr;

use std::collections::VecDeque;
use std::ffi::CString;
use std::os::raw::c_char;
use std::os::raw::c_void;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::{thread, time};

#[link(name = "asound")]
#[link(name = "musix")]

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

pub fn playSong(song_file: &str) -> ChipPlayer {
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

pub struct LinuxPlayer {
    playback_handle: *mut snd_pcm_t,
    hz: u32,
}

unsafe impl Send for LinuxPlayer {}

impl LinuxPlayer {
    fn create(&mut self) {
        let default = CString::new("default").unwrap();
        unsafe {
            let err = snd_pcm_open(
                &mut self.playback_handle,
                default.as_ptr(),
                _snd_pcm_stream_SND_PCM_STREAM_PLAYBACK,
                0,
            );
            snd_pcm_set_params(
                self.playback_handle,
                _snd_pcm_format_SND_PCM_FORMAT_S16,
                _snd_pcm_access_SND_PCM_ACCESS_RW_INTERLEAVED,
                2,
                self.hz,
                1,
                30000,
            );
        }
    }
}

impl AudioPlayer for LinuxPlayer {
    fn new(hz: u32) -> LinuxPlayer {
        let mut player = LinuxPlayer {
            playback_handle: std::ptr::null_mut(),
            hz,
        };
        player.create();
        player
    }

    fn write(&mut self, samples: &[i16]) {
        unsafe {
            snd_pcm_writei(
                self.playback_handle,
                samples.as_ptr() as *const c_void,
                (samples.len() / 2) as u64,
            );
        }
    }

    fn play(&mut self, callback: fn(&mut [i16])) {}
}

pub fn createAudioPlayer() -> LinuxPlayer {
    LinuxPlayer::new(44100)
}

fn main() {
    let mut samples: [i16; 1024 * 8] = [0; 1024 * 8];

    unsafe {
        let data_dir = CString::new("data").unwrap();
        musix_create(data_dir.as_ptr());
    }

    let mut audioPlayer = createAudioPlayer();
    let mut player = playSong("music/Starbuck - Tennis.mod");

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
