use {
    crate::{
        audio_graph::{
            AudioBuf, AudioGraphRunner, PortDirection, PortTypeId, Processor, ProcessorInfo,
        },
        audio_thread_driver::AudioThreadHandle,
        processors,
    },
    alloc::sync::Arc,
    bevy_ecs::{entity::EntityHashMap, lifecycle::HookContext, prelude::*, world::DeferredWorld},
    std::borrow::Cow,
};

/// A factory for creating processors.
pub type ProcessorFactory = dyn 'static + Sync + Send + Fn() -> Box<dyn Processor>;

#[derive(Clone, Component)]
#[component(on_remove)]
pub struct AudioGraphNode {
    info: ProcessorInfo,
    processor_factory: Arc<ProcessorFactory>,
    edges: Vec<AudioGraphEdgeId>,
}

impl AudioGraphNode {
    fn on_remove(mut world: DeferredWorld, ctx: HookContext) {
        // Remove all edges connected to this node.
        let edges = std::mem::take(&mut world.get_mut::<AudioGraphNode>(ctx.entity).unwrap().edges);
        let mut commands = world.commands();
        for edge in edges {
            commands.entity(edge.0).try_despawn();
        }
    }

    pub fn info(&self) -> &ProcessorInfo {
        &self.info
    }

    pub fn resolve_port_identifier(&self, identifier: &PortIdentifier) -> Option<usize> {
        match identifier {
            &PortIdentifier::Index(index) => {
                if index < self.info.ports.len() {
                    Some(index)
                } else {
                    None
                }
            }
            PortIdentifier::Name(name) => self.port_name_to_index(name.as_ref()),
        }
    }

    pub fn port_name_to_index(&self, name: &str) -> Option<usize> {
        self.info.ports.iter().position(|port| {
            port.name
                .as_ref()
                .is_some_and(|port_name| port_name.as_ref() == name)
        })
    }

    pub fn new(factory: Arc<ProcessorFactory>) -> Self {
        Self {
            info: factory().info(),
            processor_factory: factory,
            edges: Vec::new(),
        }
    }

    pub fn make_processor(&self) -> Box<dyn Processor> {
        (self.processor_factory)()
    }
}

impl core::fmt::Debug for AudioGraphNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioGraphNode")
            .field("info", &self.info)
            .field("edges", &self.edges)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Component)]
#[component(immutable, on_insert, on_remove)]
pub struct AudioGraphEdge {
    pub src: AudioGraphNodeId,
    pub src_port: usize,
    pub dst: AudioGraphNodeId,
    pub dst_port: usize,
}

impl AudioGraphEdge {
    fn on_remove(mut world: DeferredWorld, ctx: HookContext) {
        // Remove the edge to the destination and source nodes.
        let edge = world.get::<AudioGraphEdge>(ctx.entity).unwrap();
        let src = edge.src;
        let dst = edge.dst;

        fn try_erase<T: PartialEq>(v: &mut Vec<T>, item: &T) {
            if let Some(pos) = v.iter().position(|x| x == item) {
                v.swap_remove(pos);
            }
        }

        let mut src = world.get_mut::<AudioGraphNode>(src.0).unwrap();
        try_erase(
            &mut src.bypass_change_detection().edges,
            &AudioGraphEdgeId(ctx.entity),
        );

        let mut dst = world.get_mut::<AudioGraphNode>(dst.0).unwrap();
        try_erase(
            &mut dst.bypass_change_detection().edges,
            &AudioGraphEdgeId(ctx.entity),
        );
    }

    fn on_insert(mut world: DeferredWorld, ctx: HookContext) {
        // Add the edge to the destination and source nodes.
        let edge = world.get::<AudioGraphEdge>(ctx.entity).unwrap();
        let src = edge.src;
        let dst = edge.dst;

        let mut src = world.get_mut::<AudioGraphNode>(src.0).unwrap();
        src.bypass_change_detection()
            .edges
            .push(AudioGraphEdgeId(ctx.entity));

        let mut dst = world.get_mut::<AudioGraphNode>(dst.0).unwrap();
        dst.bypass_change_detection()
            .edges
            .push(AudioGraphEdgeId(ctx.entity));
    }
}

#[derive(Debug, Clone, Copy, Component)]
#[require(
    AudioGraphNode::new(Arc::new(|| Box::new(processors::discard::<AudioBuf<f32, 2>>()))),
    Name::new("AudioGraphOutput")
)]
pub struct AudioGraphOutput;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PortIdentifier {
    Index(usize),
    Name(Cow<'static, str>),
}

impl From<usize> for PortIdentifier {
    fn from(index: usize) -> Self {
        PortIdentifier::Index(index)
    }
}

impl From<&'static str> for PortIdentifier {
    fn from(name: &'static str) -> Self {
        PortIdentifier::Name(Cow::Borrowed(name))
    }
}

impl From<String> for PortIdentifier {
    fn from(name: String) -> Self {
        PortIdentifier::Name(Cow::Owned(name))
    }
}

impl std::fmt::Display for PortIdentifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortIdentifier::Index(index) => std::fmt::Display::fmt(&index, f),
            PortIdentifier::Name(name) => std::fmt::Display::fmt(name.as_ref(), f),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioGraphNodeId(Entity);

impl AudioGraphNodeId {
    pub fn from_entity(entity: Entity) -> Self {
        Self(entity)
    }

    pub fn get(self) -> Entity {
        self.0
    }
}

impl From<Entity> for AudioGraphNodeId {
    fn from(entity: Entity) -> Self {
        Self(entity)
    }
}

impl From<AudioGraphNodeId> for Entity {
    fn from(id: AudioGraphNodeId) -> Self {
        id.0
    }
}

impl std::fmt::Display for AudioGraphNodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioGraphEdgeId(Entity);

impl AudioGraphEdgeId {
    pub fn from_entity(entity: Entity) -> Self {
        Self(entity)
    }

    pub fn get(self) -> Entity {
        self.0
    }
}

impl From<Entity> for AudioGraphEdgeId {
    fn from(entity: Entity) -> Self {
        Self(entity)
    }
}

impl From<AudioGraphEdgeId> for Entity {
    fn from(id: AudioGraphEdgeId) -> Self {
        id.0
    }
}

impl std::fmt::Display for AudioGraphEdgeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

pub trait CommandsExt {
    fn insert_audio_graph_node<F, P>(&mut self, node: F) -> AudioGraphNodeId
    where
        F: Send + Sync + 'static + Fn() -> P,
        P: Processor;

    // FIXME: Return the `AudioGraphEdgeId`.
    fn connect_audio_graph_nodes(
        &mut self,
        src: impl Into<AudioGraphNodeId>,
        src_port: impl Into<PortIdentifier>,
        dst: impl Into<AudioGraphNodeId>,
        dst_port: impl Into<PortIdentifier>,
    );
}

impl CommandsExt for Commands<'_, '_> {
    fn insert_audio_graph_node<F, P>(&mut self, factory: F) -> AudioGraphNodeId
    where
        F: Send + Sync + 'static + Fn() -> P,
        P: Processor,
    {
        self.spawn(AudioGraphNode::new(Arc::new(move || Box::new(factory()))))
            .id()
            .into()
    }

    fn connect_audio_graph_nodes(
        &mut self,
        src: impl Into<AudioGraphNodeId>,
        src_port: impl Into<PortIdentifier>,
        dst: impl Into<AudioGraphNodeId>,
        dst_port: impl Into<PortIdentifier>,
    ) {
        self.queue(connect_audio_graph_nodes_command(
            src.into(),
            src_port.into(),
            dst.into(),
            dst_port.into(),
        ));
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ConnectAudioGraphNodesError {
    #[error("Source node `{0}` not found")]
    SourceNodeNotFound(AudioGraphNodeId),
    #[error("Destination node `{0}` not found")]
    DestinationNodeNotFound(AudioGraphNodeId),
    #[error("Source port `{0}@{1}` not found")]
    SourcePortNotFound(PortIdentifier, AudioGraphNodeId),
    #[error("Destination port `{0}@{1}` not found")]
    DestinationPortNotFound(PortIdentifier, AudioGraphNodeId),
    #[error(
        "Source port `{src_port}@{src_node}` and destination port `{dst_port}@{dst_node}` \
        do not have the same type ({src_port_type:?} != {dst_port_type:?})"
    )]
    IncompatiblePortTypes {
        src_node: AudioGraphNodeId,
        src_port: PortIdentifier,
        src_port_type: PortTypeId,
        dst_node: AudioGraphNodeId,
        dst_port: PortIdentifier,
        dst_port_type: PortTypeId,
    },
    #[error("Connecting `{0}` to `{1}` would introduce a cycle in the graph")]
    CycleDetected(AudioGraphNodeId, AudioGraphNodeId),
    #[error("Source port `{0}@{1}` is not an output port")]
    SourcePortNotOutput(PortIdentifier, AudioGraphNodeId),
    #[error("Destination port `{0}@{1}` is not an input port")]
    DestinationPortNotInput(PortIdentifier, AudioGraphNodeId),
}

fn connect_audio_graph_nodes_command(
    src: AudioGraphNodeId,
    src_port: PortIdentifier,
    dst: AudioGraphNodeId,
    dst_port: PortIdentifier,
) -> impl Command<Result<(), ConnectAudioGraphNodesError>> {
    move |world: &mut World| -> Result<(), ConnectAudioGraphNodesError> {
        let src_node = world
            .get::<AudioGraphNode>(src.0)
            .ok_or(ConnectAudioGraphNodesError::SourceNodeNotFound(src))?;

        let dst_node = world
            .get::<AudioGraphNode>(dst.0)
            .ok_or(ConnectAudioGraphNodesError::DestinationNodeNotFound(dst))?;

        // FIXME: Check for cycles properly.
        if src == dst {
            return Err(ConnectAudioGraphNodesError::CycleDetected(src, dst));
        }

        let Some(src_port_idx) = src_node.resolve_port_identifier(&src_port) else {
            return Err(ConnectAudioGraphNodesError::SourcePortNotFound(
                src_port, src,
            ));
        };

        let Some(dst_port_idx) = dst_node.resolve_port_identifier(&dst_port) else {
            return Err(ConnectAudioGraphNodesError::DestinationPortNotFound(
                dst_port, dst,
            ));
        };

        let src_port_info = &src_node.info.ports[src_port_idx];
        let dst_port_info = &dst_node.info.ports[dst_port_idx];

        if src_port_info.direction != PortDirection::Output {
            return Err(ConnectAudioGraphNodesError::SourcePortNotOutput(
                src_port, src,
            ));
        }

        if dst_port_info.direction != PortDirection::Input {
            return Err(ConnectAudioGraphNodesError::DestinationPortNotInput(
                dst_port, dst,
            ));
        }

        if dst_port_info.type_id != src_port_info.type_id {
            return Err(ConnectAudioGraphNodesError::IncompatiblePortTypes {
                src_node: src,
                src_port,
                src_port_type: src_port_info.type_id,
                dst_node: dst,
                dst_port,
                dst_port_type: dst_port_info.type_id,
            });
        }

        world.spawn(AudioGraphEdge {
            src,
            src_port: src_port_idx,
            dst,
            dst_port: dst_port_idx,
        });

        Ok(())
    }
}

pub(super) fn should_rebuild_audio_graph(
    nodes: Query<(), Changed<AudioGraphNode>>,
    edges: Query<(), Changed<AudioGraphEdge>>,
    master: Option<Single<(), With<AudioGraphOutput>>>,
    driver: Option<Res<AudioThreadHandle>>,
) -> bool {
    if master.is_none() || driver.is_none() {
        return false;
    }

    nodes.into_iter().next().is_some()
        || edges.into_iter().next().is_some()
        || driver.is_some_and(|driver| driver.is_added())
}

pub(super) fn rebuild_audio_graph(
    mut nodes: Query<(Entity, &AudioGraphNode)>,
    mut edges: Query<&AudioGraphEdge>,
    master: Single<Entity, With<AudioGraphOutput>>,
    mut driver: ResMut<AudioThreadHandle>,
) {
    let master_entity = *master;

    let mut builder = AudioGraphRunner::builder();
    let mut entity_to_node_id = EntityHashMap::new();

    for (node_entity, node) in nodes.iter_mut() {
        let id = builder.insert(node.make_processor());

        // SAFETY: The query only returns entities at most once.
        unsafe { entity_to_node_id.insert_unique_unchecked(node_entity, id) };
    }

    let mut master_output = usize::MAX;

    for edge in edges.iter_mut() {
        let src = *entity_to_node_id.get(&edge.src.0).unwrap();
        let dst = *entity_to_node_id.get(&edge.dst.0).unwrap();

        let edge_id = builder.connect(src, edge.src_port, dst, edge.dst_port);

        if master_entity == edge.dst.0 {
            debug_assert!(master_output == usize::MAX);
            master_output = edge_id;
            builder.make_external(edge_id);
        }
    }

    // The audio graph is not connected to the master output.
    if master_output == usize::MAX {
        return;
    }

    bevy_log::trace!(
        "Re-built audio graph ({} nodes, {} edges)",
        builder.node_count(),
        builder.edge_count(),
    );

    driver.set_audio_graph_runner(builder, master_output);
}
