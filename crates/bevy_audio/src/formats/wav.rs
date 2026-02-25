use {crate::Source, dasp_sample::ToSample, hound::WavReader, std::io::Read};

pub struct WavDecoder<R> {
    reader: WavReader<R>,

    // `reader` unfortunately does not provide access to its `samples_read` field so we have
    // to keep track of it ourselves.
    samples_read: u32,
}

impl<R: Read> WavDecoder<R> {
    pub fn new(reader: R) -> hound::Result<Self> {
        let reader = WavReader::new(reader)?;
        let samples_read = 0;

        Ok(Self {
            reader,
            samples_read,
        })
    }
}

impl<R: Read> Iterator for WavDecoder<R> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        self.samples_read += 1;

        // FIXME: How do we handle errors?

        fn next_sample<R: Read, T: hound::Sample + ToSample<f32>>(
            reader: &mut WavReader<R>,
        ) -> Option<f32> {
            reader
                .samples::<T>()
                .next()
                .map(|result| result.map(|sample| sample.to_sample_()).unwrap_or_default())
        }

        match (
            self.reader.spec().bits_per_sample,
            self.reader.spec().sample_format,
        ) {
            (32, hound::SampleFormat::Float) => next_sample::<R, f32>(&mut self.reader),
            (8, hound::SampleFormat::Int) => next_sample::<R, i8>(&mut self.reader),
            (16, hound::SampleFormat::Int) => next_sample::<R, i16>(&mut self.reader),
            (32, hound::SampleFormat::Int) => next_sample::<R, i32>(&mut self.reader),
            _ => None,
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.len();
        (len, Some(len))
    }
}

impl<R: Read> ExactSizeIterator for WavDecoder<R> {
    fn len(&self) -> usize {
        (self.reader.len() - self.samples_read) as usize
    }
}

impl<R: Read> Source for WavDecoder<R> {
    #[inline]
    fn channels(&self) -> u32 {
        self.reader.spec().channels as u32
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.reader.spec().sample_rate
    }
}
