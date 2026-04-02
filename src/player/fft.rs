use spectrum_analyzer::FrequencySpectrum;
use spectrum_analyzer::{
    FrequencyLimit, samples_fft_to_spectrum, scaling::scale_20_times_log10,
};

use anyhow::Result;
use anyhow::anyhow;


#[derive(Default)]
pub(crate) struct Fft {
    pub divider: usize, // 4
    pub min_freq: f32,  // 15
    pub max_freq: f32,  // 4000
    pub bucket_bins: Vec<Vec<usize>>,
    pub peak: f32,
}

fn a_weight(freq: f32) -> f32 {
    let f2 = freq * freq;
    let f4 = f2 * f2;
    (12194.0f32.powi(2) * f4)
        / ((f2 + 20.6f32.powi(2))
            * ((f2 + 107.7f32.powi(2)) * (f2 + 737.9f32.powi(2))).sqrt()
            * (f2 + 12194.0f32.powi(2)))
}

impl Fft {
    fn setup(&mut self, spectrum: &FrequencySpectrum, num_buckets: usize) {
        let data = spectrum.data();
        let min_freq = spectrum.min_fr().val();
        let max_freq = spectrum.max_fr().val();

        let log_min = min_freq.log10();
        let log_max = max_freq.log10();
        let log_step = (log_max - log_min) / num_buckets as f32;

        self.bucket_bins = (0..num_buckets)
            .map(|i| {
                let low = 10f32.powf(log_min + i as f32 * log_step);
                let high = 10f32.powf(log_min + (i + 1) as f32 * log_step);
                let center = (low * high).sqrt();

                let bins: Vec<usize> = data
                    .iter()
                    .enumerate()
                    .filter(|(_, (freq, _))| {
                        let f = freq.val();
                        f >= low && f < high
                    })
                    .map(|(idx, _)| idx)
                    .collect();

                if bins.is_empty() {
                    let nearest = data
                        .iter()
                        .enumerate()
                        .min_by(|(_, (a, _)), (_, (b, _))| {
                            (a.val() - center)
                                .abs()
                                .partial_cmp(&(b.val() - center).abs())
                                .unwrap()
                        })
                        .map(|(idx, _)| idx)
                        .unwrap_or(0);
                    vec![nearest]
                } else {
                    bins
                }
            })
            .collect();
    }

    pub fn apply(&mut self, spectrum: &FrequencySpectrum) -> Vec<f32> {
        let data = spectrum.data();

        let mut values: Vec<f32> = self
            .bucket_bins
            .iter()
            .map(|bins| {
                let center_freq = {
                    let data = spectrum.data();
                    let mid = bins[bins.len() / 2];
                    data[mid].0.val()
                };
                let sum: f32 = bins.iter().map(|&i| data[i].1.val()).sum();
                let avg = sum / bins.len() as f32;
                let weight = a_weight(center_freq) / a_weight(1000.0);
                avg * weight
            })
            .collect();

        let frame_max = values.iter().cloned().fold(0.0f32, f32::max);
        self.peak = (self.peak * 0.95).max(frame_max); // 0.95 = decay rate, tune this

        if self.peak > 0.0 {
            for v in values.iter_mut() {
                *v /= self.peak;
            }
        }
        values
    }

    pub fn run(&mut self, samples: &[f32], freq: u32) -> Result<Vec<u8>> {
        let mix: Vec<_> = samples
            .chunks(self.divider)
            .map(|a| a.iter().sum())
            .collect();
        // Pad to next power of two
        let fft_size = mix.len().next_power_of_two();
        let mut padded = mix.clone();
        padded.resize(fft_size, 0.0);
        //let window = hann_window(&padded);
        let spectrum = samples_fft_to_spectrum(
            &padded,
            freq,
            FrequencyLimit::Range(self.min_freq, self.max_freq),
            Some(&scale_20_times_log10),
        )
        .map_err(|e| anyhow!("FFT error: {:?}", e))?;

        if self.bucket_bins.is_empty() {
            self.setup(&spectrum, 20);
        }

        let buckets = self.apply(&spectrum);

        let data: Vec<u8> = buckets.iter().map(|j| ((j + 0.0) * 30.0) as u8).collect();

        // let data: Vec<u8> = spectrum
        //     .data()
        //     .iter()
        //     .map(|(_, j)| (j.val() * 0.75) as u8)
        //     .collect();
        Ok(data)
    }
}
