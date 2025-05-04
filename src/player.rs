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

use cpal::{SampleFormat, SampleRate, traits::*};

use fft::Fft;
use id3::{Tag, TagLike};
use itertools::Itertools;
use ringbuf::{StaticRb, traits::*};

use crate::{Args, log, resampler::Resampler, value::Value};
use anyhow::{Context, Result};
use musix::{ChipPlayer, MusicError};

mod fft;

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
enum PlayState {
    #[default]
    Stopped,
    Playing,
    Paused,
    Quitting,
}

#[allow(clippy::struct_field_names)]
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
        if self.song > 0 {
            if let Some(cp) = &self.chip_player {
                cp.seek(self.song - 1, 0);
                self.reset();
            }
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
            if let Some(ext) = new_song.extension() {
                if ext == "mp3" {
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

fn check_mp3() {}

#[allow(clippy::too_many_lines)]
pub(crate) fn run_player(
    args: &Args,
    mut info_producer: mpsc::Sender<Info>,
    cmd_consumer: mpsc::Receiver<Cmd>,
    msec: Arc<AtomicUsize>,
) -> Result<JoinHandle<()>> {

    let device = cpal::default_host()
        .default_output_device()
        .context("No audio device available")?;

    let mut configs = device
        .supported_output_configs()
        .context("Could not get audio configs")?;
    let buffer_size = 4096 / 2;

    let sconf = configs
        .find(|conf| {
            conf.channels() == 2
                && conf.sample_format() == SampleFormat::F32
                && conf.max_sample_rate() >= cpal::SampleRate(44100)
                && conf.min_sample_rate() <= cpal::SampleRate(44100)
        })
        .context("Could not find a compatible audio config")?;
    let config = sconf.with_sample_rate(SampleRate(44100));
    let playback_freq = 44100u32;

    let fft = Fft {
        divider: args.fft_div * 2,
        min_freq: args.min_freq as f32,
        max_freq: args.max_freq as f32,
    };

    let msec_outside = msec.clone();
    let msec_skip = msec.clone();

    Ok(thread::spawn(move || {
        let main = move || -> Result<()> {
            let ring = StaticRb::<f32, 8192>::default();
            let (mut audio_sink, mut audio_faucet) = ring.split();

            let mut resampler = Resampler::new(buffer_size / 2)?;
            let mut plugin_freq = 32000u32;

            let stream = device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                    //info.timestamp().playback.duration_since(last_ts);
                    if audio_faucet.pop_slice(data) > 0 {
                        let ms = data.len() * 1000 / (playback_freq as usize * 2);
                        msec.fetch_add(ms, Ordering::SeqCst);
                    } else {
                        data.fill(0.0);
                    }
                },
                |err| eprintln!("An error occurred on stream: {err}"),
                None,
            )?;
            stream.play()?;

            let mut target: Vec<i16> = vec![0; buffer_size];

            let mut player = Player {
                millis: msec_outside,
                ..Player::default()
            };

            while player.play_state != PlayState::Quitting {
                while let Ok(cmd_fn) = cmd_consumer.try_recv() {
                    if let Err(e) = cmd_fn(&mut player) {
                        info_producer.push_value("error", e)?;
                    }
                }

                player.update_meta(&mut info_producer)?;

                if let Some(chip_player) = &mut player.chip_player {
                    if player.ff_msec > 0 {
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
                        let rc = chip_player.get_samples(&mut target);
                        if rc == 0 {
                            info_producer.push_value("done", 0)?;
                        }

                        let hz = chip_player.get_frequency();
                        if hz != plugin_freq {
                            log!("Plugin freq: {hz}");
                            plugin_freq = hz;
                            resampler.set_frequencies(plugin_freq, playback_freq)?;
                        }

                        let samples = target
                            .iter()
                            .take(rc)
                            .map(|&s16| f32::from(s16) / 32767.0)
                            .collect_vec();
                        let new_samples = resampler.process(&samples)?;
                        audio_sink.push_slice(new_samples);

                        if rc == target.len() {
                            let data = fft.run(&samples, playback_freq)?;
                            info_producer.push_value("fft", data)?;
                        }
                    } else {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                } else {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            info_producer.push_value("quit", 1)?;
            Ok(())
        };
        main().expect("Fail");
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
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec, data).unwrap();

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
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec, Path::new("data"))
                .unwrap();

        cmd_producer.send(Box::new(move |p| p.next_song())).unwrap();
        let (_, val) = info_consumer.recv().unwrap();
        assert!(matches!(val, Value::Error(_)));

        cmd_producer.send(Box::new(move |p| p.quit())).unwrap();
        let (key, _) = info_consumer.recv().unwrap();

        assert_eq!(key, "quit");
        player_thread.join().unwrap();
    }
}
