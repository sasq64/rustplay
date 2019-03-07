use crate::audio_player::AudioPlayer;

extern crate coreaudio;
use coreaudio::audio_unit::render_callback::{self, data};
use coreaudio::audio_unit::{AudioUnit, IOType, SampleFormat};
use std::collections::VecDeque;

use std::sync::{Mutex, Arc, Condvar, MutexGuard};

struct Fifo<T> {
    deque : Arc<Mutex<VecDeque<T>>>,
    cv : Arc<Condvar>
}

impl<T> Fifo<T> {
    fn push_back(&mut self, v : T) {
        loop {
            let mut guard : MutexGuard<VecDeque<T>> = self.deque.lock().unwrap();
            if guard.len() > 64 * 1024 {
                guard = self.cv.wait(guard).expect("wait failed");
                if guard.len() > 64 * 1024 {
                    guard.push_back(v);
                    return;
                }
            } else {
                guard.push_back(v);
                return;
            }
        }
    }
    fn pop_front(&mut self) -> Option<T> {
       let result = self.deque.lock().unwrap().pop_front();
        self.cv.notify_one();
        result
    }

    fn new() -> Fifo<T> {
        Fifo { deque : Arc::new(Mutex::new(VecDeque::new())), cv : Arc::new(Condvar::new()) }
    }

    fn clone(&mut self) -> Fifo<T> {
        Fifo { deque : self.deque.clone(), cv : self.cv.clone() }
    }

    fn len(&mut self) -> usize {
        self.deque.lock().unwrap().len()
    }
}

//unsafe impl<T> Sync for Fifo<T> {}

pub struct MacPlayer {
    fifo: Fifo<i16>,
    #[allow(dead_code)]
    audio_unit: AudioUnit
}

//unsafe impl Send for MacPlayer {}

impl AudioPlayer for MacPlayer {
    fn new(hz: u32) -> MacPlayer {
        //let (samples_sender, samples_receiver) = channel::<Vec<i16>>();
        let mut audio_unit = AudioUnit::new(IOType::DefaultOutput).unwrap();
        audio_unit.set_sample_rate(hz as f64).expect("");
        let stream_format = audio_unit.output_stream_format().unwrap();
    	println!("{:#?}", &stream_format);	

	    // For this example, our sine wave expects `f32` data.
	    assert!(SampleFormat::F32 == stream_format.sample_format);

        let mut audio_fifo = Fifo::<i16>::new();
        type Args = render_callback::Args<data::NonInterleaved<f32>>;
        let fifo_clone = audio_fifo.clone();
        audio_unit.set_render_callback(move |args| {
            let Args { num_frames, mut data, .. } = args;
            println!("Need {} frames, have {} on fifo", num_frames, audio_fifo.len());
            for i in 0..num_frames {
                for channel in data.channels_mut() {
                    if let Some(sample) = audio_fifo.pop_front() {
                        channel[i] = (sample as f32) / 32768.0;
                    } else {
                        break;
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

    fn play(&mut self, _callback: fn(&mut [i16])) {}
}
