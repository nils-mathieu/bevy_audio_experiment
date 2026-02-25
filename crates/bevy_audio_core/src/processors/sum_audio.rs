use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo,
        RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
};

pub fn sum_audio<const C: usize>(inputs: usize) -> SumAudio<C>
where
    AudioBuf<f32, C>: PortType,
{
    SumAudio { inputs }
}

pub struct SumAudio<const C: usize> {
    inputs: usize,
}

impl<const C: usize> Processor for SumAudio<C>
where
    AudioBuf<f32, C>: PortType,
{
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: core::iter::once(PortDescription {
                name: Some(Cow::Borrowed("output")),
                direction: PortDirection::Output,
                type_id: <AudioBuf<f32, C>>::PORT_TYPE_ID,
            })
            .chain((0..self.inputs).map(|_| PortDescription {
                name: None,
                direction: PortDirection::Input,
                type_id: <AudioBuf<f32, C>>::PORT_TYPE_ID,
            }))
            .collect(),
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        // SAFETY: The caller must provide the port layout we requested.
        let output = unsafe { ports.get_unchecked(0) };

        let Some(output) = output else { return };

        // SAFETY: The caller must provide the port layout we requested.
        let output = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, C>>() };

        // SAFETY: The provided audio buffers must be at least as large as `ctx.sample_count`.
        unsafe { output.clear_to_unchecked(ctx.sample_count) };

        output.set_silent(true);

        // SAFETY: The caller must provide the port layout we requested.
        let inputs = unsafe { ports.get_unchecked(1..) };

        for input in inputs.iter().copied().flatten() {
            // SAFETY: The caller must provide the port layout we requested.
            let input = unsafe { input.downcast_ref_unchecked::<AudioBuf<f32, C>>() };

            if input.is_silent() {
                continue;
            }

            output.set_silent(false);

            for (dst, &src) in output
                .as_mut_slice()
                .iter_mut()
                .zip(input.as_slice())
                .take(ctx.sample_count)
            {
                *dst += src;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{audio_graph::AudioGraphRunner, processors, testing};

    #[test]
    fn sum_sources() {
        let mut builder = AudioGraphRunner::builder();
        let a = builder.insert(Box::new(testing::audio_iter([1.0, 2.0, 3.0])));
        let b = builder.insert(Box::new(testing::audio_iter([4.0, 5.0, 6.0])));
        let c = builder.insert(Box::new(testing::audio_iter([7.0, 0.0, 7.0])));
        let sum = builder.insert(Box::new(super::sum_audio::<1>(3)));
        let out = builder.insert(Box::new(testing::assert_sink([12.0, 7.0, 17.0])));
        builder.connect(a, 0, sum, 1);
        builder.connect(b, 0, sum, 2);
        builder.connect(c, 0, sum, 3);
        builder.connect(sum, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(3));
        runner.run(&mut testing::run_ctx(0));
    }

    #[test]
    fn sum_no_sources() {
        let mut builder = AudioGraphRunner::builder();
        let sum = builder.insert(Box::new(super::sum_audio::<1>(3)));
        let out = builder.insert(Box::new(testing::assert_silent()));
        builder.connect(sum, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(3));
        runner.run(&mut testing::run_ctx(0));
    }

    #[test]
    fn sum_silent_sources() {
        let mut builder = AudioGraphRunner::builder();
        let a = builder.insert(Box::new(processors::silence::<1>()));
        let b = builder.insert(Box::new(processors::silence::<1>()));
        let c = builder.insert(Box::new(processors::silence::<1>()));
        let sum = builder.insert(Box::new(processors::sum_audio::<1>(3)));
        let out = builder.insert(Box::new(testing::assert_silent()));
        builder.connect(a, 0, sum, 1);
        builder.connect(b, 0, sum, 2);
        builder.connect(c, 0, sum, 3);
        builder.connect(sum, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(3));
        runner.run(&mut testing::run_ctx(0));
    }
}
