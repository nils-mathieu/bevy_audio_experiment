use {
    crate::audio_graph::{
        PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo, RunCtx,
        SetupCtx,
    },
    alloc::borrow::Cow,
    core::marker::PhantomData,
};

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
