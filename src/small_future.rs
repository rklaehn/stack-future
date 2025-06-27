use core::{
    fmt,
    future::Future,
    mem::{align_of, size_of},
    pin::Pin,
    ptr,
    task::{Context, Poll},
};
use std::{
    alloc::{Layout, alloc, dealloc},
    marker::{PhantomData, PhantomPinned},
    rc::Rc,
};

// A wrapper to enforce coarse alignment on the buffer.
#[repr(align(8))]
struct AlignedBuffer<const N: usize> {
    buffer: [u8; N],
}

// A wrapper for heap-allocated buffer with dynamic alignment.
struct HeapBuffer {
    ptr: *mut u8,
    layout: Layout,
}

impl HeapBuffer {
    fn new<F>() -> Self {
        let size = size_of::<F>();
        let align = align_of::<F>();
        let layout = Layout::from_size_align(size, align).unwrap();
        let ptr = unsafe { alloc(layout) };
        if ptr.is_null() {
            panic!("Heap allocation failed");
        }
        unsafe {
            ptr::write_bytes(ptr, 0, size);
        }
        Self { ptr, layout }
    }

    fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }
}

impl Drop for HeapBuffer {
    fn drop(&mut self) {
        unsafe {
            dealloc(self.ptr, self.layout);
        }
    }
}

unsafe impl Send for HeapBuffer {}
unsafe impl Sync for HeapBuffer {}

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
                SmallFutureState::Heap { buffer, vtable } => (vtable.poll)(buffer.as_mut_ptr(), cx),
            }
        }
    }
}

impl<'a, T, const N: usize> Drop for SmallFuture<'a, T, N> {
    fn drop(&mut self) {
        match &mut self.0 {
            SmallFutureState::Inline { buffer, vtable } => unsafe {
                (vtable.drop)(buffer.buffer.as_mut_ptr());
            },
            SmallFutureState::Heap { buffer, vtable } => unsafe {
                (vtable.drop)(buffer.as_mut_ptr());
            },
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
    pub fn new<F: Future<Output = T> + Send + 'a>(future: F) -> Self {
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
                SmallFutureSendState::Heap { buffer, vtable } => {
                    (vtable.poll)(buffer.as_mut_ptr(), cx)
                }
            }
        }
    }
}

impl<'a, T, const N: usize> Drop for SmallFutureSend<'a, T, N> {
    fn drop(&mut self) {
        match &mut self.0 {
            SmallFutureSendState::Inline { buffer, vtable } => unsafe {
                (vtable.drop)(buffer.buffer.as_mut_ptr());
            },
            SmallFutureSendState::Heap { buffer, vtable } => unsafe {
                (vtable.drop)(buffer.as_mut_ptr());
            },
        }
    }
}

enum SmallFutureState<'a, T, const N: usize> {
    Inline {
        buffer: AlignedBuffer<N>,
        vtable: &'a VTable<T>,
    },
    Heap {
        buffer: HeapBuffer,
        vtable: &'a VTable<T>,
    },
}

enum SmallFutureSendState<'a, T, const N: usize> {
    Inline {
        buffer: AlignedBuffer<N>,
        vtable: &'a VTable<T>,
    },
    Heap {
        buffer: HeapBuffer,
        vtable: &'a VTable<T>,
    },
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
            let vtable = &VTable {
                poll: |ptr, cx| {
                    let future = unsafe { &mut *(ptr as *mut F) };
                    unsafe { Pin::new_unchecked(future).poll(cx) }
                },
                drop: |ptr| {
                    unsafe { ptr::drop_in_place(ptr as *mut F) };
                },
            };
            let mut buffer = HeapBuffer::new::<F>();
            unsafe {
                ptr::write(buffer.as_mut_ptr() as *mut F, future);
            }
            Self::Heap { buffer, vtable }
        }
    }
}

impl<'a, T: 'a, const N: usize> SmallFutureSendState<'a, T, N> {
    fn new<F: Future<Output = T> + Send + 'a>(future: F) -> Self {
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
            let vtable = &VTable {
                poll: |ptr, cx| {
                    let future = unsafe { &mut *(ptr as *mut F) };
                    unsafe { Pin::new_unchecked(future).poll(cx) }
                },
                drop: |ptr| {
                    unsafe { ptr::drop_in_place(ptr as *mut F) };
                },
            };
            let mut buffer = HeapBuffer::new::<F>();
            unsafe {
                ptr::write(buffer.as_mut_ptr() as *mut F, future);
            }
            Self::Heap { buffer, vtable }
        }
    }
}

struct VTable<T> {
    poll: unsafe fn(*mut u8, cx: &mut Context<'_>) -> Poll<T>,
    drop: unsafe fn(*mut u8),
}
