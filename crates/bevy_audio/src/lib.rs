mod source;
pub use self::source::*;

mod entity;
pub use self::entity::*;

mod audio_file;
pub use self::audio_file::*;

use {bevy_app::prelude::*, bevy_asset::prelude::*};

pub mod formats;

#[derive(Debug, Default)]
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<bevy_audio_core::AudioPlugin>() {
            app.add_plugins(bevy_audio_core::AudioPlugin);
        }

        #[cfg(feature = "cpal")]
        if !app.is_plugin_added::<bevy_audio_cpal::CpalPlugin>() {
            app.add_plugins(bevy_audio_cpal::CpalPlugin::default());
        }

        app.init_asset_loader::<AudioFileLoader>()
            .init_asset::<AudioFile>()
            .init_resource::<AudioManager>()
            .add_systems(Last, start_audio_players_system);
    }
}
