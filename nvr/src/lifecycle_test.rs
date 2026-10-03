use super::*;

#[tokio::test]
async fn same_key_waits_while_other_keys_progress() {
    let locks = Arc::new(KeyedLocks::default());
    let first = locks.lock("first").await;
    let waiting_locks = locks.clone();
    let waiting = tokio::spawn(async move { waiting_locks.lock("first").await });
    let other = tokio::time::timeout(std::time::Duration::from_secs(1), locks.lock("other"))
        .await
        .unwrap();
    assert!(!waiting.is_finished());
    drop(first);
    let second = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap();
    drop(second);
    drop(other);
    let _next = locks.lock("next").await;
    assert_eq!(
        locks.locks.lock().unwrap().len(),
        1,
        "completed gates should not accumulate"
    );
}
