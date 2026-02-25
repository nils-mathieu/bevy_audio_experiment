use {
    bevy_app::prelude::*,
    bevy_audio_core::{audio_graph::Processor, prelude::*, processors},
    bevy_ecs::prelude::*,
    std::{f32::consts::TAU, time::Duration},
};

pub fn main() {
    App::new()
        .add_plugins((
            bevy_log::LogPlugin {
                level: bevy_log::Level::TRACE,
                ..Default::default()
            },
            bevy_audio_core::AudioPlugin,
            bevy_audio_cpal::CpalPlugin::default(),
        ))
        .add_systems(Startup, initialize)
        .set_runner(examples::run_for(
            Duration::from_millis(200),
            Duration::from_secs(3),
        ))
        .run();
}

fn initialize(mut commands: Commands) {
    let beeper_id = commands.insert_audio_graph_node(beeper);
    let mono_to_stereo = commands.insert_audio_graph_node(processors::mono_to_stereo);
    let master_output = commands.spawn(AudioGraphOutput).id();
    commands.connect_audio_graph_nodes(beeper_id, "output", mono_to_stereo, "input");
    commands.connect_audio_graph_nodes(mono_to_stereo, "output", master_output, "input");
}

const FREQUENCY: f32 = 440.0;
const AMPLITUDE: f32 = 0.4;

fn beeper() -> impl Processor {
    let mut phase = 0f32;
    processors::audio_fn(move |ctx| {
        let result = phase.sin() * AMPLITUDE;
        phase += FREQUENCY * ctx.sample_rate_recip as f32 * TAU;
        if phase >= TAU {
            phase -= TAU;
        }
        [result]
    })
}
