use anyhow::Result;

pub(crate) type AudioCallback = Box<dyn FnMut(&mut [f32]) + Send>;

pub(crate) trait AudioDevice {
    fn play(&mut self, callback: AudioCallback) -> Result<()>;
    fn get_buffer_size(&self) -> usize;
    fn get_playback_freq(&self) -> u32;
}