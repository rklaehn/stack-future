//! A stack-allocated future with a fixed-size, aligned buffer.
//!
//! Often you want to get rid of the concrete type of a future, but don't want to
//! allocate on the heap.
//!
//! If you have an upper bound for the size of the future, you can use [`StackFuture`]
//! to turn your future into a type-erased future that is allocated on the stack.
//!
//! Creating a [`StackFuture`] will fail if the future is too large or has
//! too big alignment requirements.
use core::{
    future::Future,
    mem::{align_of, size_of},
    pin::Pin,
    ptr,
    task::{Context, Poll},
};
use std::{
    fmt,
    marker::{PhantomData, PhantomPinned},
    rc::Rc,
    result::Result,
};

use crate::VTable;

#[derive(Debug)]
pub enum CreateError {
    SizeTooLarge { size: usize, max_size: usize },
    AlignmentMismatch { alignment: usize, expected: usize },
}

impl fmt::Display for CreateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CreateError::SizeTooLarge { size, max_size } => {
                write!(
                    f,
                    "Future size exceeds buffer capacity: {size} > {max_size}"
                )
            }
            CreateError::AlignmentMismatch {
                alignment,
                expected,
            } => {
                write!(
                    f,
                    "Future alignment exceeds buffer alignment: {alignment} > {expected}"
                )
            }
        }
    }
}

impl std::error::Error for CreateError {}

// A wrapper to enforce coarse alignment on the buffer.
#[repr(align(8))]
struct AlignedBuffer<const N: usize> {
    // todo: use MaybeUninit to avoid zero-initialization
    buffer: [u8; N],
}

/// A stack-allocated future that erases the concrete type of the future.
///
/// This is the non-Send version of the future.
#[repr(transparent)]
pub struct LocalStackFuture<'a, T, const N: usize>(StackFutureImpl<'a, T, N>, PhantomData<Rc<()>>);

impl<'a, T, const N: usize> fmt::Debug for LocalStackFuture<'a, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalStackFuture")
            .field("size", &size_of::<Self>())
            .field("alignment", &align_of::<Self>())
            .finish()
    }
}

impl<'a, T, const N: usize> LocalStackFuture<'a, T, N> {
    /// Creates a new stack future from a concrete future.
    ///
    /// Returns an error if the future is too large or has incompatible alignment.
    pub fn new<F: Future<Output = T> + 'a>(future: F) -> Result<Self, CreateError> {
        Ok(Self(StackFutureImpl::new(future)?, PhantomData))
    }

    // Safe helper to access inner as pinned.
    fn inner(self: Pin<&mut Self>) -> Pin<&mut StackFutureImpl<'a, T, N>> {
        // Safe because #[repr(transparent)] ensures Pin<&mut Self> is equivalent to Pin<&mut StackFutureImpl>.
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }
    }
}

impl<'a, T, const N: usize> Future for LocalStackFuture<'a, T, N> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner().poll(cx)
    }
}

/// A stack-allocated future that erases the concrete type of the future.
///
/// This is the Send version of the future.
#[repr(transparent)]
pub struct StackFuture<'a, T, const N: usize>(StackFutureImpl<'a, T, N>);

impl<'a, T, const N: usize> std::fmt::Debug for StackFuture<'a, T, N> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StackFutureSend")
            .field("size", &size_of::<Self>())
            .field("alignment", &align_of::<Self>())
            .finish()
    }
}

impl<'a, T, const N: usize> StackFuture<'a, T, N> {
    /// Creates a new stack future from a concrete future.
    ///
    /// Returns an error if the future is too large or has incompatible alignment.
    pub fn new<F: Future<Output = T> + Send + 'a>(future: F) -> Result<Self, CreateError> {
        Ok(Self(StackFutureImpl::new(future)?))
    }

    // Safe helper to access inner as pinned.
    fn inner(self: Pin<&mut Self>) -> Pin<&mut StackFutureImpl<'a, T, N>> {
        // Safe because #[repr(transparent)] ensures Pin<&mut Self> is equivalent to Pin<&mut StackFutureImpl>.
        unsafe { self.map_unchecked_mut(|s| &mut s.0) }
    }
}

impl<'a, T, const N: usize> Future for StackFuture<'a, T, N> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.inner().poll(cx)
    }
}

/// A stack-allocated future with a fixed-size, aligned buffer.
///
/// Safety: this hides the Send-ness of the inner future type, so it must not
/// be publicly accessible outside of this crate.
struct StackFutureImpl<'a, T, const N: usize> {
    buffer: AlignedBuffer<N>,
    vtable: &'a VTable<T>,
    _pinned: PhantomPinned,
}

impl<'a, T, const N: usize> StackFutureImpl<'a, T, N> {
    pub fn new<F: Future<Output = T> + 'a>(future: F) -> Result<Self, CreateError> {
        // Check if the future fits in the buffer and has compatible alignment.
        if size_of::<F>() > N {
            return Err(CreateError::SizeTooLarge {
                size: size_of::<F>(),
                max_size: N,
            });
        }

        if align_of::<F>() > align_of::<AlignedBuffer<N>>() {
            return Err(CreateError::AlignmentMismatch {
                alignment: align_of::<F>(),
                expected: align_of::<AlignedBuffer<N>>(),
            });
        }

        // Create the vtable for the future type.
        let vtable = VTable::new::<F>();

        // Initialize the buffer with zeros.
        let mut buffer = AlignedBuffer { buffer: [0u8; N] };

        // Move the future into the buffer.
        unsafe {
            ptr::write(buffer.buffer.as_mut_ptr() as *mut F, future);
        }

        Ok(Self {
            buffer,
            vtable,
            _pinned: PhantomPinned,
        })
    }
}

impl<'a, T, const N: usize> Future for StackFutureImpl<'a, T, N> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            let this = self.get_unchecked_mut();
            (this.vtable.poll)(this.buffer.buffer.as_mut_ptr(), cx)
        }
    }
}

impl<'a, T, const N: usize> Drop for StackFutureImpl<'a, T, N> {
    fn drop(&mut self) {
        unsafe {
            (self.vtable.drop)(self.buffer.buffer.as_mut_ptr());
        }
    }
}
