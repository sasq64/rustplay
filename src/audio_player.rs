pub trait AudioPlayer {
    fn new(hz: u32) -> Self;
    fn write(&mut self, samples: &[i16]);
}
