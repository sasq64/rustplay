use std::ffi::CString;
use std::os::raw::c_void;

use crate::audio_player::AudioPlayer;

use alsa_sys::*;

pub struct LinuxPlayer {
    playback_handle: *mut snd_pcm_t,
    hz: u32,
}

unsafe impl Send for LinuxPlayer {}

impl LinuxPlayer {
    fn create(&mut self) -> Result<i32, i32> {

        let default = CString::new("default").unwrap();
        unsafe {
            let err = snd_pcm_open(
                &mut self.playback_handle,
                default.as_ptr(),
                SND_PCM_STREAM_PLAYBACK,
                0,
            );
            if err < 0 {
                return Result::Err(err);
            }
            snd_pcm_set_params(
                self.playback_handle,
                SND_PCM_FORMAT_S16,
                SND_PCM_ACCESS_RW_INTERLEAVED,
                2,
                self.hz,
                1,
                30000,
            );
            return Result::Ok(0);
        }
    }
}

impl AudioPlayer for LinuxPlayer {
    fn new(hz: u32) -> LinuxPlayer {
        let mut player = LinuxPlayer {
            playback_handle: std::ptr::null_mut(),
            hz,
        };
        player.create().expect("Failed to open sound device");
        player
    }

    fn write(&mut self, samples: &[i16]) {
        unsafe {
            if samples.len() == 0 {
                return;
            }
            snd_pcm_writei(
                self.playback_handle,
                samples.as_ptr() as *const c_void,
                (samples.len() / 2) as u64,
            );
        }
    }
}
