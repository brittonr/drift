use super::*;
use object_store::memory::InMemory;

fn adapter() -> S3Adapter {
    S3Adapter {
        store: Arc::new(InMemory::new()),
        prefix: "drift/v1/users/alice".into(),
    }
}

#[tokio::test]
async fn conditional_metadata_rejects_stale_writers() {
    let store = adapter();
    assert!(store.load().await.unwrap().bytes.is_none());
    assert!(store
        .compare_and_swap(None, b"first".to_vec())
        .await
        .unwrap());
    assert!(!store
        .compare_and_swap(None, b"overwrite".to_vec())
        .await
        .unwrap());
    let first = store.load().await.unwrap();
    assert!(store
        .compare_and_swap(first.revision.as_deref(), b"second".to_vec())
        .await
        .unwrap());
    assert!(!store
        .compare_and_swap(first.revision.as_deref(), b"stale".to_vec())
        .await
        .unwrap());
    assert_eq!(store.load().await.unwrap().bytes.unwrap(), b"second");
}

#[tokio::test]
async fn blobs_require_matching_content_and_stay_in_the_account_prefix() {
    let store = adapter();
    let bytes = b"audio".to_vec();
    let hash = blake3::hash(&bytes).to_hex().to_string();
    assert!(store.get(&hash).await.unwrap().is_none());
    store.put(&hash, bytes.clone()).await.unwrap();
    assert_eq!(store.get(&hash).await.unwrap().unwrap(), bytes);
    assert!(store.put(&hash, b"changed".to_vec()).await.is_err());
    assert!(store.blob_path("../../other").is_err());
    assert_eq!(
        store.blob_path(&hash).unwrap().as_ref(),
        format!("drift/v1/users/alice/blobs/{hash}")
    );
}

#[tokio::test]
async fn metadata_size_limit_rejects_before_publication() {
    let store = adapter();
    assert!(store
        .compare_and_swap(None, vec![0; MAX_METADATA_BYTES + 1])
        .await
        .is_err());
    assert!(store.load().await.unwrap().bytes.is_none());
}
