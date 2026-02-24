#![cfg_attr(not(feature = "std"), no_std)]

use {bevy_app::prelude::*, bevy_ecs::prelude::*};

extern crate alloc;

pub mod audio_graph;
pub mod audio_thread_driver;
pub mod gc;
pub mod processors;

pub mod entity;

pub mod prelude {
    pub use crate::entity::{AudioGraphOutput, CommandsExt as _};
}

#[derive(Debug, Default)]
pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Last,
            (
                collect_garbage_system,
                self::entity::rebuild_audio_graph.run_if(self::entity::should_rebuild_audio_graph),
            ),
        );
    }
}

fn collect_garbage_system() {
    self::gc::GLOBAL.collect_garbage();
}
