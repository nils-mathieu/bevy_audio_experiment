use {bevy_app::prelude::*, std::time::Duration};

pub mod audio_thread;

#[derive(Debug)]
pub struct CpalPlugin {
    pub desired_buffer_latency: Duration,
    pub desired_sample_rate: cpal::SampleRate,
}

impl Default for CpalPlugin {
    fn default() -> Self {
        Self {
            desired_buffer_latency: Duration::from_millis(15),
            desired_sample_rate: 44100,
        }
    }
}

impl Plugin for CpalPlugin {
    fn build(&self, app: &mut App) {
        match self::audio_thread::initialize_default(
            self.desired_buffer_latency,
            self.desired_sample_rate,
        ) {
            Ok((stream, handle)) => {
                app.insert_resource(stream);
                app.insert_resource(handle);
            }
            Err(err) => bevy_log::error!("Failed to initialize the audio thread: {err}"),
        }
    }
}
