use core::fmt;
use std::{
    error::Error,
    fmt::Display,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use cpal::{traits::*, SampleFormat};

use id3::{Tag, TagLike};
use ringbuf::{traits::*, StaticRb};

use spectrum_analyzer::{samples_fft_to_spectrum, scaling::*, windows::*, FrequencyLimit};

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
            Value::Text(s) => f.write_str(s.as_str()).unwrap(),
            Value::Number(n) => write!(f, "{:02}", n).unwrap(),
            Value::Error(e) => write!(f, "{}", e).unwrap(),
            Value::Data(_) => (),
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

impl From<MusicError> for Value {
    fn from(item: MusicError) -> Self {
        Value::Error(item)
    }
}

pub(crate) type PlayResult = Result<bool, MusicError>;

pub(crate) type Cmd = Box<dyn FnOnce(&mut Player) -> PlayResult + Send>;
pub(crate) type Info = (String, Value);

fn parse_mp3<R: Read>(reader: &mut R) -> io::Result<bool> {
    let mut header: [u8; 32] = [0; 32];
    reader.read_exact(&mut header)?;
    Ok(true)
}

#[derive(Default)]
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

    pub fn next_song(&mut self) -> PlayResult {
        if self.song < (self.songs - 1) {
            if let Some(cp) = &self.chip_player {
                cp.seek(self.song + 1, 0);
                self.reset();
            }
        }
        Ok(true)
    }
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

    pub fn quit(&mut self) -> PlayResult {
        self.quitting = true;
        Ok(true)
    }
}

fn push<IP: Producer<Item = Info>, V: Into<Value>>(ip: &mut IP, name: &str, val: V) -> PlayResult {
    ip.try_push((name.to_owned(), val.into()))
        .map_err(|_| MusicError {
            msg: "Could not push".to_owned(),
        })?;
    Ok(true)
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
) -> JoinHandle<()>
where
    P: Producer<Item = Info> + Send + 'static,
    C: Consumer<Item = Cmd> + Send + 'static,
{
    musix::init(Path::new("data")).unwrap();

    let device = cpal::default_host().default_output_device().unwrap();
    let mut configs = device.supported_output_configs().unwrap();
    let buffer_size = 4096 / 2;

    let sconf = configs
        .find(|conf| conf.channels() == 2 && conf.sample_format() == SampleFormat::F32)
        .expect("Could not find a compatible audio config");
    let config = sconf.with_sample_rate(cpal::SampleRate(44100));

    let min_freq = args.min_freq as f32;
    let max_freq = args.max_freq as f32;
    let fft_div = args.fft_div * 2;

    let msec_outside = msec.clone();

    thread::spawn(move || {
        let main = move || -> Result<(), Box<dyn Error>> {
            let ring = StaticRb::<f32, 8192>::default();
            let (mut producer, mut consumer) = ring.split();

            let stream = device.build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    consumer.pop_slice(data);
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
            let mut temp: Vec<f32> = vec![0.0; buffer_size];

            loop {
                if player.quitting {
                    break;
                }
                while let Some(f) = cmd_consumer.try_pop() {
                    if let Err(e) = f(&mut player) {
                        push(&mut info_producer, "error", e)?;
                    }
                }

                if let Some(cp) = &mut player.chip_player {
                    if let Some(new_song) = player.new_song.take() {
                        //println!("New song {}", new_song.to_str().unwrap());
                        push(&mut info_producer, "new", 0)?;
                        if let Some(ext) = new_song.extension() {
                            if ext == "mp3" {
                                if let Ok(duration) = mp3_duration::from_path(&new_song) {
                                    let secs = duration.as_secs() as i32;
                                    push(&mut info_producer, "length", secs)?;
                                }
                                //println!("Found mp3");
                                let tag = Tag::read_from_path(new_song)?;
                                //println!("Found tag");
                                if let Some(album) = tag.album() {
                                    push(&mut info_producer, "album", album)?;
                                }
                                if let Some(artist) = tag.artist() {
                                    push(&mut info_producer, "composer", artist)?;
                                }
                                if let Some(title) = tag.title() {
                                    push(&mut info_producer, "title", title)?;
                                }
                            }
                        }
                    }

                    while let Some(meta) = cp.get_changed_meta() {
                        let val = cp.get_meta_string(&meta).unwrap();
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
                        push(&mut info_producer, &meta, v)?;
                    }
                    if producer.vacant_len() > target.len() {
                        let rc = cp.get_samples(&mut target);
                        if rc == 0 {
                            push(&mut info_producer, "done", 0)?;
                        }
                        for i in 0..target.len() {
                            temp[i] = (target[i] as f32) / 32767.0;
                        }
                        let mix: Vec<f32> = temp.chunks(fft_div).map(|a| a.iter().sum()).collect();
                        producer.push_slice(&temp);
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
                        let data = spectrum
                            .data()
                            .iter()
                            .map(|(_, j)| (j.val() * 0.75) as u8)
                            .collect();
                        push(&mut info_producer, "fft", Value::Data(data))?;
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
            Ok(())
        };
        main().unwrap();
    })
}
