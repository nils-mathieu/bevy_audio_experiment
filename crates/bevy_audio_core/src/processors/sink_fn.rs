use {
    crate::audio_graph::{
        PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo, RunCtx,
        SetupCtx,
    },
    alloc::borrow::Cow,
    core::marker::PhantomData,
};

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
