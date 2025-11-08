use crate::Args;
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use fft::Fft;
use id3::{Tag, TagLike};
use itertools::Itertools;
use ringbuf::{StaticRb, traits::*};

use crate::{log, resampler::Resampler, value::Value};
use anyhow::Result;
use musix::{ChipPlayer, MusicError};

mod audio_device;
mod cpal_device;
mod fft;

use audio_device::{AudioCallback, AudioDevice};
use cpal_device::setup_audio_device;

pub(crate) trait AudioBackend {
    fn setup_audio_device(&self) -> Result<Box<dyn AudioDevice>>;
}

pub(crate) struct CpalBackend;

impl AudioBackend for CpalBackend {
    fn setup_audio_device(&self) -> Result<Box<dyn AudioDevice>> {
        setup_audio_device()
    }
}

pub(crate) struct NoSoundDevice {
    buffer_size: usize,
    playback_freq: u32,
}

impl NoSoundDevice {
    pub fn new() -> Self {
        Self {
            buffer_size: 1024,
            playback_freq: 44100,
        }
    }
}

impl AudioDevice for NoSoundDevice {
    fn play(&mut self, _callback: AudioCallback) -> Result<()> {
        // No-op: don't actually play audio
        Ok(())
    }

    fn get_buffer_size(&self) -> usize {
        self.buffer_size
    }

    fn get_playback_freq(&self) -> u32 {
        self.playback_freq
    }
}

pub(crate) struct NoSoundBackend {}

impl AudioBackend for NoSoundBackend {
    fn setup_audio_device(&self) -> Result<Box<dyn AudioDevice>> {
        Ok(Box::new(NoSoundDevice::new()))
    }
}

pub(crate) type PlayResult = Result<bool, MusicError>;

// Cmd is used for pushing commands to the player
pub(crate) type Cmd = Box<dyn FnOnce(&mut Player) -> PlayResult + Send>;
// Info is used for receiving information from the player
pub(crate) type Info = (String, Value);

fn parse_mp3<R: Read>(reader: &mut R) -> io::Result<bool> {
    let mut header: [u8; 32] = [0; 32];
    reader.read_exact(&mut header)?;
    Ok(true)
}

#[derive(Default, Debug, PartialEq, Clone, Copy)]
pub(crate) enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
    Quitting,
}

// impl From<i32> for PlayState {
//     fn from(n: i32) -> Self {
//         match n {
//             0 => PlayState::Stopped,
//             1 => PlayState::Playing,
//             2 => PlayState::Paused,
//             3 => PlayState::Quitting,
//             _ => PlayState::Stopped,
//         }
//     }
// }

// #[allow(clippy::struct_field_names)]
#[derive(Default, Debug)]
pub(crate) struct Player {
    chip_player: Option<ChipPlayer>,
    song: i32,
    songs: i32,
    millis: Arc<AtomicUsize>,
    play_state: PlayState,
    ff_msec: usize,
    new_song: Option<PathBuf>,
}

impl Player {
    pub fn reset(&mut self) {
        self.millis.store(0, Ordering::SeqCst);
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn next_song(&mut self) -> PlayResult {
        let cp = self.chip_player.as_ref().ok_or(MusicError {
            msg: "No active song".into(),
        })?;
        if self.song < (self.songs - 1) {
            cp.seek(self.song + 1, 0);
            self.reset();
        }
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn prev_song(&mut self) -> PlayResult {
        if self.song > 0
            && let Some(cp) = &self.chip_player
        {
            cp.seek(self.song - 1, 0);
            self.reset();
        }
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn set_song(&mut self, song: i32) -> PlayResult {
        if let Some(cp) = &self.chip_player {
            self.song = song;
            cp.seek(self.song - 1, 0);
            self.reset();
        }
        Ok(true)
    }

    pub fn load(&mut self, name: &Path) -> PlayResult {
        self.chip_player = None;
        self.chip_player = Some(musix::load_song(name)?);
        self.reset();
        self.new_song = Some(name.to_owned());
        self.play_state = PlayState::Playing;
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn ff(&mut self, msec: usize) -> PlayResult {
        self.ff_msec += msec;
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn play_pause(&mut self) -> PlayResult {
        self.play_state = match self.play_state {
            PlayState::Paused => PlayState::Playing,
            PlayState::Playing => PlayState::Paused,
            _ => self.play_state,
        };
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn quit(&mut self) -> PlayResult {
        self.play_state = PlayState::Quitting;
        Ok(true)
    }

    pub fn update_meta(&mut self, info_producer: &mut mpsc::Sender<Info>) -> Result<()> {
        if let Some(new_song) = self.new_song.take() {
            //println!("New song {}", new_song.to_str().unwrap());
            info_producer.push_value("new", 0)?;
            info_producer.push_value("song", 0)?;
            info_producer.push_value("songs", 1)?;
            self.song = 0;
            self.songs = 1;
            if let Some(ext) = new_song.extension()
                && ext == "mp3"
            {
                if let Ok(duration) = mp3_duration::from_path(&new_song) {
                    let secs = duration.as_secs() as i32;
                    info_producer.push_value("length", secs)?;
                }
                if let Ok(tag) = Tag::read_from_path(new_song) {
                    if let Some(album) = tag.album() {
                        info_producer.push_value("album", album)?;
                    }
                    if let Some(artist) = tag.artist() {
                        info_producer.push_value("composer", artist)?;
                    }
                    if let Some(title) = tag.title() {
                        info_producer.push_value("title", title)?;
                    }
                }
            }
        }
        if let Some(chip_player) = &mut self.chip_player {
            while let Some(meta) = chip_player.get_changed_meta() {
                let val = chip_player.get_meta_string(&meta).unwrap_or(String::new());
                let v: Value = match meta.as_str() {
                    "song" | "startSong" => {
                        self.song = val.parse::<i32>()?;
                        self.song.into()
                    }
                    "songs" => {
                        let n = val.parse::<i32>()?;
                        self.songs = n;
                        n.into()
                    }
                    "length" => {
                        let length = val.parse::<f64>()?;
                        length.into()
                    }
                    &_ => Value::Text(val),
                };
                info_producer.push_value(&meta, v)?;
            }
        }
        Ok(())
    }
}

trait PushValue {
    fn push_value<V: Into<Value>>(&mut self, name: &str, val: V) -> PlayResult;
}

impl PushValue for mpsc::Sender<Info> {
    fn push_value<V: Into<Value>>(&mut self, name: &str, val: V) -> PlayResult {
        self.send((name.to_owned(), val.into()))
            .map_err(|_| MusicError {
                msg: "Could not push".to_owned(),
            })?;
        Ok(true)
    }
}

const RING_BUFFER_SIZE: usize = 8192;
const AUDIO_THREAD_SLEEP_MS: u64 = 10;
const IDLE_SLEEP_MS: u64 = 100;

fn run_audio_loop<B: AudioBackend>(
    fft: Fft,
    mut info_producer: mpsc::Sender<Info>,
    cmd_consumer: mpsc::Receiver<Cmd>,
    msec: Arc<AtomicUsize>,
    backend: B,
) -> Result<()> {
    let mut audio_device = backend.setup_audio_device()?;
    let playback_freq = audio_device.get_playback_freq();
    let buffer_size = audio_device.get_buffer_size();
    let msec_outside = msec.clone();
    let msec_skip = msec.clone();
    let ring = StaticRb::<f32, RING_BUFFER_SIZE>::default();
    let (mut audio_sink, mut audio_faucet) = ring.split();

    let mut resampler = Resampler::new(buffer_size / 2)?;
    let mut plugin_freq = playback_freq;

    audio_device.play(Box::new(move |data: &mut [f32]| {
        if audio_faucet.pop_slice(data) > 0 {
            let ms = data.len() * 1000 / (playback_freq as usize * 2);
            msec.fetch_add(ms, Ordering::SeqCst);
        } else {
            data.fill(0.0);
        }
    }))?;

    let mut target: Vec<i16> = vec![0; buffer_size];
    let mut player = Player {
        millis: msec_outside,
        ..Player::default()
    };

    let mut last_state = player.play_state;

    while player.play_state != PlayState::Quitting {
        // Process commands
        while let Ok(cmd_fn) = cmd_consumer.try_recv() {
            if let Err(e) = cmd_fn(&mut player) {
                info_producer.push_value("error", e)?;
            }
        }

        if player.play_state != last_state {
            last_state = player.play_state;
            info_producer.push_value("state", last_state)?;
        }

        player.update_meta(&mut info_producer)?;

        if let Some(chip_player) = &mut player.chip_player {
            if player.ff_msec > 0 {
                // Fast forward mode
                let rc = chip_player.get_samples(&mut target);
                let ms = rc * 1000 / (plugin_freq as usize * 2);
                if ms > player.ff_msec {
                    player.ff_msec = 0;
                } else {
                    player.ff_msec -= ms;
                }
                msec_skip.fetch_add(ms, Ordering::SeqCst);
                if rc == 0 {
                    info_producer.push_value("done", 0)?;
                }
            } else if audio_sink.vacant_len() > target.len() * 2
                && player.play_state == PlayState::Playing
            {
                // Normal playback mode
                let rc = chip_player.get_samples(&mut target);
                if rc == 0 {
                    info_producer.push_value("done", 0)?;
                }

                // Handle frequency changes
                let hz = chip_player.get_frequency();
                if hz != plugin_freq {
                    log!("Plugin freq: {hz}");
                    plugin_freq = hz;
                    resampler.set_frequencies(plugin_freq, playback_freq)?;
                }

                // Process and resample audio
                let samples = target
                    .iter()
                    .take(rc)
                    .map(|&s16| f32::from(s16) / 32767.0)
                    .collect_vec();
                let new_samples = resampler.process(&samples)?;
                audio_sink.push_slice(new_samples);

                // Run FFT analysis on full buffers
                if rc == target.len() {
                    let data = fft.run(&samples, playback_freq)?;
                    info_producer.push_value("fft", data)?;
                }
            } else {
                thread::sleep(Duration::from_millis(AUDIO_THREAD_SLEEP_MS));
            }
        } else {
            thread::sleep(Duration::from_millis(IDLE_SLEEP_MS));
        }
    }

    info_producer.push_value("quit", 1)?;
    Ok(())
}

pub(crate) fn run_player<B: AudioBackend + Send + 'static>(
    args: &Args,
    info_producer: mpsc::Sender<Info>,
    cmd_consumer: mpsc::Receiver<Cmd>,
    msec: Arc<AtomicUsize>,
    backend: B,
) -> Result<JoinHandle<()>> {
    let fft = Fft {
        divider: args.fft_div * 2,
        min_freq: args.min_freq as f32,
        max_freq: args.max_freq as f32,
    };

    let info_producer_error = info_producer.clone();

    Ok(thread::spawn(move || {
        let main =
            || -> Result<()> { run_audio_loop(fft, info_producer, cmd_consumer, msec, backend) };
        if let Err(e) = main() {
            // Try to send error info back to main thread before terminating
            let _ = info_producer_error.send((
                "fatal_error".to_owned(),
                format!("Audio thread error: {}", e).into(),
            ));
            log!("Audio thread terminated with error: {}", e);
        }
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    use crate::Args;
    use crate::value::Value;

    use super::Cmd;
    use super::Info;

    #[test]
    fn musix_works() {
        let data = Path::new("data");
        assert!(data.is_dir());
        musix::init(data).unwrap();
        let mut chip_player = musix::load_song(Path::new("music.mod")).unwrap();
        let mut target = vec![0; 1024];
        let rc = chip_player.get_samples(&mut target);
        assert_eq!(rc, 1024);
        let mut target = vec![0; 8192];
        let rc = chip_player.get_samples(&mut target);
        assert_eq!(rc, 8192);
    }

    #[test]
    fn player_starts() {
        // Send commands to player
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));
        let args = Args { ..Args::default() };
        let data = Path::new("data");
        musix::init(data).unwrap();
        let backend = super::NoSoundBackend {};
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec, backend).unwrap();

        cmd_producer.send(Box::new(move |p| p.quit())).unwrap();
        let (key, _) = info_consumer.recv().unwrap();
        assert_eq!(key, "quit");
        player_thread.join().unwrap();
    }

    #[test]
    fn player_can_report_errors() {
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));
        let args = Args { ..Args::default() };
        musix::init(Path::new("data")).unwrap();
        let backend = super::NoSoundBackend {};
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec, backend).unwrap();

        cmd_producer.send(Box::new(move |p| p.next_song())).unwrap();
        let (_, val) = info_consumer.recv().unwrap();
        assert!(matches!(val, Value::Error(_)));

        cmd_producer.send(Box::new(move |p| p.quit())).unwrap();
        let (key, _) = info_consumer.recv().unwrap();

        assert_eq!(key, "quit");
        player_thread.join().unwrap();
    }
}
