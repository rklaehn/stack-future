use stack_future::{CreateError, StackFuture};

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
    let sum = buffer.buffer.iter().map(|&x| x as u64).sum();
    sum
}

#[tokio::test]
async fn smoke_test() {
    let result = StackFuture::<_, 32>::new(simple()).unwrap().await;
    assert_eq!(result, 42, "Unexpected result from StackFuture");
    let result = StackFuture::<_, 256>::new(complex()).unwrap().await;
    assert_eq!(result, 4950, "Unexpected result from StackFuture");
    let res = StackFuture::<_, 16>::new(complex());
    assert!(
        matches!(res, Err(CreateError::SizeTooLarge { .. })),
        "Expected error for too large future"
    );
    let res = StackFuture::<_, 1024>::new(large_align());
    assert!(
        matches!(res, Err(CreateError::AlignmentMismatch { .. })),
        "Expected error for misaligned future"
    );
}
