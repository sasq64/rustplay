use std::thread;
use std::time::Duration;
use anyhow::{Context, Result};
use cpal::traits::*;

use super::audio_device::{AudioDevice, AudioCallback};

pub(crate) struct NoSoundDevice {}

pub(crate) struct CPalDevice {
    device: cpal::Device,
    config: cpal::StreamConfig,
    playback_freq: u32,
    buffer_size: usize,
    stream: Option<cpal::Stream>,
}

impl AudioDevice for NoSoundDevice {
    fn play(&mut self, mut callback: AudioCallback) -> Result<()> {
        let buffer_size = self.get_buffer_size();
        let playback_freq = self.get_playback_freq();

        // Calculate the sleep duration to simulate real-time audio playback
        let samples_per_call = buffer_size;
        let duration_per_call = Duration::from_millis(
            (samples_per_call * 1000) as u64 / (playback_freq * 2) as u64, // *2 for stereo
        );

        thread::spawn(move || {
            let mut buffer = vec![0.0f32; buffer_size];
            loop {
                callback(&mut buffer);
                thread::sleep(duration_per_call);
            }
        });

        Ok(())
    }

    fn get_buffer_size(&self) -> usize {
        1024
    }

    fn get_playback_freq(&self) -> u32 {
        44100
    }
}

impl AudioDevice for CPalDevice {
    fn play(&mut self, mut callback: AudioCallback) -> Result<()> {
        let stream = self.device.build_output_stream(
            &self.config,
            move |data: &mut [f32], _info: &cpal::OutputCallbackInfo| {
                callback(data);
            },
            |err| eprintln!("An error occurred on stream: {err}"),
            None,
        )?;
        stream.play()?;
        self.stream = Some(stream);
        Ok(())
    }

    fn get_buffer_size(&self) -> usize {
        self.buffer_size
    }

    fn get_playback_freq(&self) -> u32 {
        self.playback_freq
    }
}

const BUFFER_SIZE: usize = 4096 / 2;
pub(crate) const PLAYBACK_FREQ_HZ: u32 = 44100;

pub(crate) fn setup_audio_device() -> Result<Box<dyn AudioDevice>> {
    let device = cpal::default_host()
        .default_output_device()
        .context("No audio device available")?;

    let mut configs = device
        .supported_output_configs()
        .context("Could not get audio configs")?;

    let sconf = configs
        .find(|conf| {
            conf.channels() == 2
                && conf.sample_format() == cpal::SampleFormat::F32
                && conf.max_sample_rate() >= cpal::SampleRate(PLAYBACK_FREQ_HZ)
                && conf.min_sample_rate() <= cpal::SampleRate(PLAYBACK_FREQ_HZ)
        })
        .context("Could not find a compatible audio config")?;

    let config = sconf.with_sample_rate(cpal::SampleRate(PLAYBACK_FREQ_HZ));

    Ok(Box::new(CPalDevice {
        device,
        config: config.into(),
        playback_freq: PLAYBACK_FREQ_HZ,
        buffer_size: BUFFER_SIZE,
        stream: None,
    }))
}