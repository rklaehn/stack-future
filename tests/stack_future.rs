use std::sync::OnceLock;

use stack_future::{CreateError, LocalStackFuture, StackFuture};
use static_assertions::{assert_impl_all, assert_not_impl_any};

async fn simple() -> u64 {
    42
}

async fn complex() -> u64 {
    let mut sum = 0;
    for i in 0..100 {
        sum += i;
        tokio::time::sleep(std::time::Duration::from_micros(10)).await;
    }
    sum
}

#[repr(align(256))]
struct AlignedBuffer<const N: usize> {
    buffer: [u8; N],
}

async fn large_align() -> u64 {
    let mut buffer = AlignedBuffer::<256> { buffer: [0; 256] };
    for i in 0..256 {
        buffer.buffer[i] = i as u8;
        // we need a yield point so the buffer moves into the actual future
        tokio::time::sleep(std::time::Duration::from_micros(10)).await;
    }

    buffer.buffer.iter().map(|&x| x as u64).sum()
}

/// Tests that the wrapped futures work, and also that they fail if size or alignment is wrong.
#[tokio::test]
async fn smoke_test() {
    let result = LocalStackFuture::<_, 32>::new(simple()).unwrap().await;
    assert_eq!(result, 42, "Unexpected result from StackFuture");
    let result = LocalStackFuture::<_, 256>::new(complex()).unwrap().await;
    assert_eq!(result, 4950, "Unexpected result from StackFuture");
    let res = LocalStackFuture::<_, 16>::new(complex());
    assert!(
        matches!(res, Err(CreateError::SizeTooLarge { .. })),
        "Expected error for too large future"
    );
    let res = LocalStackFuture::<_, 1024>::new(large_align());
    assert!(
        matches!(res, Err(CreateError::AlignmentMismatch { .. })),
        "Expected error for misaligned future"
    );
}

static GLOBAL_TASK: OnceLock<StackFuture<'static, u64, 128>> = OnceLock::new();

/// Test that the the static lifetime future is properly captured.
#[tokio::test]
async fn static_future_test() {
    let future = StackFuture::<_, 128>::new(simple()).unwrap();
    // This fails to compile with vtable: &'a VTable<T> because StackFuture<'static, i32, 128> is not 'static.
    GLOBAL_TASK.set(future).unwrap();
}

assert_not_impl_any!(LocalStackFuture<'static, u64, 128>: Send, Unpin);
assert_impl_all!(StackFuture<'static, u64, 128>: Send);
assert_not_impl_any!(StackFuture<'static, u64, 128>: Unpin);
