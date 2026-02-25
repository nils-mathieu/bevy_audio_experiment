use {
    crate::audio_graph::{
        AudioBuf, PortDataRaw, PortDescription, PortDirection, PortType, Processor, ProcessorInfo,
        RunCtx, SetupCtx,
    },
    alloc::{borrow::Cow, sync::Arc},
    bevy_platform::{
        cell::SyncCell,
        sync::{Mutex, PoisonError},
    },
    core::{hint::assert_unchecked, ops::DerefMut},
    std::mem::ManuallyDrop,
};

// TODO: Voice priority.

pub fn external_source_mixer<const C: usize>(
    max_voices: usize,
    buffer_size: usize,
) -> (ExternalSourceMixer<C>, ExternalSourceMixerHandle<C>) {
    let (command_producer, command_consumer) = rtrb::RingBuffer::new(16);

    let mut producers = Vec::with_capacity(max_voices);
    let mut consumers = Vec::with_capacity(max_voices);
    for i in 0..max_voices {
        let (producer, consumer) = rtrb::RingBuffer::new(buffer_size);
        producers.push(Some(AvailableVoice {
            producer,
            next_available: i + 1,
        }));
        consumers.push(VoiceState {
            consumer,
            stopping: false,
        });
    }

    let handle_state = Arc::new(Mutex::new(ExternalSourceMixerHandleState {
        voices: producers.into_boxed_slice(),
        next_available: 0,
        commands: command_producer,
    }));

    (
        ExternalSourceMixer {
            voices: consumers.into_boxed_slice(),
            playing: Vec::with_capacity(max_voices),
            commands: command_consumer,
        },
        ExternalSourceMixerHandle(handle_state),
    )
}

enum ExternalSourceMixerCommand {
    StartVoice(usize),
    StopVoice(usize),
}

struct AvailableVoice<const C: usize> {
    producer: rtrb::Producer<[f32; C]>,
    /// # Invariants
    ///
    /// If valid index into [`ExternalSourceMixerHandleState::voices`], then points to a `Some(_)`
    /// value.
    ///
    /// If invalid index, then no more voices are available after this voice in the linked list.
    next_available: usize,
}

struct ExternalSourceMixerHandleState<const C: usize> {
    voices: Box<[Option<AvailableVoice<C>>]>,
    /// # Invariants
    ///
    /// If valid index into [`ExternalSourceMixerHandleState::voices`], then points to a `Some(_)`
    /// value.
    ///
    /// If invalid index, then no more voices are available.
    next_available: usize,
    commands: rtrb::Producer<ExternalSourceMixerCommand>,
}

pub struct ExternalSourceMixerVoice<const C: usize> {
    producer: ManuallyDrop<SyncCell<rtrb::Producer<[f32; C]>>>,
    state: Arc<Mutex<ExternalSourceMixerHandleState<C>>>,
    /// # Invariants
    ///
    /// Is a valid index into [`ExternalSourceMixerHandleState::voices`], points to a `None` value.
    index: usize,
}

impl<const C: usize> ExternalSourceMixerVoice<C> {
    /// # Returns
    ///
    /// Returns whether the frame was successfully sent.
    pub fn feed(&mut self, frame: [f32; C]) -> bool {
        self.producer.get().push(frame).is_ok()
    }

    /// # Returns
    ///
    /// Returns the number of items sent.
    pub fn feed_iter(&mut self, frames: impl IntoIterator<Item = [f32; C]>) -> usize {
        let producer = self.producer.get();
        producer
            .write_chunk_uninit(producer.slots())
            .unwrap()
            .fill_from_iter(frames)
    }
}

impl<const C: usize> Drop for ExternalSourceMixerVoice<C> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let next_available = state.next_available;

        // SAFETY: By invariant, we know that `self.index` is a valid index into `state.voices`.
        let slot = unsafe { state.voices.get_unchecked_mut(self.index) };

        // SAFETY: By invariant, we know that `self.index` points to a `None` value.
        // This check ensures that the compiler can remove the `Some(_)` case where the slot is
        // already occupied and a whole voice needs to be dropped.
        unsafe { assert_unchecked(slot.is_none()) };

        // SAFETY: We're dropping the voice, ensuring that nobody is touching this field again.
        let producer = unsafe { ManuallyDrop::take(&mut self.producer) };

        *slot = Some(AvailableVoice {
            producer: SyncCell::to_inner(producer),
            next_available,
        });

        // Update the head of the free list to the slot we just provided.
        state.next_available = self.index;

        // Notify the audio thread that the voice will no longer produce audio.
        let result = state
            .commands
            .push(ExternalSourceMixerCommand::StopVoice(self.index));

        if result.is_err() {
            bevy_log::warn!(
                "Failed to notify audio thread that voice {} will no longer produce audio - the audio thread is likely running behind",
                self.index,
            );
        }
    }
}

pub struct ExternalSourceMixerHandle<const C: usize>(Arc<Mutex<ExternalSourceMixerHandleState<C>>>);

#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum SpawnVoiceError {
    #[error("The audio thread is running behind and cannot accept more commands")]
    AudioThreadBehind,
    #[error("No voice is available")]
    NoAvailableVoice,
}

impl<const C: usize> ExternalSourceMixerHandle<C> {
    fn state(&self) -> impl DerefMut<Target = ExternalSourceMixerHandleState<C>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn spawn_voice(&self) -> Result<ExternalSourceMixerVoice<C>, SpawnVoiceError> {
        let mut state = self.state();

        let index = state.next_available;

        let maybe_voice = state
            .voices
            .get_mut(index)
            .ok_or(SpawnVoiceError::NoAvailableVoice)?
            .take();

        // SAFETY: It is a type invariant that `next_available` is a valid index into `voices`.
        let voice = unsafe { maybe_voice.unwrap_unchecked() };

        state.next_available = voice.next_available;

        // Notify the audio thread that a new voice is now running.
        let result = state
            .commands
            .push(ExternalSourceMixerCommand::StartVoice(index));

        if result.is_err() {
            // The audio thread is probably running way behind.

            // SAFETY: We checked `index` already at the beginning of the function.
            unsafe { *state.voices.get_unchecked_mut(index) = Some(voice) };

            // Restore the previous next available state to the restored voice.
            state.next_available = index;

            return Err(SpawnVoiceError::AudioThreadBehind);
        }

        Ok(ExternalSourceMixerVoice {
            index,
            producer: ManuallyDrop::new(SyncCell::new(voice.producer)),
            state: self.0.clone(),
        })
    }
}

struct VoiceState<const C: usize> {
    consumer: rtrb::Consumer<[f32; C]>,
    /// Indicates that the voice is stopping, but is waiting to be emptied.
    stopping: bool,
}

pub struct ExternalSourceMixer<const C: usize> {
    voices: Box<[VoiceState<C>]>,
    /// # Invariants
    ///
    /// Contains valid indices within `voices`.
    playing: Vec<usize>,
    commands: rtrb::Consumer<ExternalSourceMixerCommand>,
}

impl<const C: usize> ExternalSourceMixer<C> {
    fn playing_voices_mut(&mut self) -> impl Iterator<Item = &mut VoiceState<C>> {
        let voices_ptr = self.voices.as_mut_ptr();
        self.playing
            .iter()
            .map(move |&idx| unsafe { &mut *voices_ptr.add(idx) })
    }
}

impl<const C: usize> Processor for ExternalSourceMixer<C>
where
    AudioBuf<f32, C>: PortType,
{
    fn info(&self) -> ProcessorInfo {
        ProcessorInfo {
            ports: vec![PortDescription {
                name: Some(Cow::Borrowed("output")),
                direction: PortDirection::Output,
                type_id: <AudioBuf<f32, C>>::PORT_TYPE_ID,
            }],
        }
    }

    fn setup(&mut self, _ctx: &mut SetupCtx) {}

    unsafe fn run(&mut self, ctx: &mut RunCtx, ports: &[Option<PortDataRaw>]) {
        // Handle commands after processing data because we want to make sure we saw everything
        // that was sent to the mixer.
        for cmd in self.commands.read_chunk(self.commands.slots()).unwrap() {
            match cmd {
                ExternalSourceMixerCommand::StartVoice(idx) => {
                    debug_assert!(idx < self.voices.len());

                    // NOTE: This `!contains` call will almost always succeed. It is only here to
                    // handle the unfortunate case of the audio thread missing a previous
                    // `StopVoice` command.
                    if !self.playing.contains(&idx) {
                        // SAFETY: We know that the `playing` vector can store all indices within
                        // `voices` if needed. The handle must prevent starting a given voice twice.
                        unsafe { assert_unchecked(self.playing.len() < self.playing.capacity()) };
                        self.playing.push(idx);

                        // SAFETY: The handle must provide a valid index.
                        unsafe { self.voices.get_unchecked_mut(idx).stopping = false };
                    }
                }
                ExternalSourceMixerCommand::StopVoice(idx) => {
                    debug_assert!(idx < self.voices.len());

                    // NOTE: This `contains` call will almost always succeed. It is only here to
                    // handle the unfortunate case of the audio thread missing a previous
                    // `StopVoice` command.
                    if self.playing.contains(&idx) {
                        // SAFETY: The handle must provide a valid index.
                        unsafe { self.voices.get_unchecked_mut(idx).stopping = true };
                    }
                }
            }
        }

        // SAFETY: The caller must respect the port layout we requested.
        let output = unsafe { ports.get_unchecked(0) };

        if let Some(output) = output {
            // SAFETY: The caller must respect the port layout we requested.
            let output = unsafe { output.downcast_mut_unchecked::<AudioBuf<f32, C>>() };

            // SAFETY: The provided audio buffers must be at least as large as `ctx.sample_count`.
            unsafe { output.clear_to_unchecked(ctx.sample_count) };
            output.set_silent(true);

            for voice in self.playing_voices_mut() {
                let to_consume = ctx.sample_count.min(voice.consumer.slots());
                let data = voice.consumer.read_chunk(to_consume).unwrap();

                if data.is_empty() {
                    continue;
                }

                output.set_silent(false);

                for (dst, src) in output
                    .frames_mut()
                    .zip(data.into_iter().chain(core::iter::repeat([0.0; C])))
                    .take(ctx.sample_count)
                {
                    for (dst, src) in dst.into_iter().zip(src) {
                        *dst += src;
                    }
                }
            }
        } else {
            // Consume all data even if the output is not connected.
            for voice in self.playing_voices_mut() {
                voice
                    .consumer
                    .read_chunk(voice.consumer.slots().min(ctx.sample_count))
                    .unwrap()
                    .commit_all();
            }
        }

        // Remove voices that are currently stopping and are empty.
        self.playing.retain(|&idx| {
            // SAFETY: The indices in `playing` are known to be valid.
            let voice = unsafe { self.voices.get_unchecked_mut(idx) };
            !voice.stopping || !voice.consumer.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::{audio_graph::AudioGraphRunner, testing};

    #[test]
    fn external_source_mixer_single_voice() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(4, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([0.0, 1.0, 2.0, 3.0, 4.0])));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(5));

        let mut voice = handle.spawn_voice().unwrap();
        voice.feed_iter([0.0, 1.0, 2.0, 3.0, 4.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(5));
    }

    #[test]
    fn external_source_mixer_multiple_voices() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(4, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([3.0, 6.0, 9.0])));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(3));

        let mut voice1 = handle.spawn_voice().unwrap();
        let mut voice2 = handle.spawn_voice().unwrap();
        let mut voice3 = handle.spawn_voice().unwrap();

        voice1.feed_iter([1.0, 2.0, 3.0].into_iter().map(|x| [x]));
        voice2.feed_iter([1.0, 2.0, 3.0].into_iter().map(|x| [x]));
        voice3.feed_iter([1.0, 2.0, 3.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(3));
    }

    #[test]
    fn external_source_mixer_no_voices() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, _handle) = super::external_source_mixer::<1>(4, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_silent()));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(5));
        runner.run(&mut testing::run_ctx(5));
    }

    #[test]
    fn external_source_mixer_voice_dropped() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(4, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([1.0, 2.0, 0.0, 0.0])));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(4));

        let mut voice = handle.spawn_voice().unwrap();
        voice.feed_iter([1.0, 2.0].into_iter().map(|x| [x]));
        drop(voice);

        runner.run(&mut testing::run_ctx(4));
    }

    #[test]
    fn external_source_mixer_voice_underrun() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(4, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([0.0, 1.0, 2.0, 0.0, 0.0])));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(5));

        let mut voice = handle.spawn_voice().unwrap();
        voice.feed_iter([0.0, 1.0, 2.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(5));
    }

    #[test]
    fn external_source_mixer_max_voices() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(2, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([2.0, 2.0])));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(2));

        let mut voice1 = handle.spawn_voice().unwrap();
        let mut voice2 = handle.spawn_voice().unwrap();
        assert!(matches!(
            handle.spawn_voice(),
            Err(super::SpawnVoiceError::NoAvailableVoice)
        ));

        voice1.feed_iter([1.0, 1.0].into_iter().map(|x| [x]));
        voice2.feed_iter([1.0, 1.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(2));
    }

    #[test]
    fn external_source_mixer_voice_reuse() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(2, 16);
        let mixer = builder.insert(Box::new(processor));
        let out = builder.insert(Box::new(testing::assert_sink([1.0, 0.0, 2.0, 0.0])));
        builder.connect(mixer, 0, out, 0);
        let mut runner = builder.build(&mut testing::setup_ctx(3));

        let mut voice1 = handle.spawn_voice().unwrap();
        voice1.feed_iter([1.0].into_iter().map(|x| [x]));
        drop(voice1);

        runner.run(&mut testing::run_ctx(2));

        let mut voice2 = handle.spawn_voice().unwrap();
        voice2.feed_iter([2.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(2));
    }

    #[test]
    fn external_source_mixer_disconnected_output() {
        let mut builder = AudioGraphRunner::builder();
        let (processor, handle) = super::external_source_mixer::<1>(4, 16);
        let _mixer = builder.insert(Box::new(processor));
        let mut runner = builder.build(&mut testing::setup_ctx(5));

        let mut voice = handle.spawn_voice().unwrap();
        voice.feed_iter([0.0, 1.0, 2.0, 3.0, 4.0].into_iter().map(|x| [x]));

        runner.run(&mut testing::run_ctx(5));
    }
}
