use {
    bevy_app::prelude::*, bevy_asset::*, bevy_audio::StereoAudioPlayer, bevy_ecs::prelude::*,
    std::time::Duration,
};

pub fn main() {
    App::new()
        .add_plugins((
            bevy_log::LogPlugin {
                level: bevy_log::Level::TRACE,
                ..Default::default()
            },
            bevy_asset::AssetPlugin::default(),
            bevy_audio::AudioPlugin,
        ))
        .add_systems(Startup, initialize)
        .set_runner(examples::run_for(
            Duration::from_millis(200),
            Duration::from_secs(6),
        ))
        .run();
}

fn initialize(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn(StereoAudioPlayer(assets.load("guitar.wav")));
}
