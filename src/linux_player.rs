
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

