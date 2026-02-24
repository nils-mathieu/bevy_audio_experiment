use {
    super::{
        AudioBuf, Discrete, PortDataBox, PortDataRaw, PortTypeId, Processor, ProcessorInfo, RunCtx,
        SetupCtx,
    },
    crate::audio_graph::PortDirection,
    std::ops::Range,
};

pub type NodeId = usize;
pub type EdgeId = usize;
pub type ScratchpadId = usize;

struct NodeEntry {
    info: ProcessorInfo,
    processor: Box<dyn Processor>,

    /// # Invariants
    ///
    /// Can be either a valid indices into the [`AudioGraphBuilder::edges`] list, or `EdgeId::MAX`
    /// (unconnected port).
    edges: Box<[EdgeId]>,
    /// # Invariants
    ///
    /// Contains no duplicates. All [`NodeId`]s are valid into the [`AudioGraphBuilder::nodes`] list.
    dependencies: Vec<NodeId>,
    /// # Invariants
    ///
    /// Contains no duplicates. All [`NodeId`]s are valid into the [`AudioGraphBuilder::nodes`] list.
    dependents: Vec<NodeId>,
}

#[derive(Debug)]
pub struct EdgeEntry {
    type_id: PortTypeId,
    external: bool,
}

#[derive(Default)]
pub struct AudioGraphBuilder {
    nodes: Vec<NodeEntry>,
    edges: Vec<EdgeEntry>,
}

impl AudioGraphBuilder {
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
        self.nodes.push(NodeEntry {
            info,
            edges,
            processor,
            dependencies: Vec::new(),
            dependents: Vec::new(),
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
            !unsafe { self.depends_on_unchecked(src, dst) },
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

        push_unique(&mut src_runner.dependents, dst);
        push_unique(&mut dst_runner.dependencies, src);

        // SAFETY: `edges` and `info.ports` have the same length. We know that `src_port` and
        // `dst_port` are valid port indices.
        let src_edge = unsafe { src_runner.edges.get_unchecked_mut(src_port) };
        let dst_edge = unsafe { dst_runner.edges.get_unchecked_mut(dst_port) };

        assert!(*dst_edge == usize::MAX, "`dst_port` is already connected");

        // There are two cases:
        // 1. If the source edge is already connected, then use the same edge data entry.
        // 2. Otherwise, create a new one.

        if *src_edge != usize::MAX {
            *dst_edge = *src_edge;
            *src_edge
        } else {
            let edge_id = self.edges.len();
            self.edges.push(EdgeEntry {
                type_id: src_port_desc.type_id,
                external: false,
            });

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
            self.edges.push(EdgeEntry {
                type_id: port_info.type_id,
                external: true,
            });

            *edge_idx = edge_id;
            edge_id
        } else {
            // SAFETY: The indices in `edges` are valid indices.
            let edge = unsafe { self.edges.get_unchecked_mut(*edge_idx) };

            edge.external = true;
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

        let schedule = {
            let mut result = Vec::new();
            let mut pending_dependencies = self
                .nodes
                .iter()
                .map(|node| node.dependencies.len())
                .collect::<Vec<_>>();
            let mut to_visit = Vec::new();

            to_visit.extend(
                self.nodes
                    .iter()
                    .enumerate()
                    .filter(|(_, node)| node.dependencies.is_empty())
                    .map(|(id, _)| id),
            );

            while let Some(node_id) = to_visit.pop() {
                result.push(node_id);

                // SAFETY: The node indices in `to_visit` are valid indices into `self.nodes`.
                let node = unsafe { self.nodes.get_unchecked(node_id) };

                for &dep in node.dependents.iter() {
                    // SAFETY: `pending_dependencies` has the same length as `self.nodes`, which
                    // ensures that `dep` is valid.
                    let in_degree = unsafe { pending_dependencies.get_unchecked_mut(dep) };

                    debug_assert!(*in_degree > 0);
                    *in_degree -= 1;
                    if *in_degree == 0 {
                        to_visit.push(dep);
                    }
                }
            }

            debug_assert_eq!(
                result.len(),
                self.nodes.len(),
                "Cycle detected while building the graph",
            );

            result
        };

        let mut scratchpad = Vec::new();
        let mut edges = self
            .edges
            .iter()
            .map(|_| EdgeRunner {
                scratchpad_id: usize::MAX,
            })
            .collect::<Vec<_>>();
        let mut port_ptrs = Vec::new();

        let nodes = self
            .nodes
            .into_iter()
            .map(|node| {
                let port_ptr_start = port_ptrs.len();
                for &edge_id in node.edges.iter() {
                    if edge_id == EdgeId::MAX {
                        port_ptrs.push(None);
                        continue;
                    }

                    // SAFETY: The IDs in `node.edges` are known to be valid.
                    let edge = unsafe { self.edges.get_unchecked(edge_id) };

                    // SAFETY: `edges` has the same length as `self.edges`, so the index is also
                    // valid for this list.
                    let edge_runner = unsafe { edges.get_unchecked_mut(edge_id) };

                    let data = if edge_runner.scratchpad_id == usize::MAX {
                        edge_runner.scratchpad_id = scratchpad.len();

                        // FIXME: Don't add a new scratchpad element if one is already known to be
                        // available and unused by any node. When the edge is external, a new
                        // scratchpad element is always used.
                        scratchpad.push(create_port_data(edge.type_id, ctx));

                        // SAFETY: We just added that element.
                        unsafe { scratchpad.last_mut().unwrap_unchecked() }
                    } else {
                        // SAFETY: The `scratchpad_id` stored in edge runners are valid.
                        unsafe { scratchpad.get_unchecked_mut(edge_runner.scratchpad_id) }
                    };

                    port_ptrs.push(Some(data.as_raw()));
                }
                let port_ptr_end = port_ptrs.len();

                NodeRunner {
                    processor: node.processor,
                    port_ptr_range: port_ptr_start..port_ptr_end,
                }
            })
            .collect::<Vec<_>>();

        #[cfg(debug_assertions)]
        {
            for edge in edges.iter() {
                assert!(edge.scratchpad_id != usize::MAX);
            }
        }

        AudioGraphRunner {
            max_sample_count: ctx.max_sample_count,
            nodes,
            edges,
            scratchpad,
            port_ptrs,
            schedule,
        }
    }
}

fn create_port_data(type_id: PortTypeId, ctx: &mut SetupCtx) -> PortDataBox {
    match type_id {
        PortTypeId::Stereo => PortDataBox::new(AudioBuf::<f32, 2>::new(ctx.max_sample_count)),
        PortTypeId::Mono => PortDataBox::new(AudioBuf::<f32, 1>::new(ctx.max_sample_count)),
        PortTypeId::F32 => PortDataBox::new(Discrete::<f32>::with_capacity(
            ctx.max_sample_count / 16,
            0.0,
        )),
        PortTypeId::Bool => PortDataBox::new(Discrete::<bool>::with_capacity(
            ctx.max_sample_count / 16,
            false,
        )),
    }
}

struct NodeRunner {
    processor: Box<dyn Processor>,

    /// Range of ports that will be passed to [`Processor::run`] when running the graph.
    ///
    /// # Invariants
    ///
    /// Contains a valid range into [`AudioGraphRunner::port_ptrs`].
    port_ptr_range: Range<usize>,
}

struct EdgeRunner {
    /// Index of the edge's data into the scratchpad.
    ///
    /// # Invariants
    ///
    /// Is a valid index into [`AudioGraphRunner::scratchpad`].
    scratchpad_id: ScratchpadId,
}

#[derive(Default)]
pub struct AudioGraphRunner {
    max_sample_count: usize,
    nodes: Vec<NodeRunner>,
    edges: Vec<EdgeRunner>,
    scratchpad: Vec<PortDataBox>,

    /// # Invariants
    ///
    /// Contains valid pointers owned by `scratchpad`. Those pointers are stable so they aren't
    /// invalidated even when scratchpad is reallocated.
    ///
    /// Indexed by [`NodeRunner::port_ptr_range`].
    port_ptrs: Vec<Option<PortDataRaw>>,
    /// Topological sort of the graph, used when running the graph.
    ///
    /// # Invariants
    ///
    /// Contains valid indices into `nodes`.
    schedule: Vec<NodeId>,
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
        // SAFETY: The `scratchpad_id` stored in edges is always valid.
        self.edges
            .get(edge)
            .map(|edge| unsafe { self.scratchpad.get_unchecked(edge.scratchpad_id) })
    }

    pub fn get_external_edge_mut(&mut self, edge: EdgeId) -> Option<&mut PortDataBox> {
        // SAFETY: The `scratchpad_id` stored in edges is always valid.
        self.edges
            .get(edge)
            .map(|edge| unsafe { self.scratchpad.get_unchecked_mut(edge.scratchpad_id) })
    }

    pub fn run(&mut self, ctx: &mut RunCtx) {
        assert!(ctx.sample_count <= self.max_sample_count);

        for &node_id in self.schedule.iter() {
            // SAFETY: The node IDs in `schedule` are always valid.
            let node = unsafe { self.nodes.get_unchecked_mut(node_id) };

            // SAFETY: When building the graph, the `port_ptr_range` is set to the correct range
            // of pointers.
            unsafe {
                node.processor.run(
                    ctx,
                    self.port_ptrs.get_unchecked(node.port_ptr_range.clone()),
                )
            };
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

    #[test]
    #[should_panic = "`src` node is invalid"]
    fn connect_invalid_src_node() {
        let mut builder = AudioGraphRunner::builder();
        let valid_id = builder.insert(Box::new(assert_sink([])));
        builder.connect(12, 0, valid_id, 0);
    }

    #[test]
    #[should_panic = "`dst` node is invalid"]
    fn connect_invalid_dst_node() {
        let mut builder = AudioGraphRunner::builder();
        let valid_id = builder.insert(Box::new(audio_iter([])));
        builder.connect(valid_id, 0, 12, 0);
    }

    #[test]
    #[should_panic = "Can't connect a node to itself"]
    fn connect_node_to_itself() {
        let mut builder = AudioGraphRunner::builder();
        let valid_id = builder.insert(Box::new(map_audio(|x| x)));
        builder.connect(valid_id, 1, valid_id, 0);
    }

    #[test]
    #[should_panic = "Cycle introduced"]
    fn connect_creates_cycle() {
        let mut builder = AudioGraphRunner::builder();
        let a = builder.insert(Box::new(map_audio(|x| x)));
        let b = builder.insert(Box::new(map_audio(|x| x)));
        let c = builder.insert(Box::new(map_audio(|x| x)));
        builder.connect(a, 1, b, 0);
        builder.connect(b, 1, c, 0);
        builder.connect(c, 1, a, 0);
    }

    #[test]
    #[should_panic = "`src_port` is invalid"]
    fn connect_invalid_src_port() {
        let mut builder = AudioGraphRunner::builder();
        let src_id = builder.insert(Box::new(audio_iter([])));
        let dst_id = builder.insert(Box::new(assert_sink([])));
        builder.connect(src_id, 12, dst_id, 0);
    }

    #[test]
    #[should_panic = "`dst_port` is invalid"]
    fn connect_invalid_dst_port() {
        let mut builder = AudioGraphRunner::builder();
        let src_id = builder.insert(Box::new(audio_iter([])));
        let dst_id = builder.insert(Box::new(assert_sink([])));
        builder.connect(src_id, 0, dst_id, 12);
    }

    #[test]
    #[should_panic = "`src_port` must be an output port"]
    fn connect_invalid_src_port_direction() {
        let mut builder = AudioGraphRunner::builder();
        let src_id = builder.insert(Box::new(assert_sink([])));
        let dst_id = builder.insert(Box::new(assert_sink([])));
        builder.connect(src_id, 0, dst_id, 0);
    }

    #[test]
    #[should_panic = "`dst_port` must be an input port"]
    fn connect_invalid_dst_port_direction() {
        let mut builder = AudioGraphRunner::builder();
        let src_id = builder.insert(Box::new(audio_iter([])));
        let dst_id = builder.insert(Box::new(audio_iter([])));
        builder.connect(src_id, 0, dst_id, 0);
    }

    #[test]
    #[should_panic = "Incompatible port types"]
    fn connect_incompatible_ports() {
        let mut builder = AudioGraphRunner::builder();

        let src_id = builder.insert(Box::new(processors::audio_fn(|_: &mut RunCtx| [0.0, 0.0])));
        let dst_id = builder.insert(Box::new(processors::sink_fn(
            |_: &mut RunCtx, _: Option<&AudioBuf<f32, 1>>| (),
        )));

        builder.connect(src_id, 0, dst_id, 0);
    }

    #[test]
    #[should_panic = "`node` is not a valid node"]
    fn connect_external_invalid_node() {
        let mut builder = AudioGraphRunner::builder();
        builder.connect_external(12, 0);
    }

    #[test]
    #[should_panic = "`port` is not a valid port for the node"]
    fn connect_external_invalid_port() {
        let mut builder = AudioGraphRunner::builder();
        let id = builder.insert(Box::new(audio_iter([])));
        builder.connect_external(id, 100);
    }

    #[test]
    #[should_panic = "`edge` is not a valid edge"]
    fn make_external_fails_with_invalid_edge() {
        let mut builder = AudioGraphRunner::builder();
        builder.make_external(100);
    }
}
