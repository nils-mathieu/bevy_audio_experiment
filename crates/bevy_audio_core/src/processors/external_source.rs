use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo,
        RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
    bevy_platform::cell::SyncCell,
};

pub fn external_source<const C: usize>(
    buffer_size: usize,
) -> (ExternalSource<C>, ExternalSourceHandle<C>) {
    let (producer, consumer) = rtrb::RingBuffer::new(buffer_size);

    (
        ExternalSource(consumer),
        ExternalSourceHandle(SyncCell::new(producer)),
    )
}

pub struct ExternalSourceHandle<const C: usize>(SyncCell<rtrb::Producer<[f32; C]>>);

impl<const C: usize> ExternalSourceHandle<C> {
    /// # Returns
    ///
    /// Returns whether the frame was successfully sent.
    pub fn feed(&mut self, frame: [f32; C]) -> bool {
        self.0.get().push(frame).is_ok()
    }

    /// # Returns
    ///
    /// Returns the number of items sent.
    pub fn feed_iter(&mut self, samples: impl IntoIterator<Item = [f32; C]>) -> usize {
        let producer = self.0.get();
        producer
            .write_chunk_uninit(producer.slots())
            .unwrap()
            .fill_from_iter(samples)
    }
}

pub struct ExternalSource<const C: usize>(rtrb::Consumer<[f32; C]>);

impl<const C: usize> Processor for ExternalSource<C>
where
    AudioBuf<f32, C>: PortType,
{
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: vec![PortDescription {
                name: Some(Cow::Borrowed("output")),
                direction: PortDirection::Output,
                type_id: <AudioBuf<f32, C>>::PORT_TYPE_ID,
            }],
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        // SAFETY: The caller must respect the port layout we requested.
        let output = unsafe { ports.get_unchecked(0) };

        let to_consume = ctx.sample_count.min(self.0.slots());
        let data = self.0.read_chunk(to_consume).unwrap();

        if let Some(output) = output {
            // SAFETY: The caller must respect the port layout we requested.
            let output = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, C>>() };

            if data.is_empty() {
                // SAFETY: The caller must ensure that audio buffers are at least as large as
                // the sample count.
                unsafe { output.clear_to_unchecked(ctx.sample_count) };
                output.set_silent(true);
                data.commit_all();
            } else {
                output.set_silent(false);

                for (dst, src) in output
                    .frames_mut()
                    .zip(data.into_iter().chain(core::iter::repeat([0.0; C])))
                    .take(ctx.sample_count)
                {
                    for (dst, src) in dst.into_iter().zip(src) {
                        *dst = src;
                    }
                }
            }
        } else {
            // Consume the data even if nothing is connected.
            data.commit_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{audio_graph::AudioGraphRunner, testing};

    #[test]
    fn external_source() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, mut handle) = super::external_source::<1>(16);
        let s = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([0.0, 1.0, 2.0, 3.0, 4.0])));
        builder.connect(s, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(5));

        handle.feed_iter([0.0, 1.0, 2.0, 3.0, 4.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(5));
    }

    #[test]
    fn external_source_disconnected() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source::<1>(16);
        drop(handle);
        let s = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_silent()));
        builder.connect(s, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(5));
        runner.run(&mut testing::run_ctx(5));
    }

    #[test]
    fn external_source_underrun() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, mut handle) = super::external_source::<1>(16);
        let s = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([0.0, 1.0, 2.0, 0.0, 0.0])));
        builder.connect(s, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(5));

        handle.feed_iter([0.0, 1.0, 2.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(5));
    }
}
