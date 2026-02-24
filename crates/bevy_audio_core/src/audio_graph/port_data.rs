use {
    super::{AudioBuf, Discrete},
    core::ptr::NonNull,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortTypeId {
    /// `AudioBuf<f32, 2>`
    Stereo,
    /// `AudioBuf<f32, 1>`
    Mono,
    /// `Discrete<f32>`
    F32,
    /// `Discrete<bool>`
    Bool,
}

pub trait PortType: 'static + Send + Sync {
    const PORT_TYPE_ID: PortTypeId;
}

impl PortType for AudioBuf<f32, 2> {
    const PORT_TYPE_ID: PortTypeId = PortTypeId::Stereo;
}

impl PortType for AudioBuf<f32, 1> {
    const PORT_TYPE_ID: PortTypeId = PortTypeId::Mono;
}

impl PortType for Discrete<bool> {
    const PORT_TYPE_ID: PortTypeId = PortTypeId::Bool;
}

impl PortType for Discrete<f32> {
    const PORT_TYPE_ID: PortTypeId = PortTypeId::F32;
}

#[repr(C)]
struct PortDataInner<T: ?Sized> {
    type_id: PortTypeId,
    data: T,
}

impl<T> PortDataInner<T> {
    pub fn new(value: T) -> Self
    where
        T: PortType,
    {
        Self {
            type_id: T::PORT_TYPE_ID,
            data: value,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PortDataRaw(NonNull<u8>);

impl PortDataRaw {
    fn new<T: PortType>(x: NonNull<PortDataInner<T>>) -> Self {
        Self(x.cast())
    }

    pub fn type_id(self) -> PortTypeId {
        // SAFETY: `PortDataRaw` always references a valid `PortDataInner`, and the first field
        // of that structure is the type ID.
        unsafe { self.0.cast::<PortTypeId>().read() }
    }

    pub fn is<T: PortType>(self) -> bool {
        self.type_id() == T::PORT_TYPE_ID
    }

    /// # Safety
    ///
    /// * The [`PortData`] instance must contain a value of type `T`.
    /// * It must be safe to access the value for the lifetime `'a`.
    pub unsafe fn downcast_ref_unchecked<'a, T: PortType>(self) -> &'a T {
        debug_assert!(self.is::<T>());

        // SAFETY: The safety is upheld by the caller.
        unsafe { &self.0.cast::<PortDataInner<T>>().as_ref().data }
    }

    /// # Safety
    ///
    /// * The [`PortData`] instance must contain a value of type `T`.
    /// * It must be safe to access the value for the lifetime `'a`.
    pub unsafe fn downcast_mut_unchecked<'a, T: PortType>(self) -> &'a mut T {
        debug_assert!(self.is::<T>());

        // SAFETY: The safety is upheld by the caller.
        unsafe { &mut self.0.cast::<PortDataInner<T>>().as_mut().data }
    }
}

// SAFETY: This type basically works like a raw pointer. It doesn't provide access to the
// underlying data, so it's safe to send and share it between threads. The user is responsible
// for converting this type to a concrete reference when it is safe to do so.
//
// Note: We're actually providing access to the inner `PortTypeId`, but it is immutable after creation,
// and it is itself `Send` and `Sync`, so that doesn't change the reasoning.
unsafe impl Send for PortDataRaw {}
unsafe impl Sync for PortDataRaw {}

pub struct PortDataBox(PortDataRaw);

impl PortDataBox {
    pub fn new<T: PortType>(data: T) -> Self {
        let p = Box::into_raw(Box::new(PortDataInner::new(data)));
        // SAFETY: The pointer returned by `Box::into_raw` is always non-null.
        Self(PortDataRaw::new(unsafe { NonNull::new_unchecked(p) }))
    }

    pub fn as_raw(&self) -> PortDataRaw {
        self.0
    }

    pub fn type_id(&self) -> PortTypeId {
        self.0.type_id()
    }

    pub fn is<T: PortType>(&self) -> bool {
        self.0.is::<T>()
    }

    /// # Safety
    ///
    /// The [`PortData`] instance must contain a value of type `T`.
    pub unsafe fn downcast_ref_unchecked<T: PortType>(&self) -> &T {
        // SAFETY: Safety is upheld by the caller.
        unsafe { self.0.downcast_ref_unchecked() }
    }

    /// # Safety
    ///
    /// The [`PortData`] instance must contain a value of type `T`.
    pub unsafe fn downcast_mut_unchecked<T: PortType>(&mut self) -> &mut T {
        // SAFETY: Safety is upheld by the caller.
        unsafe { self.0.downcast_mut_unchecked() }
    }

    pub fn downcast_ref<T: PortType>(&self) -> Option<&T> {
        if self.is::<T>() {
            Some(unsafe { self.downcast_ref_unchecked::<T>() })
        } else {
            None
        }
    }

    pub fn downcast_mut<T: PortType>(&mut self) -> Option<&mut T> {
        if self.is::<T>() {
            Some(unsafe { self.downcast_mut_unchecked::<T>() })
        } else {
            None
        }
    }
}

impl Drop for PortDataBox {
    fn drop(&mut self) {
        unsafe fn drop_as<T>(p: PortDataRaw) {
            let ptr = p.0.cast::<PortDataInner<T>>();
            // SAFETY: Safety must be upheld by the caller.
            drop(unsafe { Box::from_raw(ptr.as_ptr()) });
        }

        match self.type_id() {
            PortTypeId::F32 => unsafe { drop_as::<Discrete<f32>>(self.0) },
            PortTypeId::Bool => unsafe { drop_as::<Discrete<bool>>(self.0) },
            PortTypeId::Mono => unsafe { drop_as::<AudioBuf<f32, 1>>(self.0) },
            PortTypeId::Stereo => unsafe { drop_as::<AudioBuf<f32, 2>>(self.0) },
        }
    }
}

impl core::fmt::Debug for PortDataBox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PortDataBox({:?})", self.type_id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_ids() {
        assert_eq!(<AudioBuf<f32, 1>>::PORT_TYPE_ID, PortTypeId::Mono);
        assert_eq!(<AudioBuf<f32, 2>>::PORT_TYPE_ID, PortTypeId::Stereo);
        assert_eq!(<Discrete<f32>>::PORT_TYPE_ID, PortTypeId::F32);
        assert_eq!(<Discrete<bool>>::PORT_TYPE_ID, PortTypeId::Bool);
    }

    #[test]
    fn port_data_box_type_id() {
        let mono = PortDataBox::new(AudioBuf::<f32, 1>::default());
        let stereo = PortDataBox::new(AudioBuf::<f32, 2>::default());
        let f32 = PortDataBox::new(Discrete::<f32>::default());
        let bool = PortDataBox::new(Discrete::<bool>::default());

        assert_eq!(mono.type_id(), PortTypeId::Mono);
        assert_eq!(stereo.type_id(), PortTypeId::Stereo);
        assert_eq!(f32.type_id(), PortTypeId::F32);
        assert_eq!(bool.type_id(), PortTypeId::Bool);
    }

    #[test]
    fn port_data_box_is() {
        let mono = PortDataBox::new(AudioBuf::<f32, 1>::default());
        let stereo = PortDataBox::new(AudioBuf::<f32, 2>::default());
        let f32 = PortDataBox::new(Discrete::<f32>::default());
        let bool = PortDataBox::new(Discrete::<bool>::default());

        assert!(mono.is::<AudioBuf::<f32, 1>>());
        assert!(!mono.is::<AudioBuf::<f32, 2>>());
        assert!(stereo.is::<AudioBuf::<f32, 2>>());
        assert!(!stereo.is::<AudioBuf::<f32, 1>>());
        assert!(f32.is::<Discrete::<f32>>());
        assert!(bool.is::<Discrete::<bool>>());
    }

    #[test]
    fn port_data_box_downcast_ref() {
        let mono = PortDataBox::new(AudioBuf::<f32, 1>::default());
        let stereo = PortDataBox::new(AudioBuf::<f32, 2>::default());
        let f32 = PortDataBox::new(Discrete::<f32>::default());
        let bool = PortDataBox::new(Discrete::<bool>::default());

        assert!(mono.downcast_ref::<AudioBuf::<f32, 1>>().is_some());
        assert!(mono.downcast_ref::<AudioBuf::<f32, 2>>().is_none());
        assert!(stereo.downcast_ref::<AudioBuf::<f32, 2>>().is_some());
        assert!(stereo.downcast_ref::<AudioBuf::<f32, 1>>().is_none());
        assert!(f32.downcast_ref::<Discrete::<f32>>().is_some());
        assert!(bool.downcast_ref::<Discrete::<bool>>().is_some());
    }
}
