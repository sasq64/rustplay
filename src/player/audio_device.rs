use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use anyhow::Result;

pub(crate) type AudioCallback = Box<dyn FnMut(&mut [f32]) + Send>;

pub(crate) trait AudioDevice {
    fn play(
        &mut self,
        callback: AudioCallback,
        device_latency_us: Arc<AtomicUsize>,
    ) -> Result<()>;
    fn get_buffer_size(&self) -> usize;
    fn get_playback_freq(&self) -> u32;
}