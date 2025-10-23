use spectrum_analyzer::{
    FrequencyLimit, samples_fft_to_spectrum, scaling::scale_20_times_log10, windows::hann_window,
};

use anyhow::Result;
use anyhow::anyhow;

pub(crate) struct Fft {
    pub divider: usize,
    pub min_freq: f32,
    pub max_freq: f32,
}

impl Fft {
    pub fn run(&self, samples: &[f32], freq: u32) -> Result<Vec<u8>> {
        let mix: Vec<_> = samples
            .chunks(self.divider)
            .map(|a| a.iter().sum())
            .collect();
        // Pad to next power of two
        let fft_size = mix.len().next_power_of_two();
        let mut padded = mix.clone();
        padded.resize(fft_size, 0.0);
        let window = hann_window(&padded);
        let spectrum = samples_fft_to_spectrum(
            &window,
            freq,
            FrequencyLimit::Range(self.min_freq, self.max_freq),
            Some(&scale_20_times_log10),
        )
        .map_err(|e| anyhow!("FFT error: {:?}", e))?;
        let data: Vec<u8> = spectrum
            .data()
            .iter()
            .map(|(_, j)| (j.val() * 0.75) as u8)
            .collect();
        Ok(data)
    }
}
