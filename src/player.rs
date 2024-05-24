use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use cpal::traits::*;

use ringbuf::{traits::*, StaticRb};

use spectrum_analyzer::{samples_fft_to_spectrum, scaling::*, windows::*, FrequencyLimit};

use musix::{ChipPlayer, MusicError};

pub(crate) enum Value {
    Text(String),
    Number(i32),
    Data(Vec<u8>),
}

type PlayResult = Result<bool, MusicError>;

pub(crate) type Cmd = Box<dyn FnOnce(&mut Player) -> PlayResult + Send>;
pub(crate) type Info = (String, Value);

pub(crate) struct Player {
    chip_player: ChipPlayer,
    song: i32,
    millis: Arc<AtomicUsize>,
    song_queue: VecDeque<PathBuf>,
    playing: bool,
    quitting: bool,
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
        self.chip_player.seek(self.song + 1, 0);
        self.reset();
        Ok(true)
    }
    pub fn prev_song(&mut self) -> PlayResult {
        self.chip_player.seek(self.song - 1, 0);
        self.reset();
        Ok(true)
    }

    pub fn load(&mut self, name: &Path) -> PlayResult {
        self.chip_player = ChipPlayer::new();
        self.chip_player = musix::load_song(name)?;
        self.reset();
        self.playing = true;
        Ok(true)
    }

    pub fn quit(&mut self) -> PlayResult {
        self.quitting = true;
        Ok(true)
    }
}

pub(crate) fn run_player<P, C>(
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
    let mut config = device.default_output_config().unwrap();
    let configs = device.supported_output_configs().unwrap();
    let buffer_size = 4096;
    for conf in configs {
        if let Some(conf2) = conf.try_with_sample_rate(cpal::SampleRate(44100)) {
            config = conf2;
            break;
        }
    }

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
            chip_player: musix::ChipPlayer::new(),
            song: 0,
            millis: msec_outside,
            song_queue: VecDeque::new(),
            playing: false,
            quitting: false,
        };

        let mut temp: Vec<f32> = vec![0.0; buffer_size];
        loop {
            if player.quitting {
                break;
            }
            while let Some(f) = cmd_consumer.try_pop() {
                let _ = f(&mut player);
            }

            if !player.playing {
                let _ = player.next();
            }

            while let Some(meta) = player.chip_player.get_changed_meta() {
                let val = player.chip_player.get_meta_string(&meta).unwrap();
                let v: Value = match meta.as_str() {
                    "song" | "startSong" => {
                        player.song = val.parse::<i32>().unwrap();
                        Value::Number(player.song)
                    }
                    "songs" | "length" => {
                        let l = val.parse::<i32>().unwrap();
                        Value::Number(l)
                    }
                    &_ => Value::Text(val),
                };
                let _ = info_producer.try_push((meta, v));
            }
            if producer.vacant_len() > target.len() {
                player.chip_player.get_samples(&mut target);
                for i in 0..target.len() {
                    temp[i] = (target[i] as f32) / 32767.0;
                }
                producer.push_slice(&temp);
                let mix: Vec<f32> = temp.chunks(2).map(|a| a[0] + a[1]).collect();
                let window = hann_window(&mix);
                let spectrum = samples_fft_to_spectrum(
                    &window,
                    //&temp,
                    44100,
                    FrequencyLimit::Range(15.0, 1500.0),
                    Some(&scale_20_times_log10), //divide_by_N_sqrt),
                )
                .unwrap();
                let data = spectrum
                    .data()
                    .iter()
                    .map(|(_, j)| (j.val() * 0.75) as u8)
                    .collect();
                let _ = info_producer.try_push(("fft".to_owned(), Value::Data(data)));
            } else {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    })
}
