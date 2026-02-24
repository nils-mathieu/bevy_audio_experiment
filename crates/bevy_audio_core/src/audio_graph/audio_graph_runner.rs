use {
    super::{
        AudioBuf, Discrete, PortDataBox, PortDataRaw, PortTypeId, Processor, ProcessorInfo, RunCtx,
        SetupCtx,
    },
    crate::audio_graph::PortDirection,
    core::{cell::Cell, hint::assert_unchecked},
};

pub type NodeId = usize;
pub type EdgeId = usize;

struct NodeRunner {
    info: ProcessorInfo,
    processor: Box<dyn Processor>,

    /// # Invariants
    ///
    /// Can be either a valid indices into the [`AudioGraphRunner::edges`] list, or `EdgeId::MAX`
    /// (unconnected port).
    edges: Box<[EdgeId]>,
    /// # Invariants
    ///
    /// Contains no duplicates. All [`NodeId`]s are valid into the [`AudioGraphRunner::nodes`] list.
    dependencies: Vec<NodeId>,
    /// # Invariants
    ///
    /// Contains no duplicates. All [`NodeId`]s are valid into the [`AudioGraphRunner::nodes`] list.
    dependents: Vec<NodeId>,

    /// Only used during graph execution. Contains the number of dependencies of this node
    /// that haven't yet finished executing.
    ///
    /// When this number reaches zero, the node is scheduled for running.
    ///
    /// NOTE: Can be turned into an `AtomicUsize` if we want to execute the graph in parallel.
    pending_dependencies: Cell<usize>,
}

#[derive(Default)]
struct PortDataAllocator {
    stereo: Vec<PortDataBox>,
    mono: Vec<PortDataBox>,
    f32: Vec<PortDataBox>,
    bool: Vec<PortDataBox>,
}

impl PortDataAllocator {
    pub fn allocate(&mut self, type_id: PortTypeId) -> Option<PortDataBox> {
        match type_id {
            PortTypeId::Stereo => self.stereo.pop(),
            PortTypeId::Mono => self.mono.pop(),
            PortTypeId::F32 => self.f32.pop(),
            PortTypeId::Bool => self.bool.pop(),
        }
    }

    pub fn deallocate(&mut self, data: PortDataBox) {
        match data.type_id() {
            PortTypeId::Stereo => self.stereo.push(data),
            PortTypeId::Mono => self.mono.push(data),
            PortTypeId::F32 => self.f32.push(data),
            PortTypeId::Bool => self.bool.push(data),
        }
    }
}

#[derive(Debug)]
pub struct EdgeData {
    type_id: PortTypeId,

    /// # Invariants
    ///
    /// * If `external` is `true`, then this is always `Some(_)` after the graph
    ///   is built. External edges have their own [`PortDataBox`] which isn't taken from the
    ///   allocator.
    /// * If `external` is `false`, then this is `Some(_)` during graph execution if the node
    ///   has `pending_inputs > 0`.
    data: Option<PortDataBox>,

    total_inputs: usize,
    /// # Invariants
    ///
    /// This is always less or equal to `total_inputs`.
    pending_inputs: usize,

    external: bool,
}

#[derive(Default)]
pub struct AudioGraphBuilder {
    nodes: Vec<NodeRunner>,
    edges: Vec<EdgeData>,
}

impl AudioGraphBuilder {
    #[track_caller]
    pub fn depends_on(&self, a: NodeId, b: NodeId) -> bool {
        assert!(a < self.nodes.len(), "Node IDs must be valid");

        // SAFETY: We just made sure that `a` and `b` are valid node IDs.
        unsafe { self.depends_on_unchecked(a, b) }
    }

    /// # Safety
    ///
    /// `a` must be a valid node ID.
    unsafe fn depends_on_unchecked(&self, a: NodeId, b: NodeId) -> bool {
        // SAFETY: The caller must ensure that the node is valid.
        let a_runner = unsafe { self.nodes.get_unchecked(a) };

        // SAFETY: The nodes stored in `dependencies` are valid.
        a_runner
            .dependencies
            .iter()
            .any(|&dep| dep == b || unsafe { self.depends_on_unchecked(dep, b) })
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn insert(&mut self, processor: Box<dyn Processor>) -> NodeId {
        let id = self.nodes.len();
        let info = processor.info();
        let edges = info.ports.iter().map(|_| usize::MAX).collect();
        self.nodes.push(NodeRunner {
            info,
            edges,
            processor,
            dependencies: Vec::new(),
            dependents: Vec::new(),
            pending_dependencies: Cell::new(0),
        });
        id
    }

    #[track_caller]
    pub fn connect(
        &mut self,
        src: NodeId,
        src_port: usize,
        dst: NodeId,
        dst_port: usize,
    ) -> EdgeId {
        assert!(src < self.nodes.len(), "`src` node is invalid");
        assert!(dst < self.nodes.len(), "`dst` node is invalid");
        assert!(src != dst, "Can't connect a node to itself");

        // SAFETY: `src` has been checked to be a valid node.
        assert!(
            !unsafe { self.depends_on_unchecked(dst, src) },
            "Cycle introduced"
        );

        // SAFETY: We just made sure that `src` and `dst` are not the same node, and that they
        // are valid nodes.
        let [src_runner, dst_runner] = unsafe { self.nodes.get_disjoint_unchecked_mut([src, dst]) };

        assert!(
            src_port < src_runner.info.ports.len(),
            "`src_port` is invalid",
        );
        assert!(
            dst_port < dst_runner.info.ports.len(),
            "`dst_port` is invalid",
        );

        // SAFETY: We just made sure that the port indices are valid.
        let src_port_desc = unsafe { src_runner.info.ports.get_unchecked(src_port) };
        let dst_port_desc = unsafe { dst_runner.info.ports.get_unchecked(dst_port) };

        assert!(
            src_port_desc.direction == PortDirection::Output,
            "`src_port` must be an output port",
        );
        assert!(
            dst_port_desc.direction == PortDirection::Input,
            "`dst_port` must be an input port",
        );
        assert_eq!(
            src_port_desc.type_id, dst_port_desc.type_id,
            "Incompatible port types",
        );

        // SAFETY: `edges` and `info.ports` have the same length. We know that `src_port` and
        // `dst_port` are valid port indices.
        let src_edge = unsafe { src_runner.edges.get_unchecked_mut(src_port) };
        let dst_edge = unsafe { dst_runner.edges.get_unchecked_mut(dst_port) };

        assert!(*dst_edge == usize::MAX, "`dst_port` is already connected");

        // There are two cases:
        // 1. If the source edge is already connected, then use the same edge data entry.
        // 2. Otherwise, create a new one.

        if *src_edge != usize::MAX {
            // SAFETY: When not `usize::MAX`, edge indices are valid.
            let edge_data = unsafe { self.edges.get_unchecked_mut(*src_edge) };

            *dst_edge = *src_edge;
            edge_data.total_inputs += 1;

            *src_edge
        } else {
            let edge_id = self.edges.len();
            self.edges.push(EdgeData {
                type_id: src_port_desc.type_id,
                data: None,
                total_inputs: 1,
                pending_inputs: 0,
                external: false,
            });

            push_unique(&mut src_runner.dependents, dst);
            push_unique(&mut dst_runner.dependencies, src);

            *src_edge = edge_id;
            *dst_edge = edge_id;

            edge_id
        }
    }

    #[track_caller]
    pub fn connect_external(&mut self, node: NodeId, port: usize) -> EdgeId {
        let runner = self
            .nodes
            .get_mut(node)
            .expect("`node` is not a valid node");

        let port_info = runner
            .info
            .ports
            .get(port)
            .expect("`port` is not a valid port for the node");

        // SAFETY: `edges` and `info.ports` have the same length. We know that `port` is a valid
        // port index, so it is in bounds here too.
        let edge_idx = unsafe { runner.edges.get_unchecked_mut(port) };

        if *edge_idx == usize::MAX {
            let edge_id = self.edges.len();
            self.edges.push(EdgeData {
                type_id: port_info.type_id,
                data: None,
                total_inputs: (port_info.direction == PortDirection::Input) as usize,
                pending_inputs: 0,
                external: true,
            });

            *edge_idx = edge_id;

            edge_id
        } else {
            // SAFETY: The indices in `edges` are valid indices.
            let edge = unsafe { self.edges.get_unchecked_mut(*edge_idx) };

            edge.external = true;

            if port_info.direction == PortDirection::Input {
                edge.total_inputs += 1;
            }

            *edge_idx
        }
    }

    pub fn make_external(&mut self, edge: EdgeId) {
        self.edges
            .get_mut(edge)
            .expect("`edge` is not a valid edge")
            .external = true;
    }

    pub fn build(mut self, ctx: &mut SetupCtx) -> AudioGraphRunner {
        for node in self.nodes.iter_mut() {
            node.processor.setup(ctx);
        }

        let max_port_count = self
            .nodes
            .iter()
            .map(|x| x.edges.len())
            .max()
            .unwrap_or_default();
        let port_data = Vec::with_capacity(max_port_count);

        // TODO: The queue must be large enough to hold the maximum number of nodes that might
        // become ready at once. `self.nodes.len()` is an upper bound but it might be possible to
        // compute a more efficient value.
        let ready_queue = Vec::with_capacity(self.nodes.len());

        // TODO: Only allocate what is needed instead of one buffer per edge.
        let mut port_data_allocator = PortDataAllocator::default();
        for edge in self.edges.iter_mut() {
            let data = match edge.type_id {
                PortTypeId::Stereo => PortDataBox::new(AudioBuf::<f32, 2>::with_capacity(
                    ctx.max_sample_count,
                    || 0.0,
                )),
                PortTypeId::Mono => PortDataBox::new(AudioBuf::<f32, 1>::with_capacity(
                    ctx.max_sample_count,
                    || 0.0,
                )),
                PortTypeId::F32 => PortDataBox::new(Discrete::<f32>::with_capacity(
                    ctx.max_sample_count / 16,
                    0.0,
                )),
                PortTypeId::Bool => PortDataBox::new(Discrete::<bool>::with_capacity(
                    ctx.max_sample_count / 16,
                    false,
                )),
            };

            if edge.external {
                edge.data = Some(data);
            } else {
                port_data_allocator.deallocate(data);
            }
        }

        AudioGraphRunner {
            max_sample_count: ctx.max_sample_count,
            nodes: self.nodes,
            edges: self.edges,
            port_data_allocator,
            port_data,
            ready_queue,
        }
    }
}

#[derive(Default)]
pub struct AudioGraphRunner {
    max_sample_count: usize,
    nodes: Vec<NodeRunner>,
    edges: Vec<EdgeData>,
    port_data_allocator: PortDataAllocator,
    port_data: Vec<Option<PortDataRaw>>,

    /// # Invariants
    ///
    /// * Must be large enough to hold all nodes that might become ready at the same time during
    ///   graph execution without reallocating.
    /// * All stored node IDs are valid indices into `self.nodes`.
    ready_queue: Vec<NodeId>,
}

impl AudioGraphRunner {
    pub fn builder() -> AudioGraphBuilder {
        AudioGraphBuilder::default()
    }

    pub fn max_sample_count(&self) -> usize {
        self.max_sample_count
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn get_external_edge(&self, edge: EdgeId) -> Option<&PortDataBox> {
        // SAFETY: If `external` is set, then the `data` is always set.
        self.edges
            .get(edge)
            .filter(|x| x.external)
            .map(|x| unsafe { x.data.as_ref().unwrap_unchecked() })
    }

    pub fn get_external_edge_mut(&mut self, edge: EdgeId) -> Option<&mut PortDataBox> {
        // SAFETY: If `external` is set, then the `data` is always set.
        self.edges
            .get_mut(edge)
            .filter(|x| x.external)
            .map(|x| unsafe { x.data.as_mut().unwrap_unchecked() })
    }

    pub fn get_edge(&self, node: NodeId, port: usize) -> Option<EdgeId> {
        self.nodes
            .get(node)
            .and_then(|node| node.edges.get(port))
            .copied()
            .filter(|&edge| edge != usize::MAX)
    }

    pub fn run(&mut self, ctx: &mut RunCtx) {
        assert!(ctx.sample_count <= self.max_sample_count);

        self.ready_queue.clear();

        for (node_id, node) in self.nodes.iter_mut().enumerate() {
            *node.pending_dependencies.get_mut() = node.dependencies.len();
            if *node.pending_dependencies.get_mut() == 0 {
                self.ready_queue.push(node_id);
            }
        }

        for edge in self.edges.iter_mut() {
            edge.pending_inputs = edge.total_inputs;
        }

        while let Some(node_id) = self.ready_queue.pop() {
            // SAFETY: The nodes in `ready_queue` are always valid.
            let node = unsafe { self.nodes.get_unchecked_mut(node_id) };

            self.port_data.clear();
            debug_assert!(self.port_data.capacity() >= node.edges.len());
            for i in 0..node.edges.len() {
                // SAFETY: `i < edges.len()`
                let edge_idx = unsafe { *node.edges.get_unchecked(i) };

                if edge_idx == usize::MAX {
                    self.port_data.push(None);
                    continue;
                }

                // SAFETY: The edges in `node.edges` are always valid.
                let edge = unsafe { self.edges.get_unchecked_mut(edge_idx) };

                // SAFETY: `info.ports` has the same length as `edges`, and `i < edges.len()`.
                let port_info = unsafe { node.info.ports.get_unchecked(i) };

                debug_assert_eq!(port_info.type_id, edge.type_id);

                let edge_data = match port_info.direction {
                    PortDirection::Output => {
                        // This is an output. We need to allocate a new buffer and initialize
                        // the edge data.

                        if edge.external {
                            debug_assert!(edge.data.is_some());

                            // SAFETY: If the edge is external, `data` is always set.
                            unsafe { edge.data.as_mut().unwrap_unchecked() }
                        } else {
                            let data = self.port_data_allocator.allocate(port_info.type_id);
                            debug_assert!(data.is_some());
                            // SAFETY: During `build()`, sufficient buffers are pre-allocated for
                            // all edges. Non-external edges have their buffers deallocated to the
                            // allocator, ensuring availability during execution.
                            let data = unsafe { data.unwrap_unchecked() };

                            debug_assert!(edge.data.is_none());

                            // SAFETY: Non-external output edges have `data = None` at the start
                            // of processing. This hint eliminates the branch in `Option::insert`
                            // that would drop an existing value.
                            unsafe { assert_unchecked(edge.data.is_none()) };

                            edge.data.insert(data)
                        }
                    }
                    PortDirection::Input => {
                        debug_assert!(edge.data.is_some());
                        debug_assert!(edge.pending_inputs > 0);

                        edge.pending_inputs -= 1;

                        // SAFETY: Input ports only execute after their source has produced output.
                        // The topological ordering guarantees that `data` is `Some` when inputs
                        // are processed.
                        unsafe { edge.data.as_mut().unwrap_unchecked() }
                    }
                };

                self.port_data.push(Some(edge_data.as_raw()));
            }

            // SAFETY: We just initialized `self.port_data` so it contains the ports for this
            // processor.
            unsafe { node.processor.run(ctx, self.port_data.as_slice()) };

            for i in 0..node.edges.len() {
                // SAFETY: Loop bound ensures `i < node.edges.len()`
                let edge_idx = unsafe { *node.edges.get_unchecked(i) };

                if edge_idx == usize::MAX {
                    continue;
                }

                // SAFETY: The edges in `node.edges` are always valid.
                let edge = unsafe { self.edges.get_unchecked_mut(edge_idx) };

                if edge.pending_inputs == 0 && !edge.external {
                    debug_assert!(edge.data.is_some());

                    // SAFETY: Non-external edges with `pending_inputs = 0` have just finished processing
                    // all inputs, so `data` must be `Some` (it was set when the output was produced).
                    self.port_data_allocator
                        .deallocate(unsafe { edge.data.take().unwrap_unchecked() });
                }
            }

            // SAFETY: The nodes in `ready_queue` are always valid.
            let node = unsafe { self.nodes.get_unchecked(node_id) };
            for &dependent_id in node.dependents.iter() {
                // SAFETY: The nodes in `dependents` are always valid.
                let dependent = unsafe { self.nodes.get_unchecked(dependent_id) };

                let prev = dependent.pending_dependencies.get();
                dependent.pending_dependencies.set(prev - 1);
                if prev == 1 {
                    debug_assert!(self.ready_queue.len() < self.ready_queue.capacity());
                    self.ready_queue.push(dependent_id);
                }
            }
        }
    }
}

fn push_unique<T: PartialEq>(v: &mut Vec<T>, item: T) {
    if !v.contains(&item) {
        v.push(item);
    }
}

#[cfg(test)]
mod tests {
    use {
        super::AudioGraphRunner,
        crate::{
            audio_graph::{AudioBuf, Processor, RunCtx, SetupCtx},
            processors,
        },
    };

    fn run_ctx(sample_count: usize) -> RunCtx {
        RunCtx {
            sample_count,
            sample_rate: 44100,
            sample_rate_recip: 44100f64.recip(),
        }
    }

    fn setup_ctx(max_sample_count: usize) -> SetupCtx {
        SetupCtx {
            max_sample_count,
            sample_rate: 44100,
            sample_rate_recip: 44100f64.recip(),
        }
    }

    fn map_audio(mut f: impl 'static + Send + FnMut(f32) -> f32) -> impl Processor {
        processors::map_audio::<_, 1>(move |_: &mut RunCtx, x| f(x))
    }

    fn assert_sink<I>(expected: I) -> impl Processor
    where
        I: IntoIterator<Item = f32>,
        I::IntoIter: 'static + Send + ExactSizeIterator,
    {
        let mut iter = expected.into_iter();
        processors::sink_fn(move |ctx: &mut RunCtx, out: Option<&AudioBuf<f32, 1>>| {
            let actual = out
                .into_iter()
                .flat_map(|x| x.as_slice())
                .take(ctx.sample_count);
            for (&actual, expected) in actual.zip(&mut iter) {
                assert_eq!(actual, expected);
            }
        })
    }

    fn audio_iter<I>(iter: I) -> impl Processor
    where
        I: IntoIterator<Item = f32>,
        I::IntoIter: 'static + Send,
    {
        let mut iter = iter.into_iter();
        processors::audio_fn(move |_: &mut RunCtx| [iter.next().unwrap()])
    }

    #[test]
    fn empty_runner() {
        let mut runner = AudioGraphRunner::builder().build(&mut setup_ctx(16));
        runner.run(&mut run_ctx(16));
        runner.run(&mut run_ctx(8));
    }

    #[test]
    fn one_node() {
        let mut builder = AudioGraphRunner::builder();
        builder.insert(Box::new(map_audio(|x| x * 2.0)));
        let mut runner = builder.build(&mut setup_ctx(8));
        runner.run(&mut run_ctx(8));
        runner.run(&mut run_ctx(4));
    }

    #[test]
    fn source_and_sink_basic() {
        let mut builder = AudioGraphRunner::builder();
        let sink_id = builder.insert(Box::new(assert_sink([1.0, 2.0, 3.0])));
        let source_id = builder.insert(Box::new(audio_iter([1.0, 2.0, 3.0])));
        builder.connect(source_id, 0, sink_id, 0);
        let mut runner = builder.build(&mut setup_ctx(3));
        runner.run(&mut run_ctx(3));
    }

    #[test]
    fn source_and_sink_three_runs() {
        let mut builder = AudioGraphRunner::builder();
        let sink_id = builder.insert(Box::new(assert_sink([1.0, 2.0, 3.0])));
        let source_id = builder.insert(Box::new(audio_iter([1.0, 2.0, 3.0])));
        builder.connect(source_id, 0, sink_id, 0);
        let mut runner = builder.build(&mut setup_ctx(3));
        runner.run(&mut run_ctx(1));
        runner.run(&mut run_ctx(1));
        runner.run(&mut run_ctx(1));
    }

    #[test]
    fn basic_processing() {
        let mut builder = AudioGraphRunner::builder();
        let source_id = builder.insert(Box::new(audio_iter([1.0, 2.0, 3.0])));
        let double_id = builder.insert(Box::new(map_audio(|x| x * 2.0)));
        let sink_id = builder.insert(Box::new(assert_sink([2.0, 4.0, 6.0])));
        builder.connect(source_id, 0, double_id, 0);
        builder.connect(double_id, 1, sink_id, 0);
        let mut runner = builder.build(&mut setup_ctx(3));
        runner.run(&mut run_ctx(3));
    }

    #[test]
    fn basic_external_output() {
        let mut builder = AudioGraphRunner::builder();
        let source_id = builder.insert(Box::new(audio_iter([1.0, 2.0, 3.0])));
        let edge_id = builder.connect_external(source_id, 0);
        let mut runner = builder.build(&mut setup_ctx(8));

        runner.run(&mut run_ctx(3));

        let data = runner
            .get_external_edge(edge_id)
            .unwrap()
            .downcast_ref::<AudioBuf<f32, 1>>()
            .unwrap();

        assert_eq!(&data.as_slice()[0..3], &[1.0, 2.0, 3.0]);
    }

    #[test]
    fn basic_external_input() {
        let mut builder = AudioGraphRunner::builder();
        let sink_id = builder.insert(Box::new(assert_sink([1.0, 2.0, 4.0])));
        let edge_id = builder.connect_external(sink_id, 0);
        let mut runner = builder.build(&mut setup_ctx(8));

        let data = runner
            .get_external_edge_mut(edge_id)
            .unwrap()
            .downcast_mut::<AudioBuf<f32, 1>>()
            .unwrap();

        data.as_mut_slice()[0] = 1.0;
        data.as_mut_slice()[1] = 2.0;
        data.as_mut_slice()[2] = 4.0;

        runner.run(&mut run_ctx(3));
    }
}
