use {
    super::{PortDataRaw, PortTypeId},
    alloc::borrow::Cow,
};

#[derive(Debug)]
pub struct RunCtx {
    pub sample_count: usize,
    pub sample_rate: u32,
    pub sample_rate_recip: f64,
}

#[derive(Debug)]
pub struct SetupCtx {
    pub max_sample_count: usize,
    pub sample_rate: u32,
    pub sample_rate_recip: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortDirection {
    Input,
    Output,
}

#[derive(Debug, Clone)]
pub struct PortDescription {
    pub name: Option<Cow<'static, str>>,
    pub direction: PortDirection,
    pub type_id: PortTypeId,
}

#[derive(Debug, Clone)]
pub struct ProcessorInfo {
    pub ports: Vec<PortDescription>,
}

pub trait Processor: 'static + Send {
    fn info(&self) -> ProcessorInfo;

    fn setup(&mut self, ctx: &mut SetupCtx);

    /// # Safety
    ///
    /// * `ports` must match the layout specified by [`info()`].
    ///
    /// [`info()`]: Processor::info
    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]);
}

impl<T: ?Sized + Processor> Processor for Box<T> {
    fn info(&self) -> ProcessorInfo {
        T::info(self)
    }

    fn setup(&mut self, ctx: &mut SetupCtx) {
        T::setup(self, ctx)
    }

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        unsafe { T::run(self, ctx, ports) }
    }
}
