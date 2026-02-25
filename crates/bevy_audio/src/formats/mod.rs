use {crate::Source, bevy_ecs::error::BevyError, std::io::Read};

#[cfg(feature = "wav")]
mod wav;
#[cfg(feature = "wav")]
pub use self::wav::*;

pub enum Decoder<R> {
    #[cfg(feature = "wav")]
    Wav(WavDecoder<R>),
}

impl<R: Read> Decoder<R> {
    pub fn new(reader: R) -> Result<Self, BevyError> {
        #[cfg(feature = "wav")]
        {
            match WavDecoder::new(reader) {
                Ok(decoder) => Ok(Decoder::Wav(decoder)),
                Err(err) => Err(err.into()),
            }
        }
    }
}

impl<R: Read> Iterator for Decoder<R> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            #[cfg(feature = "wav")]
            Decoder::Wav(decoder) => decoder.next(),
        }
    }
}

impl<R: Read> Source for Decoder<R> {
    fn channels(&self) -> u32 {
        match self {
            #[cfg(feature = "wav")]
            Decoder::Wav(decoder) => decoder.channels(),
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            #[cfg(feature = "wav")]
            Decoder::Wav(decoder) => decoder.sample_rate(),
        }
    }
}
