use core::fmt;
use std::{
    error::Error,
    fmt::Display,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
};

use cpal::{SampleFormat, SampleRate, traits::*};

use id3::{Tag, TagLike};
use ringbuf::{StaticRb, traits::*};

use spectrum_analyzer::{
    FrequencyLimit, samples_fft_to_spectrum, scaling::scale_20_times_log10, windows::hann_window,
};

use musix::{ChipPlayer, MusicError};

use crate::Args;

pub(crate) enum Value {
    Text(String),
    Number(i32),
    Data(Vec<u8>),
    Error(MusicError),
}

impl Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Value::Text(s) => f.write_str(s.as_str())?,
            Value::Number(n) => write!(f, "{n:02}")?,
            Value::Error(e) => write!(f, "{e}")?,
            Value::Data(_) => write!(f, "Data")?,
        }
        Ok(())
    }
}

impl From<i32> for Value {
    fn from(item: i32) -> Self {
        Value::Number(item)
    }
}

impl From<String> for Value {
    fn from(item: String) -> Self {
        Value::Text(item)
    }
}

impl From<&str> for Value {
    fn from(item: &str) -> Self {
        Value::Text(item.to_owned())
    }
}

impl From<Vec<u8>> for Value {
    fn from(item: Vec<u8>) -> Self {
        Value::Data(item)
    }
}

impl From<MusicError> for Value {
    fn from(item: MusicError) -> Self {
        Value::Error(item)
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

#[derive(Default)]
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
        if self.song < (self.songs - 1) {
            if let Some(cp) = &self.chip_player {
                cp.seek(self.song + 1, 0);
                self.reset();
            }
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
}

trait PushValue {
    fn push_value<V: Into<Value>>(&mut self, name: &str, val: V) -> PlayResult;
}

impl<T> PushValue for T
where
    T: Producer<Item = Info>,
{
    fn push_value<V: Into<Value>>(&mut self, name: &str, val: V) -> PlayResult
    where
        Self: Producer<Item = Info>,
    {
        self.try_push((name.to_owned(), val.into()))
            .map_err(|_| MusicError {
                msg: "Could not push".to_owned(),
            })?;
        Ok(true)
    }
}

pub(crate) fn run_player<P, C>(
    args: &Args,
    mut info_producer: P,
    mut cmd_consumer: C,
    msec: Arc<AtomicUsize>,
) -> Result<JoinHandle<()>, MusicError>
where
    P: Producer<Item = Info> + Send + 'static,
    C: Consumer<Item = Cmd> + Send + 'static,
{
    musix::init(Path::new("data"))?;

    let device = cpal::default_host()
        .default_output_device()
        .ok_or_else(|| MusicError {
            msg: "No audio device available".into(),
        })?;

    let mut configs = device.supported_output_configs().map_err(|e| MusicError {
        msg: format!("Could not get audio configs: {e}"),
    })?;
    let buffer_size = 4096 / 2;

    let sconf = configs
        .find(|conf| {
            conf.channels() == 2
                && conf.sample_format() == SampleFormat::F32
                && conf.max_sample_rate() >= cpal::SampleRate(44100)
                && conf.min_sample_rate() <= cpal::SampleRate(44100)
        })
        .ok_or_else(|| MusicError {
            msg: "Could not find a compatible audio config".into(),
        })?;
    let config = sconf.with_sample_rate(SampleRate(44100));

    let min_freq = args.min_freq as f32;
    let max_freq = args.max_freq as f32;
    let fft_div = args.fft_div * 2;

    let msec_outside = msec.clone();

    Ok(thread::spawn(move || {
        let main = move || -> Result<(), Box<dyn Error>> {
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
                ..Default::default()
            };

            info_producer.push_value("done", 0)?;

            loop {
                if player.quitting {
                    break;
                }
                while let Some(f) = cmd_consumer.try_pop() {
                    if let Err(e) = f(&mut player) {
                        info_producer.push_value("error", e)?;
                    }
                }

                if let Some(cp) = &mut player.chip_player {
                    if let Some(new_song) = player.new_song.take() {
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

                    while let Some(meta) = cp.get_changed_meta() {
                        let val = cp.get_meta_string(&meta).unwrap_or("".into());
                        let v: Value = match meta.as_str() {
                            "song" | "startSong" => {
                                player.song = val.parse::<i32>()?;
                                player.song.into()
                            }
                            "songs" => {
                                let n = val.parse::<i32>()?;
                                player.songs = n;
                                n.into()
                            }
                            "length" => {
                                let length = val.parse::<i32>()?;
                                length.into()
                            }
                            &_ => Value::Text(val),
                        };
                        info_producer.push_value(&meta, v)?;
                    }
                    if audio_sink.vacant_len() > target.len() {
                        let rc = cp.get_samples(&mut target);
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
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            Ok(())
        };
        main().expect("");
    }))
}
#[cfg(test)]
mod tests {
    use core::time;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::thread;

    use ringbuf::{HeapRb, StaticRb, traits::*};

    use crate::Args;

    use super::Cmd;
    use super::Info;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn player_starts() {
        // Send commands to player
        let (mut cmd_producer, cmd_consumer) = HeapRb::<Cmd>::new(5).split();

        // Receive info from player
        let (info_producer, _) = StaticRb::<Info, 64>::default().split();
        let msec = Arc::new(AtomicUsize::new(0));
        let args = Args {
            ..Default::default()
        };
        let player_thread =
            crate::player::run_player(&args, info_producer, cmd_consumer, msec).unwrap();

        cmd_producer
            .try_push(Box::new(move |p| p.quit()))
            .unwrap_or_else(|_| panic!("Could not push to cmd_producer"));
        thread::sleep(time::Duration::from_millis(500));
        if player_thread.is_finished() {
            player_thread.join().unwrap();
        } else {
            panic!("Thread did not quit");
        }
    }
}
