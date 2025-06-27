use stack_future::StackFuture;

async fn test() -> u64 {
    42
}

#[tokio::test]
async fn smoke_test() {
    let future = StackFuture::<_, 32>::new(test());
    assert!(future.is_some(), "Failed to create StackFuture");
    let future = future.unwrap();
    let result = future.await;
    assert_eq!(result, 42, "Unexpected result from StackFuture");
}
