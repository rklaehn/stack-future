mod stack_future;
use std::task::{Context, Poll};

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
