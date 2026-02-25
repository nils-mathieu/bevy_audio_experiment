pub trait Source: Iterator<Item = f32> {
    fn channels(&self) -> u32;
    fn sample_rate(&self) -> u32;
}
