use std::ffi::CString;
use std::os::raw::c_void;

use crate::audio_player::AudioPlayer;

extern crate winapi;

pub struct WinPlayer {
    id : u32
}

unsafe impl Send for WinPlayer {}


impl AudioPlayer for WinPlayer {
    fn new(hz: u32) -> WinPlayer {
	    let mut wfx = winapi::shared::mmreg::WAVEFORMATEX {
            nSamplesPerSec : 44100,
            wBitsPerSample : 16,
            nChannels : 2,
            cbSize : 0,
            wFormatTag : winapi::shared::mmreg::WAVE_FORMAT_PCM,
            nBlockAlign : 0,
            nAvgBytesPerSec : 0
        };
        wfx.nBlockAlign = (wfx.wBitsPerSample >> 3) * wfx.nChannels;
        wfx.nAvgBytesPerSec = (wfx.nBlockAlign as u32) * wfx.nSamplesPerSec;

        WinPlayer { id : 0 }
    }

    fn write(&mut self, samples: &[i16]) {
        unsafe {
            if samples.len() == 0 {
                return;
            }
        }
    }
}
