use anyhow::Result;
use itertools::Itertools;
use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

#[allow(clippy::struct_field_names)]
pub struct Resampler {
    resampler: SincFixedIn<f32>,
    wave_out: Vec<f32>,
    samples_out: Vec<f32>,
    buffer_size: usize,
    enabled: bool,
}

impl Resampler {
    /// `buffer_size` is number of stereo samples to feed into it at each process
    pub fn new(buffer_size: usize) -> Result<Resampler> {
        let params = SincInterpolationParameters {
            sinc_len: 256,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Linear,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let resampler = SincFixedIn::<f32>::new(1.0, 4.0, params, buffer_size, 2)?;
        let wave_out: Vec<f32> = vec![0.0; buffer_size * 6];
        let samples_out: Vec<f32> = vec![0.0; buffer_size * 6];
        Ok(Resampler {
            resampler,
            wave_out,
            samples_out,
            buffer_size,
            enabled: false,
        })
    }

    pub fn set_frequencies(&mut self, source_hz: u32, target_hz: u32) -> Result<()> {
        use rubato::Resampler;
        self.enabled = source_hz != target_hz;
        let ratio = f64::from(target_hz) / f64::from(source_hz);
        self.resampler.set_resample_ratio(ratio, false)?;
        Ok(())
    }

    pub fn process<'a>(&'a mut self, samples: &'a [f32]) -> Result<&'a [f32]> {
        use rubato::Resampler;

        if self.enabled {
            let left = samples.iter().copied().step_by(2).collect_vec();
            let right = samples.iter().copied().skip(1).step_by(2).collect_vec();
            let input = vec![left, right];
            let (out_left, out_right) = self.wave_out.split_at_mut(self.buffer_size * 3);
            let mut output = vec![out_left, out_right];
            let (_rcount, wcount) =
                self.resampler
                    .process_into_buffer(&input, &mut output, None)?;
            let (left, right) = self.wave_out.split_at(self.buffer_size * 3);
            self.samples_out.resize(wcount * 2, 0.0);
            for (i, (&l, &r)) in left.iter().zip(right.iter()).take(wcount).enumerate() {
                self.samples_out[i * 2] = l;
                self.samples_out[i * 2 + 1] = r;
            }
            return Ok(&self.samples_out);
        }
        Ok(samples)
    }
}

mod test {

    #[test]
    fn test_resample() {
        use super::Resampler;
        use itertools::Itertools;

        let n = 10000;
        let mut resampler = Resampler::new(n).unwrap();
        let floats = (0..n).map(|i| (i * 2) as f32);
        let floats2 = floats.clone().collect_vec().into_iter();
        let test_vec = floats.zip(floats2).flat_map(|(l, r)| [l, r]).collect_vec();
        eprintln!("{:?}", &test_vec[..20]);
        resampler.set_frequencies(10, 20).unwrap();

        let result = resampler.process(&test_vec).unwrap();
        eprintln!("{:?}", &result[..20]);
    }
}
