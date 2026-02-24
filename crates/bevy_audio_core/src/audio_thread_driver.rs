use {
    crate::{
        audio_graph::{AudioBuf, AudioGraphBuilder, AudioGraphRunner, EdgeId, RunCtx, SetupCtx},
        gc::GcBox,
    },
    bevy_ecs::prelude::*,
    bevy_platform::cell::SyncCell,
};

enum Command {
    SetAudioGraphRunner {
        new_runner: GcBox<AudioGraphRunner>,
        master_output: EdgeId,
    },
}

#[derive(Resource)]
pub struct AudioThreadHandle {
    sample_rate: u32,
    max_buffer_size: usize,
    command_sender: SyncCell<rtrb::Producer<Command>>,
}

impl AudioThreadHandle {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn max_buffer_size(&self) -> usize {
        self.max_buffer_size
    }

    pub fn set_audio_graph_runner(&mut self, runner: AudioGraphBuilder, master_output: EdgeId) {
        let runner = runner.build(&mut SetupCtx {
            max_sample_count: self.max_buffer_size,
            sample_rate: self.sample_rate,
            sample_rate_recip: (self.sample_rate as f64).recip(),
        });

        let result = self
            .command_sender
            .get()
            .push(Command::SetAudioGraphRunner {
                new_runner: GcBox::new(runner),
                master_output,
            });

        if result.is_err() {
            bevy_log::warn!(
                "Failed to send `SetAudioGraphRunner` command - too many commands in queue"
            );
        }
    }
}

pub struct AudioThreadDriver {
    command_receiver: rtrb::Consumer<Command>,
    audio_graph: GcBox<AudioGraphRunner>,
    master_output: EdgeId,
}

impl AudioThreadDriver {
    pub fn audio_graph(&self) -> &AudioGraphRunner {
        &self.audio_graph
    }

    pub fn audio_graph_mut(&mut self) -> &mut AudioGraphRunner {
        &mut self.audio_graph
    }

    pub fn run(&mut self, ctx: &mut RunCtx) {
        let commands = self
            .command_receiver
            .read_chunk(self.command_receiver.slots())
            .unwrap();

        for cmd in commands {
            match cmd {
                Command::SetAudioGraphRunner {
                    new_runner,
                    master_output,
                } => {
                    self.audio_graph = new_runner;
                    self.master_output = master_output;
                }
            }
        }

        self.audio_graph.run(ctx);
    }

    pub fn get_output(&self) -> Option<&AudioBuf<f32, 2>> {
        self.audio_graph
            .get_external_edge(self.master_output)
            .and_then(|data| data.downcast_ref())
    }
}

pub fn create(max_buffer_size: usize, sample_rate: u32) -> (AudioThreadHandle, AudioThreadDriver) {
    let (command_sender, command_receiver) = rtrb::RingBuffer::new(8);

    let handle = AudioThreadHandle {
        max_buffer_size,
        sample_rate,
        command_sender: SyncCell::new(command_sender),
    };

    let driver = AudioThreadDriver {
        command_receiver,
        audio_graph: GcBox::new(AudioGraphRunner::default()),
        master_output: 0,
    };

    (handle, driver)
}
