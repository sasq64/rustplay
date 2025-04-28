use anyhow::Result;
use itertools::Itertools;
use rubato::{SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

pub struct Resampler {
    resampler: SincFixedIn<f32>,
    wave_out: Vec<f32>,
    samples_out: Vec<f32>,
    source_hz: f64,
    target_hz: f64,
    buffer_size: usize,
}

impl Resampler {
    /// buffer_size is number of stereo samples to feed into it at each process
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
            source_hz: 44100.0,
            target_hz: 44100.0,
            buffer_size,
        })
    }

    pub fn set_frequencies(&mut self, source_hz: f64, target_hz: f64) -> Result<()> {
        use rubato::Resampler;
        self.source_hz = source_hz;
        self.target_hz = target_hz;
        self.resampler
            .set_resample_ratio(target_hz / source_hz, false)?;
        Ok(())
    }

    pub fn process<'a>(&'a mut self, samples: &'a [f32]) -> Result<&'a [f32]> {
        use rubato::Resampler;

        if self.source_hz != self.target_hz {
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
        resampler.set_frequencies(10.0, 20.0).unwrap();

        let result = resampler.process(&test_vec).unwrap();
        eprintln!("{:?}", &result[..20]);
    }
}
