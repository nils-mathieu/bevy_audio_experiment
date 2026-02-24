use {
    super::{GcNode, GcObject, GcObjectExt},
    std::{
        mem::ManuallyDrop,
        ops::{Deref, DerefMut},
        ptr::NonNull,
        sync::atomic::{AtomicPtr, Ordering},
    },
};

#[repr(C)]
struct Inner<T: ?Sized> {
    node: GcNode,
    data: T,
}

pub struct GcBox<T: ?Sized>(NonNull<Inner<T>>);

// SAFETY: `GcBox` provides access to its inner value using the regular XOR access pattern. This
// ensures that as long as the inner value is `Send`/`Sync`, the box itself can be `Send`/`Sync`.
unsafe impl<T: ?Sized + Send> Send for GcBox<T> {}
unsafe impl<T: ?Sized + Sync> Sync for GcBox<T> {}

impl<T> GcBox<T> {
    pub fn new(data: T) -> Self {
        let inner = Box::into_raw(Box::new(Inner {
            node: GcNode::new(|node: *mut GcNode| unsafe {
                drop(Box::from_raw(node.cast::<Inner<T>>()))
            }),
            data,
        }));

        // SAFETY: The pointer returned by `Box::into_raw` is valid and non-null.
        Self(unsafe { NonNull::new_unchecked(inner) })
    }

    pub fn into_inner(self) -> T {
        unsafe { Box::from_raw(self.0.as_ptr()).data }
    }
}

impl<T: ?Sized> GcBox<T> {
    fn inner_mut(&mut self) -> &mut Inner<T> {
        unsafe { self.0.as_mut() }
    }

    fn inner(&self) -> &Inner<T> {
        unsafe { self.0.as_ref() }
    }

    pub fn as_mut_ptr(&mut self) -> *mut T {
        &raw mut self.inner_mut().data
    }

    pub fn as_ptr(&self) -> *const T {
        &raw const self.inner().data
    }

    pub fn into_raw(this: Self) -> *mut T {
        ManuallyDrop::new(this).as_mut_ptr()
    }

    /// # Safety
    ///
    /// Pointer must come from a previous call to [`into_raw()`](Self::into_raw).
    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        // SAFETY: This computation because the pointer was originally obtained from a call
        // to `into_raw()`.
        let offset = unsafe {
            let align_mask = align_of_val(&*ptr).unchecked_sub(1);
            size_of::<Inner<()>>().unchecked_add(align_mask) & align_mask
        };

        // SAFETY: The pointer was originally obtained from a call to `into_raw()`.
        Self(unsafe { NonNull::new_unchecked(ptr.byte_sub(offset) as *mut Inner<T>) })
    }
}

impl<T: ?Sized> Deref for GcBox<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner().data
    }
}

impl<T: ?Sized> DerefMut for GcBox<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner_mut().data
    }
}

// SAFETY: The node pointer returned by `gc_node` remains valid until the object is dropped.
unsafe impl<T: ?Sized> GcObject for GcBox<T> {
    fn gc_node(&mut self) -> *mut GcNode {
        &raw mut self.inner_mut().node
    }
}

impl<T: ?Sized> Drop for GcBox<T> {
    fn drop(&mut self) {
        // SAFETY: We're dropping the object, so the object won't be used again.
        unsafe { self.collect_by_ref() };
    }
}

pub struct AtomicGcBox<T>(AtomicPtr<T>);

impl<T> AtomicGcBox<T> {
    pub const fn null() -> Self {
        Self(AtomicPtr::new(std::ptr::null_mut()))
    }

    pub fn replace_option(&self, b: Option<GcBox<T>>, order: Ordering) -> Option<GcBox<T>> {
        let new_p = b.map_or_else(std::ptr::null_mut, |b| GcBox::into_raw(b));
        let p = self.0.swap(new_p, order);

        if p.is_null() {
            None
        } else {
            // SAFETY: If the pointer is not null, then it points to a valid `GcBox`.
            Some(unsafe { GcBox::from_raw(p) })
        }
    }

    pub fn take(&self, order: Ordering) -> Option<GcBox<T>> {
        self.replace_option(None, order)
    }

    pub fn replace(&self, value: GcBox<T>, order: Ordering) -> Option<GcBox<T>> {
        self.replace_option(Some(value), order)
    }
}

impl<T> From<GcBox<T>> for AtomicGcBox<T> {
    fn from(value: GcBox<T>) -> Self {
        let p = GcBox::into_raw(value);
        Self(AtomicPtr::new(p))
    }
}

impl<T> Drop for AtomicGcBox<T> {
    fn drop(&mut self) {
        let p = *self.0.get_mut();

        if !p.is_null() {
            // SAFETY: If the pointer is not null, then it points to a valid `GcBox`.
            drop(unsafe { GcBox::from_raw(p) });
        }
    }
}
