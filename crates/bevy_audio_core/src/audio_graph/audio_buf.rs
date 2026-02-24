pub struct AudioBuf<T, const C: usize> {
    // TODO: Store the frame count instead of the total sample count.
    data: Box<[T]>,
}

impl<T, const C: usize> AudioBuf<T, C> {
    pub fn with_capacity(capacity: usize, fill: impl FnMut() -> T) -> Self {
        Self {
            data: std::iter::repeat_with(fill)
                .take(capacity.checked_mul(C).expect("Capacity overflow"))
                .collect(),
        }
    }

    pub fn frame_count(&self) -> usize {
        debug_assert!(self.data.len().is_multiple_of(C));
        self.data.len() / C
    }

    pub fn as_slice(&self) -> &[T] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [T] {
        &mut self.data
    }

    /// # Safety
    ///
    /// `channel` must be less than `C`.
    pub unsafe fn channel_unchecked(&self, channel: usize) -> &[T] {
        debug_assert!(channel < C);
        let frame_count = self.frame_count();

        // SAFETY: Caller must ensure that the channel index is within bounds.
        unsafe {
            self.data.get_unchecked(
                channel.unchecked_mul(frame_count)
                    ..channel.unchecked_add(1).unchecked_mul(frame_count),
            )
        }
    }

    /// # Safety
    ///
    /// `channel` must be less than `C`.
    pub unsafe fn channel_unchecked_mut(&mut self, channel: usize) -> &mut [T] {
        debug_assert!(channel < C);
        let frame_count = self.frame_count();

        // SAFETY: Caller must ensure that the channel index is within bounds.
        unsafe {
            self.data.get_unchecked_mut(
                channel.unchecked_mul(frame_count)
                    ..channel.unchecked_add(1).unchecked_mul(frame_count),
            )
        }
    }

    /// # Safety
    ///
    /// * `channel` must be less than `C`.
    /// * `frame` must be less than `self.frame_count()`.
    #[track_caller]
    pub unsafe fn get_unchecked(&self, channel: usize, frame: usize) -> &T {
        debug_assert!(channel < C);
        debug_assert!(frame < self.frame_count());

        // SAFETY: Caller must ensure that the channel index and frame index are within bounds.
        unsafe { self.channel_unchecked(channel).get_unchecked(frame) }
    }

    /// # Safety
    ///
    /// * `channel` must be less than `C`.
    /// * `frame` must be less than `self.frame_count()`.
    #[track_caller]
    pub unsafe fn get_unchecked_mut(&mut self, channel: usize, frame: usize) -> &mut T {
        debug_assert!(channel < C);
        debug_assert!(frame < self.frame_count());

        // SAFETY: Caller must ensure that the channel index and frame index are within bounds.
        unsafe { self.channel_unchecked_mut(channel).get_unchecked_mut(frame) }
    }

    pub fn frames(&self) -> impl Iterator<Item = [&T; C]> {
        let p = self.data.as_ptr();
        let frames = self.frame_count();
        (0..frames).map(move |frame| {
            core::array::from_fn(|channel| unsafe { get_unchecked(p, frames, channel, frame) })
        })
    }

    pub fn frames_mut(&mut self) -> impl Iterator<Item = [&mut T; C]> {
        let p = self.data.as_mut_ptr();
        let frames = self.frame_count();
        (0..frames).map(move |frame| {
            core::array::from_fn(move |channel| unsafe {
                get_unchecked_mut(p, frames, channel, frame)
            })
        })
    }

    pub fn interleaved(&self) -> impl Iterator<Item = &T> {
        let p = self.data.as_ptr();
        let frames = self.frame_count();
        (0..frames).flat_map(move |frame| {
            (0..C).map(move |channel| unsafe { get_unchecked(p, frames, channel, frame) })
        })
    }

    pub fn interleaved_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let p = self.data.as_mut_ptr();
        let frames = self.frame_count();
        (0..frames).flat_map(move |frame| {
            (0..C).map(move |channel| unsafe { get_unchecked_mut(p, frames, channel, frame) })
        })
    }
}

unsafe fn get_unchecked<'a, T>(p: *const T, frames: usize, c: usize, f: usize) -> &'a T {
    unsafe { &*p.add(c.unchecked_mul(frames).unchecked_add(f)) }
}

unsafe fn get_unchecked_mut<'a, T>(p: *mut T, frames: usize, c: usize, f: usize) -> &'a mut T {
    unsafe { &mut *p.add(c.unchecked_mul(frames).unchecked_add(f)) }
}

impl<T, const C: usize> Default for AudioBuf<T, C> {
    fn default() -> Self {
        Self {
            data: Vec::new().into_boxed_slice(),
        }
    }
}
