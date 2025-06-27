use std::{pin::Pin, rc::Rc, sync::OnceLock};

use stack_future::{SmallFuture, SmallFutureSend};
use static_assertions::{assert_impl_all, assert_not_impl_any};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
        tokio::time::sleep(std::time::Duration::from_micros(10)).await;
    }
    buffer.buffer.iter().map(|&x| x as u64).sum()
}

async fn large_size() -> u64 {
    let _large = [0u8; 1024]; // Larger than 128 bytes.
    tokio::time::sleep(std::time::Duration::from_micros(10)).await;
    42
}

async fn non_send_future() -> u64 {
    let _rc = Rc::new(42);
    tokio::time::sleep(std::time::Duration::from_micros(10)).await;
    42
}

// Static assertions for trait implementations.
assert_not_impl_any!(SmallFuture<'static, u64, 128>: Send, Unpin);
assert_impl_all!(SmallFutureSend<'static, u64, 128>: Send);
assert_not_impl_any!(SmallFutureSend<'static, u64, 128>: Unpin);

#[tokio::test]
async fn smoke_test() {
    // Test inline storage.
    let result = SmallFuture::<_, 32>::new(simple()).await;
    assert_eq!(result, 42, "Unexpected result from SmallFuture inline");
    let result = SmallFuture::<_, 256>::new(complex()).await;
    assert_eq!(result, 4950, "Unexpected result from SmallFuture inline");
    // Test heap storage for large size.
    let result = SmallFuture::<_, 16>::new(large_size()).await;
    assert_eq!(result, 42, "Unexpected result from SmallFuture heap");
    // Test heap storage for large alignment.
    let result = SmallFuture::<_, 1024>::new(large_align()).await;
    assert_eq!(result, 32640, "Unexpected result from SmallFuture heap");
    // Test non-Send future.
    let result = SmallFuture::<_, 32>::new(non_send_future()).await;
    assert_eq!(result, 42, "Unexpected result from SmallFuture non-Send");
}

static GLOBAL_TASK: OnceLock<SmallFutureSend<'static, u64, 128>> = OnceLock::new();

#[tokio::test]
async fn static_future_test() {
    let future = SmallFutureSend::<_, 128>::new(simple());
    // Stores a SmallFutureSend in a 'static context, verifying compatibility with 'static futures.
    GLOBAL_TASK.set(future).unwrap();
}

#[tokio::test]
async fn test_boxing_for_unpin() {
    // Verify that boxing allows SmallFuture to work in Unpin-requiring contexts.
    let future = SmallFutureSend::<u64, 128>::new(simple());
    let boxed: BoxFuture<u64> = Box::pin(future);
    assert_eq!(boxed.await, 42);
}
