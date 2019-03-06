pub trait AudioPlayer {
    fn new(hz: u32) -> Self;
    fn write(&mut self, samples: &[i16]);
    fn play(&mut self, callback: fn(&mut [i16]));
}
