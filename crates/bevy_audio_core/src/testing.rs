use crate::{
    audio_graph::{AudioBuf, Processor, RunCtx, SetupCtx},
    processors,
};

/// Creates a [`RunCtx`] with some sane defaults.
pub fn run_ctx(sample_count: usize) -> RunCtx {
    RunCtx {
        sample_count,
        sample_rate: 44100,
        sample_rate_recip: 44100f64.recip(),
    }
}

/// Creates a [`SetupCtx`] with some sane defaults.
pub fn setup_ctx(max_sample_count: usize) -> SetupCtx {
    SetupCtx {
        max_sample_count,
        sample_rate: 44100,
        sample_rate_recip: 44100f64.recip(),
    }
}

/// Creates a [`Processor`] that applies a function to each sample.
pub fn map_audio(mut f: impl 'static + Send + FnMut(f32) -> f32) -> impl Processor {
    processors::map_audio::<_, 1>(move |_: &mut RunCtx, x| f(x))
}

/// Creates a [`Processor`] that asserts its input matches the expected values from an iterator.
pub fn assert_sink<I>(expected: I) -> impl Processor
where
    I: IntoIterator<Item = f32>,
    I::IntoIter: 'static + Send + ExactSizeIterator,
{
    let mut iter = expected.into_iter();
    processors::sink_fn(move |ctx: &mut RunCtx, out: Option<&AudioBuf<f32, 1>>| {
        assert!(out.is_none_or(|buf| !buf.is_silent()));
        let actual = out
            .into_iter()
            .flat_map(|x| x.flat_iter())
            .take(ctx.sample_count);
        for (&actual, expected) in actual.zip(&mut iter) {
            assert_eq!(actual, expected);
        }
    })
}

/// Creates a [`Processor`] that asserts its input is silent.
pub fn assert_silent() -> impl Processor {
    processors::sink_fn(move |ctx: &mut RunCtx, out: Option<&AudioBuf<f32, 1>>| {
        assert!(out.is_none_or(|buf| buf.is_silent()));
        let actual = out
            .into_iter()
            .flat_map(|x| x.flat_iter())
            .take(ctx.sample_count);
        for &actual in actual {
            assert_eq!(actual, 0.0);
        }
    })
}

/// Creates a [`Processor`] that outputs the contents of the given iterator.
pub fn audio_iter<I>(iter: I) -> impl Processor
where
    I: IntoIterator<Item = f32>,
    I::IntoIter: 'static + Send,
{
    let mut iter = iter.into_iter();
    processors::audio_fn(move |_: &mut RunCtx| [iter.next().unwrap()])
}
