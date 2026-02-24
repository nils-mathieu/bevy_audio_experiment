use {
    bevy_audio_core::{
        audio_graph::RunCtx,
        audio_thread_driver::{AudioThreadDriver, AudioThreadHandle},
    },
    bevy_ecs::resource::Resource,
    cpal::{
        FromSample,
        traits::{DeviceTrait, HostTrait, StreamTrait},
    },
    std::time::Duration,
};

#[derive(Debug, thiserror::Error)]
pub enum AudioThreadError {
    #[error("No output device was found")]
    NoDevice,
    #[error("The device is not available")]
    DeviceNotAvailable,
    #[error("The device format is not supported")]
    FormatNotSupported,
    #[error("{0}")]
    Other(
        #[from]
        #[source]
        cpal::BackendSpecificError,
    ),
}

impl From<cpal::DefaultStreamConfigError> for AudioThreadError {
    #[rustfmt::skip]
    fn from(value: cpal::DefaultStreamConfigError) -> Self {
        match value {
            cpal::DefaultStreamConfigError::DeviceNotAvailable => AudioThreadError::DeviceNotAvailable,
            cpal::DefaultStreamConfigError::StreamTypeNotSupported => AudioThreadError::FormatNotSupported,
            cpal::DefaultStreamConfigError::BackendSpecific { err } => AudioThreadError::Other(err),
        }
    }
}

impl From<cpal::SupportedStreamConfigsError> for AudioThreadError {
    #[rustfmt::skip]
    fn from(value: cpal::SupportedStreamConfigsError) -> Self {
        match value {
            cpal::SupportedStreamConfigsError::DeviceNotAvailable => AudioThreadError::DeviceNotAvailable,
            cpal::SupportedStreamConfigsError::InvalidArgument => AudioThreadError::DeviceNotAvailable,
            cpal::SupportedStreamConfigsError::BackendSpecific { err } => AudioThreadError::Other(err),
        }
    }
}

impl From<cpal::BuildStreamError> for AudioThreadError {
    #[rustfmt::skip]
    fn from(value: cpal::BuildStreamError) -> Self {
        match value {
            cpal::BuildStreamError::DeviceNotAvailable => AudioThreadError::DeviceNotAvailable,
            cpal::BuildStreamError::StreamConfigNotSupported => AudioThreadError::FormatNotSupported,
            cpal::BuildStreamError::InvalidArgument => AudioThreadError::FormatNotSupported,
            cpal::BuildStreamError::StreamIdOverflow => AudioThreadError::DeviceNotAvailable,
            cpal::BuildStreamError::BackendSpecific { err } => AudioThreadError::Other(err),
        }
    }
}

impl From<cpal::PlayStreamError> for AudioThreadError {
    fn from(value: cpal::PlayStreamError) -> Self {
        match value {
            cpal::PlayStreamError::DeviceNotAvailable => AudioThreadError::DeviceNotAvailable,
            cpal::PlayStreamError::BackendSpecific { err } => AudioThreadError::Other(err),
        }
    }
}

#[derive(Resource)]
pub struct CpalStream(pub cpal::Stream);

const PREFERRED_SAMPLE_RATE: cpal::SampleRate = 44_100;

pub fn initialize_default(
    desired_buffer_latency: Duration,
    desired_sample_rate: cpal::SampleRate,
) -> Result<(CpalStream, AudioThreadHandle), AudioThreadError> {
    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .ok_or(AudioThreadError::NoDevice)?;

    initialize(&device, desired_buffer_latency, desired_sample_rate)
}

pub fn initialize(
    device: &cpal::Device,
    desired_buffer_latency: Duration,
    desired_sample_rate: cpal::SampleRate,
) -> Result<(CpalStream, AudioThreadHandle), AudioThreadError> {
    let best_config_range = device
        .supported_output_configs()?
        .max_by(|range1, range2| {
            use std::cmp::Ordering::Equal;

            const PREFERRED_CHANNEL_COUNTS: &[cpal::ChannelCount] = &[2, 1];

            for &count in PREFERRED_CHANNEL_COUNTS {
                let result = (range1.channels() == count).cmp(&(range2.channels() == count));
                if result != Equal {
                    return result;
                }
            }

            const PREFERRED_FORMATS: &[cpal::SampleFormat] = &[
                cpal::SampleFormat::F32,
                cpal::SampleFormat::F64,
                cpal::SampleFormat::I24,
                cpal::SampleFormat::U24,
                cpal::SampleFormat::I16,
                cpal::SampleFormat::U16,
                cpal::SampleFormat::I32,
                cpal::SampleFormat::U32,
                cpal::SampleFormat::I64,
                cpal::SampleFormat::U64,
            ];

            for &format in PREFERRED_FORMATS {
                let result =
                    (range1.sample_format() == format).cmp(&(range2.sample_format() == format));
                if result != Equal {
                    return result;
                }
            }

            let sample_rate_range1 = range1.min_sample_rate()..=range1.max_sample_rate();
            let sample_rate_range2 = range2.min_sample_rate()..=range2.max_sample_rate();

            let is_in_range1 = sample_rate_range1.contains(&desired_sample_rate);
            let is_in_range2 = sample_rate_range2.contains(&desired_sample_rate);
            let sample_rate_cmp = is_in_range1.cmp(&is_in_range2);
            if sample_rate_cmp != Equal {
                return sample_rate_cmp;
            }

            range1.max_sample_rate().cmp(&range2.max_sample_rate())
        })
        .ok_or(AudioThreadError::FormatNotSupported)?;

    let sample_rate = PREFERRED_SAMPLE_RATE.clamp(
        best_config_range.min_sample_rate(),
        best_config_range.max_sample_rate(),
    );

    let preferred_buffer_size =
        ((desired_buffer_latency.as_nanos() * sample_rate as u128 + 500_000_000) / 1_000_000_000)
            .try_into()
            .unwrap_or(u32::MAX);

    let driver_buffer_size = match *best_config_range.buffer_size() {
        cpal::SupportedBufferSize::Range { min, max } => preferred_buffer_size.clamp(min, max),
        cpal::SupportedBufferSize::Unknown => preferred_buffer_size,
    };

    let config = cpal::StreamConfig {
        channels: best_config_range.channels(),
        sample_rate,
        buffer_size: cpal::BufferSize::Fixed(driver_buffer_size),
    };
    let format = best_config_range.sample_format();

    let (handle, driver) = bevy_audio_core::audio_thread_driver::create(
        driver_buffer_size as usize,
        config.sample_rate,
    );

    bevy_log::info!(
        "\
        Creating output audio stream with:\n\
        - Format: {format:?}\n\
        - Buffer size: {driver_buffer_size} frames ({buffer_time:#?})\n\
        - Sample rate: {sample_rate} Hz\n\
        - Channels: {channels}\n\
        ",
        channels = config.channels,
        buffer_time =
            Duration::from_secs_f64(driver_buffer_size as f64 / config.sample_rate as f64),
    );

    let stream = match format {
        cpal::SampleFormat::I8 => build_output_stream::<i8>(device, &config, driver)?,
        cpal::SampleFormat::I16 => build_output_stream::<i16>(device, &config, driver)?,
        cpal::SampleFormat::I24 => build_output_stream::<cpal::I24>(device, &config, driver)?,
        cpal::SampleFormat::I32 => build_output_stream::<i32>(device, &config, driver)?,
        cpal::SampleFormat::I64 => build_output_stream::<i64>(device, &config, driver)?,
        cpal::SampleFormat::U8 => build_output_stream::<u8>(device, &config, driver)?,
        cpal::SampleFormat::U16 => build_output_stream::<u16>(device, &config, driver)?,
        cpal::SampleFormat::U24 => build_output_stream::<cpal::U24>(device, &config, driver)?,
        cpal::SampleFormat::U32 => build_output_stream::<u32>(device, &config, driver)?,
        cpal::SampleFormat::U64 => build_output_stream::<u64>(device, &config, driver)?,
        cpal::SampleFormat::F32 => build_output_stream::<f32>(device, &config, driver)?,
        cpal::SampleFormat::F64 => build_output_stream::<f64>(device, &config, driver)?,
        _ => return Err(AudioThreadError::FormatNotSupported),
    };

    stream.play()?;

    Ok((CpalStream(stream), handle))
}

fn build_output_stream<T: cpal::SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut driver: AudioThreadDriver,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate;
    let sample_rate_recip = (config.sample_rate as f64).recip();

    device.build_output_stream::<T, _, _>(
        config,
        move |data, _info| {
            let mut remaining = &mut *data;

            while !remaining.is_empty() {
                debug_assert!(remaining.len().is_multiple_of(channels));
                let max_to_generate = remaining.len() / channels;
                let sample_count = max_to_generate.min(driver.audio_graph().max_sample_count());

                driver.run(&mut RunCtx {
                    sample_count,
                    sample_rate,
                    sample_rate_recip,
                });

                let Some(output) = driver.get_output() else {
                    break;
                };

                // SAFETY: For the following accesses into `remaining`, we have the following:
                // sample_count * channels <= min(max_to_generate, max_sample_count) * channels <= max_to_generate * channels
                // And `max_to_generate = remaining.len() / channels`
                // so finally, `sample_count * channels <= remaining.len()`

                for channel in 0..channels {
                    for frame in 0..sample_count {
                        unsafe {
                            let dst_index = frame.unchecked_mul(channels).unchecked_add(channel);
                            let dst = remaining.get_unchecked_mut(dst_index);
                            let src = output.get_unchecked(channel, frame);

                            *dst = cpal::Sample::to_sample(*src);
                        }
                    }
                }

                remaining =
                    unsafe { remaining.get_unchecked_mut(sample_count.unchecked_mul(channels)..) };
            }

            remaining.fill(T::EQUILIBRIUM);
        },
        |err| bevy_log::error!("Stream error: {err}"),
        None,
    )
}
