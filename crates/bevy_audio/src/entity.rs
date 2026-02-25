use {
    crate::{AudioFile, Source, formats::Decoder},
    bevy_asset::prelude::*,
    bevy_audio_core::{
        entity::{AudioGraphEdge, AudioGraphNode, AudioGraphNodeId, AudioGraphOutput},
        processors::{self, ExternalSourceMixerHandle},
    },
    bevy_ecs::{lifecycle::HookContext, prelude::*, world::DeferredWorld},
    std::{
        io::Cursor,
        sync::{Arc, PoisonError, RwLock},
    },
};

struct AudioManagerState {
    mixer: Option<ExternalSourceMixerHandle<2>>,
}

// TODO: Implement a proper bus manager.
#[derive(Resource)]
pub struct AudioManager {
    state: Arc<RwLock<AudioManagerState>>,
    output: AudioGraphNodeId,
}

impl FromWorld for AudioManager {
    fn from_world(world: &mut World) -> Self {
        let state = Arc::new(RwLock::new(AudioManagerState { mixer: None }));

        let state2 = state.clone();
        let output: AudioGraphNodeId = world
            .spawn(AudioGraphNode::new(Arc::new(move || {
                let (processor, handle) = processors::external_source_mixer(16, 512);
                state2.write().unwrap_or_else(PoisonError::into_inner).mixer = Some(handle);
                Box::new(processor)
            })))
            .id()
            .into();

        let master = world.spawn(AudioGraphOutput).id().into();

        world.spawn(AudioGraphEdge {
            src: output,
            src_port: 0,
            dst: master,
            dst_port: 0,
        });

        Self { state, output }
    }
}

impl AudioManager {
    pub fn output_node(&self) -> AudioGraphNodeId {
        self.output
    }
}

#[derive(Component)]
#[component(on_remove)]
pub struct StereoAudioPlayer(pub Handle<AudioFile>);

impl StereoAudioPlayer {
    fn on_remove(mut world: DeferredWorld, ctx: HookContext) {
        world
            .commands()
            .entity(ctx.entity)
            .try_remove::<StereoAudioPlayerHandle>();
    }
}

pub(super) fn start_audio_players_system(
    mut commands: Commands,
    assets: Res<Assets<AudioFile>>,
    mut unstarted_players: Query<(Entity, &StereoAudioPlayer), Without<StereoAudioPlayerHandle>>,
    audio_manager: Res<AudioManager>,
) {
    for (player_entity, player) in unstarted_players.iter_mut() {
        let Some(file) = assets.get(&player.0) else {
            // Not yet loaded or invalid handle.
            return;
        };

        let Some(mut voice) = audio_manager
            .state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .mixer
            .as_ref()
            .and_then(|x| x.spawn_voice().ok())
        else {
            // Voice error or audio graph not built.
            return;
        };

        let Ok(mut decoder) = Decoder::new(Cursor::new(file.0.clone())) else {
            return;
        };

        // TODO: Handle mono audio files.
        assert!(decoder.channels() == 2);

        let task = bevy_tasks::AsyncComputeTaskPool::get().spawn(async move {
            loop {
                while voice.can_send() == 0 {
                    bevy_tasks::futures_lite::future::yield_now().await;
                }

                let sent = voice.feed_iter(std::iter::from_fn(|| {
                    if let Some(l) = decoder.next()
                        && let Some(r) = decoder.next()
                    {
                        Some([l, r])
                    } else {
                        None
                    }
                }));

                if sent == 0 {
                    break;
                }
            }
        });

        commands
            .entity(player_entity)
            .insert(StereoAudioPlayerHandle { _task: task });
    }
}

// FIXME: Delete the entity once it is done playing.

#[derive(Component)]
pub struct StereoAudioPlayerHandle {
    _task: bevy_tasks::Task<()>,
}
