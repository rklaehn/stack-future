mod stack_future;
use std::{
    pin::Pin,
    ptr,
    task::{Context, Poll},
};

pub use stack_future::{CreateError, LocalStackFuture, StackFuture};

mod small_future;
pub use small_future::{LocalSmallFuture, SmallFuture};
// A wrapper to enforce coarse alignment on the buffer.
#[repr(align(8))]
struct AlignedBuffer<const N: usize> {
    buffer: [u8; N],
}

struct VTable<T> {
    poll: unsafe fn(*mut u8, cx: &mut Context<'_>) -> Poll<T>,
    drop: unsafe fn(*mut u8),
}

impl<T> VTable<T> {
    fn new<'a, F: Future<Output = T> + 'a>() -> &'a Self {
        &Self {
            poll: |ptr, cx| {
                let future = unsafe { &mut *(ptr as *mut F) };
                unsafe { Pin::new_unchecked(future).poll(cx) }
            },
            drop: |ptr| {
                unsafe { ptr::drop_in_place(ptr as *mut F) };
            },
        }
    }
}
