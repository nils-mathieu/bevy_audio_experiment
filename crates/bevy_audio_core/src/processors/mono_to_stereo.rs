use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortTypeId, Processor,
        ProcessorInfo, RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
};

pub fn mono_to_stereo() -> MonoToStereo {
    MonoToStereo
}

pub struct MonoToStereo;

impl Processor for MonoToStereo {
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: vec![
                PortDescription {
                    name: Some(Cow::Borrowed("input")),
                    direction: PortDirection::Input,
                    type_id: PortTypeId::Mono,
                },
                PortDescription {
                    name: Some(Cow::Borrowed("output")),
                    direction: PortDirection::Output,
                    type_id: PortTypeId::Stereo,
                },
            ],
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        // SAFETY: The caller must ensure that the port layout was respected.
        let input = unsafe { ports.get_unchecked(0) };
        let output = unsafe { ports.get_unchecked(1) };

        if let Some(input) = input
            && let Some(output) = output
        {
            // SAFETY: The caller must ensure that the port layout was respected.
            let input = unsafe { input.downcast_ref_unchecked::<AudioBuf<f32, 1>>() };
            let output = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, 2>>() };

            for ([dst_left, dst_right], [&src]) in output
                .frames_mut()
                .zip(input.frames())
                .take(ctx.sample_count)
            {
                *dst_left = src;
                *dst_right = src;
            }
        }
    }
}
