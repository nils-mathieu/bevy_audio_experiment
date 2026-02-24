use bytemuck::Zeroable;

pub struct AudioBuf<T, const C: usize> {
    /// # Invariants
    ///
    /// * When `true`, then `data` must be assumed to contain only zeros. Note that the reciprocal
    ///   isn't necessarily true.
    silent: bool,
    // TODO: Store the frame count instead of the total sample count.
    // TODO: Align each channel buffer to 64 bytes so that we can use huge SIMD instructions when
    // iterating over it.
    data: Box<[T]>,
}

impl<T, const C: usize> AudioBuf<T, C> {
    pub fn with_capacity(capacity: usize) -> Self
    where
        T: Zeroable,
    {
        Self {
            silent: true,
            data: bytemuck::zeroed_slice_box(capacity.checked_mul(C).expect("Capacity overflow")),
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

    pub fn set_silent(&mut self, silent: bool) {
        self.silent = silent;
    }

    pub fn is_silent(&self) -> bool {
        self.silent
    }

    pub fn clear(&mut self)
    where
        T: Zeroable,
    {
        // SAFETY: `T` is zeroable, so we can safely write zeros on it.
        // TODO: Once the buffers are aligned to some higher power of two, we can use
        // optimized SIMD methods to clear the buffer.
        unsafe { std::ptr::write_bytes(self.data.as_mut_ptr(), 0x00, self.data.len()) };
    }

    /// # Safety
    ///
    /// * `count` must be less than or equal to the buffer's frame count.
    pub unsafe fn clear_to_unchecked(&mut self, count: usize)
    where
        T: Zeroable,
    {
        debug_assert!(count < self.frame_count());
        for channel in self.channels_mut() {
            // TODO: Once the buffers are aligned to some higher power of two, we can use
            // optimized SIMD methods to clear the buffer.
            unsafe { std::ptr::write_bytes(channel.as_mut_ptr(), 0x00, count) };
        }
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

    pub fn channels(&self) -> impl Iterator<Item = &[T]> {
        let p = self.data.as_ptr();
        let frames = self.frame_count();
        // SAFETY: `channel < C`.
        (0..C).map(move |channel| unsafe { get_channel_unchecked(p, frames, channel) })
    }

    pub fn channels_mut(&mut self) -> impl Iterator<Item = &mut [T]> {
        let p = self.data.as_mut_ptr();
        let frames = self.frame_count();
        // SAFETY: `channel < C`.
        (0..C).map(move |channel| unsafe { get_channel_unchecked_mut(p, frames, channel) })
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
        unsafe { get_unchecked(self.data.as_ptr(), self.frame_count(), channel, frame) }
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
        unsafe { get_unchecked_mut(self.data.as_mut_ptr(), self.frame_count(), channel, frame) }
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

unsafe fn get_channel_unchecked<'a, T>(p: *const T, frames: usize, channel: usize) -> &'a [T] {
    unsafe { core::slice::from_raw_parts(p.add(channel.unchecked_mul(frames)), frames) }
}

unsafe fn get_channel_unchecked_mut<'a, T>(
    p: *mut T,
    frames: usize,
    channel: usize,
) -> &'a mut [T] {
    unsafe { core::slice::from_raw_parts_mut(p.add(channel.unchecked_mul(frames)), frames) }
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
            silent: true,
        }
    }
}
