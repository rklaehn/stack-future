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
use std::result::Result;

use snafu::Snafu;

#[derive(Debug, Snafu)]
pub enum CreateError {
    #[snafu(display("Future size exceeds buffer capacity: {size} > {max_size}"))]
    SizeTooLarge { size: usize, max_size: usize },
    #[snafu(display("Future alignment exceeds buffer alignment: {alignment} > {expected}",))]
    AlignmentMismatch { alignment: usize, expected: usize },
}

// A wrapper to enforce coarse alignment on the buffer.
#[repr(align(8))]
struct AlignedBuffer<const N: usize> {
    buffer: [u8; N],
}

// A stack-allocated future with a fixed-size, aligned buffer.
pub struct StackFuture<'a, T, const N: usize> {
    buffer: AlignedBuffer<N>,
    vtable: &'a VTable<T>,
}

// Vtable for type-erased future operations.
struct VTable<T> {
    poll: unsafe fn(*mut u8, cx: &mut Context<'_>) -> Poll<T>,
    drop: unsafe fn(*mut u8),
}

impl<'a, T, const N: usize> StackFuture<'a, T, N> {
    pub fn new<F: Future<Output = T> + 'a>(future: F) -> Result<Self, CreateError> {
        // Check if the future fits in the buffer and has compatible alignment.
        if size_of::<F>() > N {
            return Err(SizeTooLargeSnafu {
                size: size_of::<F>(),
                max_size: N,
            }
            .build());
        }

        if align_of::<F>() > align_of::<AlignedBuffer<N>>() {
            return Err(AlignmentMismatchSnafu {
                alignment: align_of::<F>(),
                expected: align_of::<AlignedBuffer<N>>(),
            }
            .build());
        }

        // Create the vtable for the future type.
        let vtable = &VTable {
            poll: |ptr, cx| {
                let future = unsafe { &mut *(ptr as *mut F) };
                unsafe { Pin::new_unchecked(future).poll(cx) }
            },
            drop: |ptr| {
                unsafe { ptr::drop_in_place(ptr as *mut F) };
            },
        };

        // Initialize the buffer with zeros.
        let mut buffer = AlignedBuffer { buffer: [0u8; N] };

        // Move the future into the buffer.
        unsafe {
            ptr::write(buffer.buffer.as_mut_ptr() as *mut F, future);
        }

        Ok(Self { buffer, vtable })
    }
}

impl<'a, T, const N: usize> Future for StackFuture<'a, T, N> {
    type Output = T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        unsafe { (this.vtable.poll)(this.buffer.buffer.as_mut_ptr(), cx) }
    }
}

impl<'a, T, const N: usize> Drop for StackFuture<'a, T, N> {
    fn drop(&mut self) {
        unsafe {
            (self.vtable.drop)(self.buffer.buffer.as_mut_ptr());
        }
    }
}
