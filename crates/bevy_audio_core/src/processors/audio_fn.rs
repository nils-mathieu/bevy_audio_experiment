use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo,
        RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
};

pub fn audio_fn<F, const C: usize>(f: F) -> AudioFn<F, C>
where
    AudioBuf<f32, C>: PortType,
    F: 'static + Send + FnMut(&mut RunCtx) -> [f32; C],
{
    AudioFn(f)
}

pub struct AudioFn<F, const C: usize>(F);

impl<F, const C: usize> Processor for AudioFn<F, C>
where
    AudioBuf<f32, C>: PortType,
    F: 'static + Send + FnMut(&mut RunCtx) -> [f32; C],
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
        // SAFETY: Caller must ensure that the ports match the layout we requested.
        let output = unsafe { ports.get_unchecked(0) };

        if let Some(output) = output {
            // SAFETY: Caller must ensure that the port is of the correct type.
            let buf = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, C>>() };

            buf.set_silent(false);
            for dst in buf.frames_mut().take(ctx.sample_count) {
                for (dst, src) in dst.into_iter().zip((self.0)(ctx)) {
                    *dst = src;
                }
            }
        }
    }
}
