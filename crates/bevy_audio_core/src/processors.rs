use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, PortTypeId, Processor,
        ProcessorInfo, RunCtx, SetupCtx,
    },
    alloc::borrow::Cow,
    std::marker::PhantomData,
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

            for dst in buf.frames_mut().take(ctx.sample_count) {
                for (dst, src) in dst.into_iter().zip((self.0)(ctx)) {
                    *dst += src;
                }
            }
        }
    }
}

pub fn map_audio_frames<F, const C: usize>(f: F) -> MapAudioFrames<F, C>
where
    AudioBuf<f32, C>: PortType,
    F: 'static + Send + FnMut(&mut RunCtx, [f32; C]) -> [f32; C],
{
    MapAudioFrames(f)
}

pub struct MapAudioFrames<F, const C: usize>(F);

impl<F, const C: usize> Processor for MapAudioFrames<F, C>
where
    AudioBuf<f32, C>: PortType,
    F: 'static + Send + FnMut(&mut RunCtx, [f32; C]) -> [f32; C],
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

            for (dst, src) in output_buf
                .frames_mut()
                .zip(input_buf.frames())
                .take(ctx.sample_count)
            {
                let mapped = (self.0)(ctx, src.map(|value| *value));
                for (dst, src) in dst.into_iter().zip(mapped) {
                    *dst += src;
                }
            }
        }
    }
}

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

            for (dst, src) in output_buf
                .frames_mut()
                .zip(input_buf.frames())
                .take(ctx.sample_count)
            {
                for (dst, src) in dst.into_iter().zip(src) {
                    *dst += (self.0)(ctx, *src);
                }
            }
        }
    }
}

pub fn sink_fn<F, D>(f: F) -> SinkFn<F, D>
where
    F: 'static + Send + FnMut(&mut RunCtx, Option<&D>),
    D: PortType,
{
    SinkFn {
        f,
        _port_data: PhantomData,
    }
}

pub struct SinkFn<F, D> {
    f: F,
    _port_data: PhantomData<D>,
}

impl<F, D> Processor for SinkFn<F, D>
where
    F: 'static + Send + FnMut(&mut RunCtx, Option<&D>),
    D: PortType,
{
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: vec![PortDescription {
                name: Some(Cow::Borrowed("input")),
                direction: PortDirection::Input,
                type_id: D::PORT_TYPE_ID,
            }],
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        let output = unsafe {
            ports
                .get_unchecked(0)
                .map(|port| port.downcast_ref_unchecked::<D>())
        };

        (self.f)(ctx, output);
    }
}

pub fn discard<D>() -> Discard<D>
where
    D: PortType,
{
    Discard(Default::default())
}

pub struct Discard<D>(PhantomData<D>);

impl<D> Default for Discard<D> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<D: PortType> Processor for Discard<D> {
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: vec![PortDescription {
                name: Some(Cow::Borrowed("input")),
                direction: PortDirection::Input,
                type_id: D::PORT_TYPE_ID,
            }],
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, _ctx: &mut RunCtx, _ports: &[Option<PortDataRaw>]) {}
}

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
                *dst_left += src;
                *dst_right += src;
            }
        }
    }
}
