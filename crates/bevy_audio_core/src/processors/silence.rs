use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo,
        RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
};

pub fn silence<const C: usize>() -> Silence<C> {
    Silence
}

pub struct Silence<const C: usize>;

impl<const C: usize> Processor for Silence<C>
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
        // SAFETY: Caller must ensure that the ports match the layout we requested.
        let output = unsafe { ports.get_unchecked(0) };

        if let Some(output) = output {
            // SAFETY: Caller must ensure that the port is of the correct type.
            let buf = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, C>>() };

            buf.set_silent(true);

            // SAFETY: Caller must ensure that audio buffers are at least as large as
            // `sample_count`.
            unsafe { buf.clear_to_unchecked(ctx.sample_count) };
        }
    }
}
