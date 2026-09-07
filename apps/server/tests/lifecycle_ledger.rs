use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
use axum::{
    Router,
    body::to_bytes,
    extract::{Request, State},
    http::StatusCode,
    routing::any,
};
use harry_potter_server::lifecycle::{
    FileLedger, PurgeError, PurgeProof, S3Ledger, TombstoneLedger,
};
use uuid::Uuid;

fn proof(completed: Option<i64>) -> PurgeProof {
    PurgeProof {
        version: 1,
        expires_at_ms: 1000,
        detected_at_ms: 2000,
        completed_at_ms: completed,
    }
}

#[tokio::test]
async fn restore_reads_durable_tombstones_and_rejects_missing_or_corrupt_ledgers() {
    let directory =
        std::env::temp_dir().join(format!("restore_ledger_{}", Uuid::new_v4().simple()));
    let ledger = FileLedger::new(directory.clone());
    let key = "c".repeat(64);
    assert_eq!(ledger.lookup(&key).await, Err(PurgeError::Ledger));
    ledger.record(&key, &proof(None)).await.unwrap();
    assert_eq!(ledger.lookup(&key).await.unwrap(), Some(proof(None)));
    assert_eq!(ledger.lookup(&"d".repeat(64)).await.unwrap(), None);
    std::fs::write(directory.join(format!("{key}.tombstone.json")), b"{}").unwrap();
    assert_eq!(ledger.lookup(&key).await, Err(PurgeError::Ledger));
    assert_eq!(ledger.lookup("../escape").await, Err(PurgeError::Ledger));
    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
async fn local_proofs_survive_reopen_reject_conflicts_and_expire_only_after_fourteen_days() {
    let directory = std::env::temp_dir().join(format!("ledger_{}", Uuid::new_v4().simple()));
    let ledger = FileLedger::new(directory.clone());
    let key = "a".repeat(64);
    ledger.record(&key, &proof(None)).await.unwrap();
    FileLedger::new(directory.clone())
        .record(&key, &proof(None))
        .await
        .unwrap();
    let mut conflict = proof(None);
    conflict.detected_at_ms = 3000;
    assert_eq!(
        ledger.record(&key, &conflict).await,
        Err(PurgeError::Ledger)
    );
    ledger.maintain(2_000_000_000).await.unwrap();
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        1,
        "unfinished proofs must remain"
    );
    ledger.record(&key, &proof(Some(4000))).await.unwrap();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    ledger.maintain(now).await.unwrap();
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
        2,
        "a late completion ACK starts a fresh retention window"
    );
    for entry in std::fs::read_dir(&directory).unwrap() {
        std::fs::File::open(entry.unwrap().path())
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_millis(4000)),
            )
            .unwrap();
    }

    ledger.maintain(1_209_603_999).await.unwrap();
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
    ledger.maintain(1_209_604_000).await.unwrap();
    assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
    std::fs::remove_dir(directory).unwrap();
}

#[derive(Clone, Default)]
struct ObjectStore {
    objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    fail: Arc<std::sync::atomic::AtomicBool>,
}
async fn s3_request(State(store): State<ObjectStore>, request: Request) -> (StatusCode, String) {
    if store.fail.load(std::sync::atomic::Ordering::SeqCst) {
        return (
            StatusCode::FORBIDDEN,
            "<Error><Code>AccessDenied</Code></Error>".into(),
        );
    }
    if request.uri().query() == Some("versioning") {
        return (
            StatusCode::OK,
            "<VersioningConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"/>".into(),
        );
    }
    if request.uri().query() == Some("lifecycle") {
        return (StatusCode::OK, "<LifecycleConfiguration><Rule><ID>retention</ID><Status>Enabled</Status><Filter><Prefix></Prefix></Filter><Expiration><Days>14</Days></Expiration></Rule></LifecycleConfiguration>".into());
    }
    assert!(
        request.headers()["authorization"]
            .to_str()
            .unwrap()
            .starts_with("AWS4-HMAC-SHA256 ")
    );
    let path = request.uri().path().to_owned();
    if request.method() == "GET" {
        return store.objects.lock().unwrap().get(&path).map_or_else(
            || {
                (
                    StatusCode::NOT_FOUND,
                    "<Error><Code>NoSuchKey</Code></Error>".into(),
                )
            },
            |bytes| (StatusCode::OK, String::from_utf8(bytes.clone()).unwrap()),
        );
    }
    assert_eq!(request.method(), "PUT");
    assert_eq!(request.headers()["x-amz-server-side-encryption"], "AES256");
    let bytes = to_bytes(request.into_body(), 4096).await.unwrap().to_vec();
    store.objects.lock().unwrap().insert(path, bytes);
    (StatusCode::OK, String::new())
}

#[tokio::test]
async fn s3_adapter_uses_signed_encrypted_durable_puts_and_propagates_storage_failures() {
    let store = ObjectStore::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/{*path}", any(s3_request))
        .with_state(store.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let config = aws_sdk_s3::Config::builder()
        .behavior_version(BehaviorVersion::latest())
        .region(Region::new("sa-east-1"))
        .credentials_provider(Credentials::new(
            "fixture-access",
            "fixture-secret",
            None,
            None,
            "fixture",
        ))
        .endpoint_url(format!("http://{address}"))
        .force_path_style(true)
        .build();
    let ledger = S3Ledger::new(aws_sdk_s3::Client::from_conf(config), "proofs".into());
    ledger.validate_retention().await.unwrap();
    let key = "b".repeat(64);
    ledger.record(&key, &proof(None)).await.unwrap();
    ledger.record(&key, &proof(None)).await.unwrap();
    ledger.record(&key, &proof(Some(4000))).await.unwrap();
    assert_eq!(ledger.lookup(&key).await.unwrap(), Some(proof(None)));
    assert_eq!(ledger.lookup(&"e".repeat(64)).await.unwrap(), None);
    {
        let objects = store.objects.lock().unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(
            serde_json::from_slice::<PurgeProof>(
                &objects[&format!("/proofs/{key}.tombstone.json")]
            )
            .unwrap(),
            proof(None)
        );
    }
    store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        ledger.record(&key, &proof(None)).await,
        Err(PurgeError::Ledger)
    );
    assert_eq!(ledger.validate_retention().await, Err(PurgeError::Ledger));
    assert_eq!(ledger.lookup(&key).await, Err(PurgeError::Ledger));
    server.abort();
}

#[tokio::test]
async fn a_restored_purge_can_publish_a_new_verification_without_conflicting_with_old_completion() {
    let directory =
        std::env::temp_dir().join(format!("restored_completion_{}", Uuid::new_v4().simple()));
    let ledger = FileLedger::new(directory.clone());
    let key = "f".repeat(64);
    ledger.record(&key, &proof(None)).await.unwrap();
    ledger.record(&key, &proof(Some(4000))).await.unwrap();
    ledger.record(&key, &proof(Some(5000))).await.unwrap();
    ledger.record(&key, &proof(Some(4000))).await.unwrap();
    let completed: PurgeProof = serde_json::from_slice(
        &std::fs::read(directory.join(format!("{key}.complete.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(completed, proof(Some(5000)));
    std::fs::remove_dir_all(directory).unwrap();
}
