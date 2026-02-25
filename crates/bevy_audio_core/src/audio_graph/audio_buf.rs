use {
    bytemuck::Zeroable,
    std::{
        alloc::{Layout, handle_alloc_error},
        hint::assert_unchecked,
        num::NonZero,
        ptr::NonNull,
    },
};

const ALIGNMENT: usize = 64;

pub struct AudioBuf<T, const C: usize> {
    /// # Invariants
    ///
    /// * When `true`, then `data` must be assumed to contain only zeros. Note that the reciprocal
    ///   isn't necessarily true.
    silent: bool,
    frame_count: usize,
    /// References `C` buffers of `frame_count` elements.
    ///
    /// Each buffer is aligned to `ALIGNMENT` bytes, adding padding to the end of each channel if
    /// required.
    data: NonNull<T>,
}

// SAFETY: `AudioBuf` uses the regular XOR access, ensuring that it can be safely sent/shared
// between threads if `T` can.
unsafe impl<T: Send, const C: usize> Send for AudioBuf<T, C> {}
unsafe impl<T: Sync, const C: usize> Sync for AudioBuf<T, C> {}

impl<T, const C: usize> AudioBuf<T, C> {
    pub fn new(frame_count: usize) -> Self
    where
        T: Zeroable,
    {
        let alignment = ALIGNMENT.max(align_of::<T>());

        let data = if size_of::<T>() == 0 || frame_count == 0 || C == 0 {
            // SAFETY: `ALIGNMENT` is not zero.
            unsafe { NonNull::<T>::without_provenance(NonZero::new_unchecked(alignment)) }
        } else {
            let layout = frame_count
                .checked_mul(size_of::<T>())
                .and_then(|packed_channel_size| align_up_checked(packed_channel_size, alignment))
                .and_then(|padded_channel_size| padded_channel_size.checked_mul(C))
                .and_then(|total_size| Layout::from_size_align(total_size, alignment).ok())
                .expect("Capacity overflow");

            let data = unsafe { alloc::alloc::alloc_zeroed(layout) };

            NonNull::new(data)
                .unwrap_or_else(|| handle_alloc_error(layout))
                .cast::<T>()
        };

        Self {
            silent: true,
            frame_count,
            data,
        }
    }

    pub fn as_ptr(&self) -> *const T {
        let p = self.data.as_ptr();

        // SAFETY: This is an invariant of `data`. Putting this here may help the optimizer
        // vectorize operations on the buffer in some cases.
        unsafe { assert_unchecked(p.addr().is_multiple_of(ALIGNMENT)) };

        p
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        let p = self.data.as_ptr();

        // SAFETY: This is an invariant of `data`. Putting this here may help the optimizer
        // vectorize operations on the buffer in some cases.
        unsafe { assert_unchecked(p.addr().is_multiple_of(ALIGNMENT)) };

        p
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn sample_count(&self) -> usize {
        // SAFETY: We were able to allocate that many samples in the first place, ensuring that
        // overflow isn't possible.
        unsafe { self.frame_count.unchecked_mul(C) }
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
        unsafe { std::ptr::write_bytes(self.as_mut_ptr(), 0x00, self.sample_count()) };
    }

    /// # Safety
    ///
    /// * `count` must be less than or equal to the buffer's frame count.
    #[track_caller]
    pub unsafe fn clear_to_unchecked(&mut self, count: usize)
    where
        T: Zeroable,
    {
        debug_assert!(count <= self.frame_count());
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

        // SAFETY: Caller must ensure that the channel index is within bounds.
        unsafe { &*get_channel_unchecked(self.as_ptr().cast_mut(), self.frame_count(), channel) }
    }

    /// # Safety
    ///
    /// `channel` must be less than `C`.
    pub unsafe fn channel_unchecked_mut(&mut self, channel: usize) -> &mut [T] {
        debug_assert!(channel < C);

        // SAFETY: Caller must ensure that the channel index is within bounds.
        unsafe { &mut *get_channel_unchecked(self.as_mut_ptr(), self.frame_count(), channel) }
    }

    pub fn channel(&self, channel: usize) -> Option<&[T]> {
        if channel < C {
            // SAFETY: We just made sure that the provided index is valid.
            unsafe {
                Some(&*get_channel_unchecked(
                    self.as_ptr().cast_mut(),
                    self.frame_count(),
                    channel,
                ))
            }
        } else {
            None
        }
    }

    pub fn channel_mut(&mut self, channel: usize) -> Option<&mut [T]> {
        if channel < C {
            // SAFETY: We just made sure that the provided index is valid.
            unsafe {
                Some(&mut *get_channel_unchecked(
                    self.as_mut_ptr(),
                    self.frame_count(),
                    channel,
                ))
            }
        } else {
            None
        }
    }

    pub fn channels(&self) -> impl Iterator<Item = &[T]> {
        let p = self.as_ptr().cast_mut();
        let frames = self.frame_count();
        // SAFETY: `channel < C`.
        (0..C).map(move |channel| unsafe { &*get_channel_unchecked(p, frames, channel) })
    }

    pub fn channels_mut(&mut self) -> impl Iterator<Item = &mut [T]> {
        let p = self.as_mut_ptr();
        let frames = self.frame_count();
        // SAFETY: `channel < C`.
        (0..C).map(move |channel| unsafe { &mut *get_channel_unchecked(p, frames, channel) })
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
        unsafe { &*get_ptr_unchecked(self.as_ptr().cast_mut(), self.frame_count(), channel, frame) }
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
        unsafe { &mut *get_ptr_unchecked(self.as_mut_ptr(), self.frame_count(), channel, frame) }
    }

    pub fn frames(&self) -> impl Iterator<Item = [&T; C]> {
        let p = self.as_ptr().cast_mut();
        let frames = self.frame_count();
        (0..frames).map(move |frame| {
            core::array::from_fn(|channel| unsafe {
                &*get_ptr_unchecked(p, frames, channel, frame)
            })
        })
    }

    pub fn frames_mut(&mut self) -> impl Iterator<Item = [&mut T; C]> {
        let p = self.as_mut_ptr();
        let frames = self.frame_count();
        (0..frames).map(move |frame| {
            core::array::from_fn(move |channel| unsafe {
                &mut *get_ptr_unchecked(p, frames, channel, frame)
            })
        })
    }

    pub fn interleaved(&self) -> impl Iterator<Item = &T> {
        let p = self.as_ptr().cast_mut();
        let frames = self.frame_count();
        (0..frames).flat_map(move |frame| {
            (0..C).map(move |channel| unsafe { &*get_ptr_unchecked(p, frames, channel, frame) })
        })
    }

    pub fn interleaved_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let p = self.as_mut_ptr();
        let frames = self.frame_count();
        (0..frames).flat_map(move |frame| {
            (0..C).map(move |channel| unsafe { &mut *get_ptr_unchecked(p, frames, channel, frame) })
        })
    }

    pub fn flat_iter(&self) -> impl Iterator<Item = &T> {
        let p = self.as_ptr().cast_mut();
        let frames = self.frame_count();
        (0..C).flat_map(move |c| {
            (0..frames).map(move |f| unsafe { &*get_ptr_unchecked(p, frames, c, f) })
        })
    }

    pub fn flat_iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        let p = self.as_ptr().cast_mut();
        let frames = self.frame_count();
        (0..C).flat_map(move |c| {
            (0..frames).map(move |f| unsafe { &mut *get_ptr_unchecked(p, frames, c, f) })
        })
    }
}

impl<T, const C: usize> Drop for AudioBuf<T, C> {
    fn drop(&mut self) {
        // SAFETY: This size was used to allocate the buffer in the first place, ensuring that the
        // this operation does not overflow and that the final layout is valid.
        let layout = unsafe {
            let alignment = ALIGNMENT.max(align_of::<T>());
            let size =
                align_up_unchecked(self.frame_count.unchecked_mul(size_of::<T>()), alignment)
                    .unchecked_mul(C);
            Layout::from_size_align_unchecked(size, alignment)
        };

        if layout.size() != 0 {
            // SAFETY: If the layout's size is not zero, then we allocated the buffer with
            // `alloc_zeroed` previously.
            unsafe { alloc::alloc::dealloc(self.data.as_ptr().cast(), layout) };
        }
    }
}

unsafe fn get_channel_ptr_unchecked<T>(p: *mut T, frames: usize, channel: usize) -> *mut T {
    unsafe {
        let p = p.byte_add(channel.unchecked_mul(channel_size_in_bytes::<T>(frames)));

        // Help the optimizer vectorize loops that start with this pointer.
        assert_unchecked(p.addr().is_multiple_of(ALIGNMENT));

        p
    }
}

unsafe fn get_channel_unchecked<T>(p: *mut T, frames: usize, channel: usize) -> *mut [T] {
    unsafe {
        core::ptr::slice_from_raw_parts_mut(get_channel_ptr_unchecked(p, frames, channel), frames)
    }
}

unsafe fn get_ptr_unchecked<T>(p: *mut T, frames: usize, c: usize, f: usize) -> *mut T {
    unsafe { get_channel_ptr_unchecked(p, frames, c).add(f) }
}

unsafe fn channel_size_in_bytes<T>(frames: usize) -> usize {
    // Get the packed size of the channel, then align it up to the buffer alignment.
    unsafe {
        align_up_unchecked(
            frames.unchecked_mul(size_of::<T>()),
            ALIGNMENT.max(align_of::<T>()),
        )
    }
}

fn align_up_checked(x: usize, align: usize) -> Option<usize> {
    let mask = align - 1;
    x.checked_add(mask).map(|x| x & !mask)
}

unsafe fn align_up_unchecked(x: usize, align: usize) -> usize {
    unsafe {
        let mask = align.unchecked_sub(1);
        x.unchecked_add(mask) & !mask
    }
}
