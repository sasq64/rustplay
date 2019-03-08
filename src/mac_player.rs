extern crate coreaudio;
use coreaudio::audio_unit::render_callback::{self, data};
use coreaudio::audio_unit::{AudioUnit, IOType, SampleFormat};

use crate::fifo::Fifo;
use crate::audio_player::AudioPlayer;

pub struct MacPlayer {
    fifo: Fifo<i16>,
    #[allow(dead_code)]
    audio_unit: AudioUnit
}

impl AudioPlayer for MacPlayer {
    fn new(hz: u32) -> MacPlayer {
        let mut audio_unit : AudioUnit = AudioUnit::new(IOType::DefaultOutput).unwrap();
        audio_unit.set_sample_rate(hz as f64).expect("");

        let stream_format = audio_unit.output_stream_format().unwrap();
        println!("{:#?}", &stream_format);  

        assert!(SampleFormat::F32 == stream_format.sample_format);

        let mut audio_fifo = Fifo::<i16>::new();
        type Args = render_callback::Args<data::NonInterleaved<f32>>;
        let fifo_clone = audio_fifo.clone();
        audio_unit.set_render_callback(move |args| {
            let Args { num_frames, mut data, .. } = args;
            for i in 0..num_frames {
                for channel in data.channels_mut() {
                    if let Some(sample) = audio_fifo.pop_front() {
                        channel[i] = (sample as f32) / 32768.0;
                    } else {
                        channel[i] = 0.0;
                    }
                }
            }

            Ok(())
        }).expect("Could not set render_callback");
        audio_unit.start().expect("Cant start audio");

        MacPlayer { fifo : fifo_clone, audio_unit }
    }

    fn write(&mut self, samples: &[i16]) {
        for s in samples.iter() {
            self.fifo.push_back(*s);
        }
    }

}
