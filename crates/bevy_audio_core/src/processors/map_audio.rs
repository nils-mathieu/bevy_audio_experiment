use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo,
        RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
};

pub fn map_audio<F, const C: usize>(f: F) -> MapAudio<F, C>
where
    AudioBuf<f32, C>: PortType,
    F: 'static + Send + FnMut(&mut RunCtx, f32) -> f32,
{
    MapAudio(f)
}

pub struct MapAudio<F, const C: usize>(F);

impl<F, const C: usize> Processor for MapAudio<F, C>
where
    AudioBuf<f32, C>: PortType,
    F: 'static + Send + FnMut(&mut RunCtx, f32) -> f32,
{
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: vec![
                PortDescription {
                    name: Some(Cow::Borrowed("input")),
                    direction: PortDirection::Input,
                    type_id: <AudioBuf<f32, C>>::PORT_TYPE_ID,
                },
                PortDescription {
                    name: Some(Cow::Borrowed("output")),
                    direction: PortDirection::Output,
                    type_id: <AudioBuf<f32, C>>::PORT_TYPE_ID,
                },
            ],
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        // SAFETY: Caller must ensure that the ports match the layout we requested.
        let input = unsafe { ports.get_unchecked(0) };
        let output = unsafe { ports.get_unchecked(1) };

        if let Some(input) = input
            && let Some(output) = output
        {
            // SAFETY: Caller must ensure that the port is of the correct type.
            let input_buf = unsafe { input.downcast_ref_unchecked::<AudioBuf<f32, C>>() };
            let output_buf = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, C>>() };

            output_buf.set_silent(false);

            for (dst, src) in output_buf
                .flat_iter_mut()
                .zip(input_buf.flat_iter())
                .take(ctx.sample_count)
            {
                *dst = (self.0)(ctx, *src);
            }
        }
    }
}
