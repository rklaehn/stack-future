use core::{
    fmt,
    future::Future,
    mem::{align_of, size_of},
    pin::Pin,
    ptr,
    task::{Context, Poll},
};
use std::{
    marker::{PhantomData, PhantomPinned},
    rc::Rc,
};

// A wrapper to enforce coarse alignment on the buffer.
#[repr(align(8))]
struct AlignedBuffer<const N: usize> {
    buffer: [u8; N],
}

/// A stack-allocated future that erases the concrete type, falling back to heap if needed.
///
/// This is non-Send and !Unpin, safe for any future (e.g., containing Rc).
/// Use `SmallFutureSend` for Send futures in multi-threaded contexts.
/// Note: Due to !Unpin, this may require boxing (e.g., `Box::pin`) for Unpin-requiring APIs.
#[repr(transparent)]
pub struct SmallFuture<'a, T, const N: usize>(
    SmallFutureState<'a, T, N>,
    PhantomPinned,
    PhantomData<Rc<()>>,
);

impl<'a, T, const N: usize> SmallFuture<'a, T, N> {
    /// Creates a new stack future from a concrete future.
    ///
    /// Uses stack allocation if the future fits and has compatible alignment; otherwise, falls back to heap.
    pub fn new<F: Future<Output = T> + 'a>(future: F) -> Self {
        Self(SmallFutureState::new(future), PhantomPinned, PhantomData)
    }
}

impl<'a, T, const N: usize> fmt::Debug for SmallFuture<'a, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmallFuture")
            .field("size", &size_of::<Self>())
            .field("alignment", &align_of::<Self>())
            .finish()
    }
}

impl<'a, T, const N: usize> Future for SmallFuture<'a, T, N> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            let this = self.get_unchecked_mut();
            match &mut this.0 {
                SmallFutureState::Inline { buffer, vtable } => {
                    (vtable.poll)(buffer.buffer.as_mut_ptr(), cx)
                }
                SmallFutureState::Heap(future) => future.as_mut().poll(cx),
            }
        }
    }
}

impl<'a, T, const N: usize> Drop for SmallFuture<'a, T, N> {
    fn drop(&mut self) {
        if let SmallFutureState::Inline { buffer, vtable } = &mut self.0 {
            unsafe {
                (vtable.drop)(buffer.buffer.as_mut_ptr());
            }
        }
    }
}

/// A stack-allocated future that erases the concrete type, falling back to heap if needed.
///
/// This is Send, Sync, and !Unpin, suitable for Send futures in multi-threaded contexts (e.g., tokio::spawn).
/// Note: Due to !Unpin, this may require boxing (e.g., `Box::pin`) for Unpin-requiring APIs.
#[repr(transparent)]
pub struct SmallFutureSend<'a, T, const N: usize>(SmallFutureSendState<'a, T, N>, PhantomPinned);

impl<'a, T, const N: usize> SmallFutureSend<'a, T, N> {
    /// Creates a new stack future from a concrete Send future.
    ///
    /// Uses stack allocation if the future fits and has compatible alignment; otherwise, falls back to heap.
    pub fn new<F: Future<Output = T> + Send + Sync + 'a>(future: F) -> Self {
        Self(SmallFutureSendState::new(future), PhantomPinned)
    }
}

impl<'a, T, const N: usize> fmt::Debug for SmallFutureSend<'a, T, N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmallFutureSend")
            .field("size", &size_of::<Self>())
            .field("alignment", &align_of::<Self>())
            .finish()
    }
}

impl<'a, T, const N: usize> Future for SmallFutureSend<'a, T, N> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            let this = self.get_unchecked_mut();
            match &mut this.0 {
                SmallFutureSendState::Inline { buffer, vtable } => {
                    (vtable.poll)(buffer.buffer.as_mut_ptr(), cx)
                }
                SmallFutureSendState::Heap(future) => future.as_mut().poll(cx),
            }
        }
    }
}

impl<'a, T, const N: usize> Drop for SmallFutureSend<'a, T, N> {
    fn drop(&mut self) {
        if let SmallFutureSendState::Inline { buffer, vtable } = &mut self.0 {
            unsafe {
                (vtable.drop)(buffer.buffer.as_mut_ptr());
            }
        }
    }
}

enum SmallFutureState<'a, T, const N: usize> {
    Inline {
        buffer: AlignedBuffer<N>,
        vtable: &'a VTable<T>,
    },
    Heap(Pin<Box<dyn Future<Output = T> + 'a>>),
}

enum SmallFutureSendState<'a, T, const N: usize> {
    Inline {
        buffer: AlignedBuffer<N>,
        vtable: &'a VTable<T>,
    },
    Heap(Pin<Box<dyn Future<Output = T> + Send + Sync + 'a>>),
}

impl<'a, T: 'a, const N: usize> SmallFutureState<'a, T, N> {
    fn new<F: Future<Output = T> + 'a>(future: F) -> Self {
        if size_of::<F>() <= N && align_of::<F>() <= align_of::<AlignedBuffer<N>>() {
            let vtable = &VTable {
                poll: |ptr, cx| {
                    let future = unsafe { &mut *(ptr as *mut F) };
                    unsafe { Pin::new_unchecked(future).poll(cx) }
                },
                drop: |ptr| {
                    unsafe { ptr::drop_in_place(ptr as *mut F) };
                },
            };
            let mut buffer = AlignedBuffer { buffer: [0u8; N] };
            unsafe {
                ptr::write(buffer.buffer.as_mut_ptr() as *mut F, future);
            }
            Self::Inline { buffer, vtable }
        } else {
            Self::Heap(Box::pin(future))
        }
    }
}

impl<'a, T: 'a, const N: usize> SmallFutureSendState<'a, T, N> {
    fn new<F: Future<Output = T> + Send + Sync + 'a>(future: F) -> Self {
        if size_of::<F>() <= N && align_of::<F>() <= align_of::<AlignedBuffer<N>>() {
            let vtable = &VTable {
                poll: |ptr, cx| {
                    let future = unsafe { &mut *(ptr as *mut F) };
                    unsafe { Pin::new_unchecked(future).poll(cx) }
                },
                drop: |ptr| {
                    unsafe { ptr::drop_in_place(ptr as *mut F) };
                },
            };
            let mut buffer = AlignedBuffer { buffer: [0u8; N] };
            unsafe {
                ptr::write(buffer.buffer.as_mut_ptr() as *mut F, future);
            }
            Self::Inline { buffer, vtable }
        } else {
            Self::Heap(Box::pin(future))
        }
    }
}

struct VTable<T> {
    poll: unsafe fn(*mut u8, cx: &mut Context<'_>) -> Poll<T>,
    drop: unsafe fn(*mut u8),
}
