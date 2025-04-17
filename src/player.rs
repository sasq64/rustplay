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

use id3::{Tag, TagLike};
use ringbuf::{StaticRb, traits::*};

use spectrum_analyzer::{
    FrequencyLimit, samples_fft_to_spectrum, scaling::scale_20_times_log10, windows::hann_window,
};

use crate::{Args, value::Value};
use anyhow::Context;
use anyhow::Result;
use musix::{ChipPlayer, MusicError};

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

#[derive(Default, Debug)]
#[allow(clippy::struct_field_names)]
pub(crate) struct Player {
    chip_player: Option<ChipPlayer>,
    song: i32,
    songs: i32,
    millis: Arc<AtomicUsize>,
    playing: bool,
    quitting: bool,
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
        self.playing = true;
        Ok(true)
    }

    #[allow(clippy::unnecessary_wraps)]
    pub fn quit(&mut self) -> PlayResult {
        self.quitting = true;
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
                    let tag = Tag::read_from_path(new_song)?;
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
                let val = chip_player.get_meta_string(&meta).unwrap_or("".into());
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

pub(crate) fn run_player(
    args: &Args,
    mut info_producer: mpsc::Sender<Info>,
    cmd_consumer: mpsc::Receiver<Cmd>,
    msec: Arc<AtomicUsize>,
) -> Result<JoinHandle<()>> {
    musix::init(Path::new("data"))?;

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

    let min_freq = args.min_freq as f32;
    let max_freq = args.max_freq as f32;
    let fft_div = args.fft_div * 2;

    let msec_outside = msec.clone();

    Ok(thread::spawn(move || {
        let main = move || -> Result<()> {
            let ring = StaticRb::<f32, 8192>::default();
            let (mut audio_sink, mut audio_faucet) = ring.split();

            let stream = device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    audio_faucet.pop_slice(data);
                    let ms = data.len() * 1000 / (44100 * 2);
                    msec.fetch_add(ms, Ordering::SeqCst);
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

            // info_producer.push_value("done", 0)?;

            while !player.quitting {
                while let Ok(cmd_fn) = cmd_consumer.try_recv() {
                    if let Err(e) = cmd_fn(&mut player) {
                        info_producer.push_value("error", e)?;
                    }
                }

                player.update_meta(&mut info_producer)?;

                if let Some(chip_player) = &mut player.chip_player {
                    if audio_sink.vacant_len() > target.len() {
                        let rc = chip_player.get_samples(&mut target);
                        if rc == 0 {
                            info_producer.push_value("done", 0)?;
                        }
                        let samples: Vec<f32> =
                            target.iter().map(|s16| (*s16 as f32) / 32767.0).collect();
                        let mix: Vec<f32> =
                            samples.chunks(fft_div).map(|a| a.iter().sum()).collect();
                        audio_sink.push_slice(&samples);
                        let window = hann_window(&mix);
                        let spectrum = samples_fft_to_spectrum(
                            &window,
                            //&temp,
                            44100,
                            FrequencyLimit::Range(min_freq, max_freq),
                            Some(&scale_20_times_log10), //divide_by_N_sqrt),
                        )
                        .map_err(|_| MusicError {
                            msg: "FFT Error".into(),
                        })?;
                        let data: Vec<u8> = spectrum
                            .data()
                            .iter()
                            .map(|(_, j)| (j.val() * 0.75) as u8)
                            .collect();
                        info_producer.push_value("fft", data)?;
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
mod tests {
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::mpsc;

    use crate::Args;
    use crate::value::Value;

    use super::Cmd;
    use super::Info;

    #[test]
    #[allow(clippy::unwrap_used)]
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
    #[allow(clippy::unwrap_used)]
    fn player_starts() {
        // Send commands to player
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));
        let args = Args { ..Args::default() };
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec).unwrap();

        cmd_producer.send(Box::new(move |p| p.quit())).unwrap();
        let (key, _) = info_consumer.recv().unwrap();
        assert_eq!(key, "quit");
        player_thread.join().unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn player_can_report_errors() {
        let (cmd_producer, cmd_consumer) = mpsc::channel::<Cmd>();

        // Receive info from player
        let (info_producer, info_consumer) = mpsc::channel::<Info>();
        let msec = Arc::new(AtomicUsize::new(0));
        let args = Args { ..Args::default() };
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec).unwrap();

        cmd_producer.send(Box::new(move |p| p.next_song())).unwrap();
        let (_, val) = info_consumer.recv().unwrap();
        assert!(matches!(val, Value::Error(_)));

        let path = PathBuf::from("loz15.miniusf");
        cmd_producer.send(Box::new(move |p| p.load(&path))).unwrap();
        

        cmd_producer.send(Box::new(move |p| p.quit())).unwrap();
        let (key, _) = info_consumer.recv().unwrap();

        assert_eq!(key, "quit");
        player_thread.join().unwrap();
    }
}
