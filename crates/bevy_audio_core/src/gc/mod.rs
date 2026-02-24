use std::{
    mem::ManuallyDrop,
    sync::atomic::{AtomicPtr, Ordering},
};

mod gc_box;
pub use self::gc_box::*;

pub static GLOBAL: GarbageCollector = GarbageCollector::new();

pub struct GcNode {
    next: AtomicPtr<GcNode>,

    /// # Safety
    ///
    /// The object must not be used after the function is called.
    drop: unsafe fn(*mut GcNode),
}

impl GcNode {
    pub fn new(drop: unsafe fn(*mut GcNode)) -> Self {
        Self {
            next: AtomicPtr::new(std::ptr::null_mut()),
            drop,
        }
    }
}

pub struct GarbageCollector {
    head: AtomicPtr<GcNode>,
}

impl GarbageCollector {
    pub const fn new() -> Self {
        Self {
            head: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// # Safety
    ///
    /// `node` must remain valid until we call `drop`.
    pub unsafe fn collect(&self, node: *mut GcNode) {
        // FIXME: Find out what ordering we need in case of success. We're using the most
        // restrictive ordering for now.
        let mut current = self.head.load(Ordering::SeqCst);

        loop {
            // SAFETY: The caller must ensure that `node` remains valid until we call `drop`.
            unsafe { *(*node).next.get_mut() = current };

            match self.head.compare_exchange_weak(
                current,
                node,
                // FIXME: Find out what ordering we need in case of success. We're using the most
                // restrictive ordering for now.
                Ordering::SeqCst,
                // FIXME: Find out what ordering we need in case of success. We're using the most
                // restrictive ordering for now.
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
    }

    pub fn collect_garbage(&self) {
        // FIXME: Find out what ordering we need in case of success. We're using the most
        // restrictive ordering for now.
        let mut current = self.head.swap(std::ptr::null_mut(), Ordering::SeqCst);

        while !current.is_null() {
            let (next, drop) = unsafe {
                let node = &mut *current;
                (*node.next.get_mut(), node.drop)
            };

            // SAFETY: We took ownership of the whole queue, so we know that nobody is accessing
            // the object anymore.
            unsafe { drop(current) };

            current = next;
        }
    }
}

impl Default for GarbageCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// # Safety
///
/// [`gc_node()`] must return a pointer that remains valid until the object is dropped
/// by calling its `drop` function.
pub unsafe trait GcObject {
    fn gc_node(&mut self) -> *mut GcNode;
}

pub trait GcObjectExt: GcObject {
    fn into_gc_node(self) -> *mut GcNode
    where
        Self: Sized,
    {
        ManuallyDrop::new(self).gc_node()
    }

    /// # Safety
    ///
    /// The caller must ensure that the object is not used after this call.
    unsafe fn collect_into_by_ref(&mut self, target: &GarbageCollector) {
        // SAFETY: The caller must ensure that the object is not used after this call, and the
        // `GcObject` trait ensures that the node remains valid until the object is dropped.
        unsafe { target.collect(self.gc_node()) };
    }

    /// # Safety
    ///
    /// The caller must ensure that the object is not used after this call.
    unsafe fn collect_by_ref(&mut self) {
        // SAFETY: Requirements upheld by the caller.
        unsafe { self.collect_into_by_ref(&GLOBAL) };
    }

    fn collect_into(self, target: &GarbageCollector)
    where
        Self: Sized,
    {
        // SAFETY: We don't use the object anymore after this point. The node will remain valid
        // because this is a requirement of `GcObject`.
        unsafe { target.collect(self.into_gc_node()) };
    }

    fn collect(self)
    where
        Self: Sized,
    {
        self.collect_into(&GLOBAL);
    }

    fn drop_now(self)
    where
        Self: Sized,
    {
        let node = self.into_gc_node();

        // SAFETY: We don't use the object anymore after this point.
        unsafe { ((*node).drop)(node) };
    }
}

impl<T: ?Sized + GcObject> GcObjectExt for T {}
