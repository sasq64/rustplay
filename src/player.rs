use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use cpal::{traits::*, SampleFormat};

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

impl From<MusicError> for Value {
    fn from(item: MusicError) -> Self {
        Value::Error(item)
    }
}

pub(crate) type PlayResult = Result<bool, MusicError>;

pub(crate) type Cmd = Box<dyn FnOnce(&mut Player) -> PlayResult + Send>;
pub(crate) type Info = (String, Value);

pub(crate) struct Player {
    chip_player: Option<ChipPlayer>,
    song: i32,
    millis: Arc<AtomicUsize>,
    song_queue: VecDeque<PathBuf>,
    playing: bool,
    quitting: bool,
    new_song: bool,
}

impl Player {
    pub fn reset(&mut self) {
        self.millis.store(0, Ordering::SeqCst);
    }

    pub fn next(&mut self) -> PlayResult {
        if let Some(song_file) = self.song_queue.pop_front() {
            self.load(&song_file)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub fn add_song(&mut self, path: &Path) -> PlayResult {
        self.song_queue.push_back(path.to_owned());
        Ok(true)
    }

    pub fn next_song(&mut self) -> PlayResult {
        if let Some(cp) = &self.chip_player {
            cp.seek(self.song + 1, 0);
            self.reset();
        }
        Ok(true)
    }
    pub fn prev_song(&mut self) -> PlayResult {
        if let Some(cp) = &self.chip_player {
            cp.seek(self.song - 1, 0);
            self.reset();
        }
        Ok(true)
    }

    pub fn load(&mut self, name: &Path) -> PlayResult {
        self.chip_player = None;
        self.chip_player = Some(musix::load_song(name)?);
        self.reset();
        self.new_song = true;
        self.playing = true;
        Ok(true)
    }

    pub fn quit(&mut self) -> PlayResult {
        self.quitting = true;
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
        let ring = StaticRb::<f32, 8192>::default();
        let (mut producer, mut consumer) = ring.split();

        let stream = device
            .build_output_stream(
                &config.into(),
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    consumer.pop_slice(data);
                    let ms = data.len() * 1000 / (44100 * 2);
                    msec.fetch_add(ms, Ordering::SeqCst);
                },
                |err| eprintln!("An error occurred on stream: {err}"),
                None,
            )
            .unwrap();
        stream.play().unwrap();

        let mut target: Vec<i16> = vec![0; buffer_size];

        let mut player = Player {
            chip_player: None,
            song: 0,
            millis: msec_outside,
            song_queue: VecDeque::new(),
            playing: false,
            quitting: false,
            new_song: true,
        };

        let _ = info_producer.try_push(("done".into(), 0.into()));
        let mut temp: Vec<f32> = vec![0.0; buffer_size];

        loop {
            if player.quitting {
                break;
            }
            while let Some(f) = cmd_consumer.try_pop() {
                if let Err(e) = f(&mut player) {
                    let _ = info_producer.try_push(("error".into(), e.into()));
                }
            }

            if !player.playing {
                let _ = player.next();
            }

            if let Some(cp) = &mut player.chip_player {
                if player.new_song {
                    player.new_song = false;
                    let _ = info_producer.try_push(("new".into(), 0.into()));
                }

                while let Some(meta) = cp.get_changed_meta() {
                    let val = cp.get_meta_string(&meta).unwrap();
                    let v: Value = match meta.as_str() {
                        "song" | "startSong" => {
                            player.song = val.parse::<i32>().unwrap();
                            player.song.into()
                        }
                        "songs" | "length" => {
                            let length = val.parse::<i32>().unwrap();
                            length.into()
                        }
                        &_ => Value::Text(val),
                    };
                    let _ = info_producer.try_push((meta, v));
                }
                if producer.vacant_len() > target.len() {
                    cp.get_samples(&mut target);
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
                    .unwrap();
                    let data = spectrum
                        .data()
                        .iter()
                        .map(|(_, j)| (j.val() * 0.75) as u8)
                        .collect();
                    let _ = info_producer.try_push(("fft".into(), Value::Data(data)));
                } else {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    })
}
