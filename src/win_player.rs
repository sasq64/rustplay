use std::ffi::CString;
use std::os::raw::c_void;

use crate::audio_player::AudioPlayer;

extern crate winapi;

pub struct WinPlayer {
    id: u32,
}

unsafe impl Send for WinPlayer {}

use winapi::shared::mmreg;
use winapi::um::mmeapi;
use winapi::um::mmsystem;

#[no_mangle]
pub unsafe extern "C" fn waveOutProc(
    hWaveOut: mmsystem::HWAVEOUT,
    uMsg: u32,
    dwInstance: usize,
    dwParam1: usize,
    dwParam2: usize,
) {
    if (uMsg != mmsystem::WOM_DONE) {
        return;
    }
    println!("In waveout {}", uMsg);
    //InternalPlayer *ap = (InternalPlayer*)dwInstance;
}

impl WinPlayer {
    fn init(&mut self) {
        let mut wfx = mmreg::WAVEFORMATEX {
            nSamplesPerSec: 44100,
            wBitsPerSample: 16,
            nChannels: 2,
            cbSize: 0,
            wFormatTag: mmreg::WAVE_FORMAT_PCM,
            nBlockAlign: 0,
            nAvgBytesPerSec: 0,
        };
        wfx.nBlockAlign = (wfx.wBitsPerSample >> 3) * wfx.nChannels;
        wfx.nAvgBytesPerSec = (wfx.nBlockAlign as u32) * wfx.nSamplesPerSec;
        unsafe {
            let mut hWaveOut: mmsystem::HWAVEOUT = std::ptr::null_mut();
            let wp = waveOutProc as *const c_void;
            let thiz = self as *mut _ as *mut c_void;
            if mmeapi::waveOutOpen(
                &mut hWaveOut,
                mmsystem::WAVE_MAPPER,
                &mut wfx,
                wp as usize,
                thiz as usize,
                mmsystem::CALLBACK_FUNCTION,
            ) != mmsystem::MMSYSERR_NOERROR
            {}
        }
    }
}

impl AudioPlayer for WinPlayer {
    fn new(hz: u32) -> WinPlayer {
        let mut res = WinPlayer { id: 0 };
        res.init();
        res
    }

    fn write(&mut self, samples: &[i16]) {
        unsafe {
            if samples.len() == 0 {
                return;
            }
        }
    }
}
