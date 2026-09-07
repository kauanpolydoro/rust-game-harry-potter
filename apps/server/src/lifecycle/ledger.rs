use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

use super::{PurgeError, PurgeProof, TombstoneLedger};

/// Durable local ledger outside PostgreSQL, for development and failure tests.
/// Use a separate durable volume which is never restored with operational data.
#[derive(Clone)]
pub struct FileLedger {
    directory: PathBuf,
}
impl FileLedger {
    #[must_use]
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }
}

impl TombstoneLedger for FileLedger {
    async fn maintain(&self, now_ms: i64) -> Result<(), PurgeError> {
        let directory = self.directory.clone();
        tokio::task::spawn_blocking(move || expire_local_proofs(&directory, now_ms))
            .await
            .map_err(|_| PurgeError::Ledger)?
            .map_err(|_| PurgeError::Ledger)
    }

    async fn record(&self, key: &str, proof: &PurgeProof) -> Result<(), PurgeError> {
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PurgeError::Ledger);
        }
        let directory = self.directory.clone();
        let name = format!(
            "{key}.{}.json",
            if proof.completed_at_ms.is_some() {
                "complete"
            } else {
                "tombstone"
            }
        );
        let bytes = serde_json::to_vec(proof).map_err(|_| PurgeError::Ledger)?;
        tokio::task::spawn_blocking(move || durable_write(&directory, &name, &bytes))
            .await
            .map_err(|_| PurgeError::Ledger)?
            .map_err(|_| PurgeError::Ledger)
    }
}

fn durable_write(directory: &std::path::Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    fs::create_dir_all(directory)?;
    // Persist every newly created directory entry, including relative paths,
    // before PostgreSQL can rely on the external acknowledgement.
    for ancestor in directory.ancestors() {
        let path = if ancestor.as_os_str().is_empty() {
            std::path::Path::new(".")
        } else {
            ancestor
        };
        File::open(path)?.sync_all()?;
    }
    let path = directory.join(name);
    let temporary = directory.join(format!(".{}.pending", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        match fs::hard_link(&temporary, &path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if fs::read(&path)? != bytes {
                    return Err(io::Error::other("conflicting ledger proof"));
                }
            }
            Err(error) => return Err(error),
        }
        File::open(directory)?.sync_all()
    })();
    let cleanup = fs::remove_file(&temporary);
    result.and(cleanup)?;
    File::open(directory)?.sync_all()
}

/// Production ledger in a dedicated S3 bucket, outside the operational restore.
#[derive(Clone)]
pub struct S3Ledger {
    client: aws_sdk_s3::Client,
    bucket: String,
}

impl S3Ledger {
    #[must_use]
    pub fn new(client: aws_sdk_s3::Client, bucket: String) -> Self {
        Self { client, bucket }
    }

    /// Requires the dedicated bucket's declared 14-day, unversioned retention.
    ///
    /// # Errors
    /// Fails closed when retention cannot be inspected or is not configured.
    pub async fn validate_retention(&self) -> Result<(), PurgeError> {
        let versioning = self
            .client
            .get_bucket_versioning()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|_| PurgeError::Ledger)?;
        if versioning.status().is_some() {
            return Err(PurgeError::Ledger);
        }
        let lifecycle = self
            .client
            .get_bucket_lifecycle_configuration()
            .bucket(&self.bucket)
            .send()
            .await
            .map_err(|_| PurgeError::Ledger)?;
        let rules = lifecycle.rules();
        if rules.len() != 1 {
            return Err(PurgeError::Ledger);
        }
        let rule = &rules[0];
        let all_objects = rule.filter().is_some_and(|filter| {
            filter.prefix().is_none_or(str::is_empty)
                && filter.tag().is_none()
                && filter.and().is_none()
                && filter.object_size_greater_than().is_none()
                && filter.object_size_less_than().is_none()
        });
        if rule.status() != &aws_sdk_s3::types::ExpirationStatus::Enabled
            || !all_objects
            || rule
                .expiration()
                .and_then(aws_sdk_s3::types::LifecycleExpiration::days)
                != Some(14)
        {
            return Err(PurgeError::Ledger);
        }
        Ok(())
    }
}

impl TombstoneLedger for S3Ledger {
    async fn record(&self, key: &str, proof: &PurgeProof) -> Result<(), PurgeError> {
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PurgeError::Ledger);
        }
        let name = format!(
            "{key}.{}.json",
            if proof.completed_at_ms.is_some() {
                "complete"
            } else {
                "tombstone"
            }
        );
        let bytes = serde_json::to_vec(proof).map_err(|_| PurgeError::Ledger)?;
        // The same immutable payload is safe to overwrite on an uncertain ACK.
        // Rewriting also renews the minimum 14-day retention before root removal.
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(name)
            .content_type("application/json")
            .server_side_encryption(aws_sdk_s3::types::ServerSideEncryption::Aes256)
            .body(bytes.into())
            .send()
            .await
            .map_err(|_| PurgeError::Ledger)?;
        Ok(())
    }
}

fn expire_local_proofs(directory: &std::path::Path, now_ms: i64) -> io::Result<()> {
    const RETENTION_MS: i64 = 14 * 24 * 60 * 60 * 1000;
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(key) = name
            .to_str()
            .and_then(|name| name.strip_suffix(".complete.json"))
        else {
            continue;
        };
        if key.len() != 64 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            continue;
        }
        let bytes = match fs::read(entry.path()) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let proof: PurgeProof = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        let written_ms = i64::try_from(
            entry
                .metadata()?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(io::Error::other)?
                .as_millis(),
        )
        .map_err(io::Error::other)?;
        if proof.completed_at_ms.is_some_and(|completed| {
            now_ms.saturating_sub(completed.max(written_ms)) >= RETENTION_MS
        }) {
            remove_if_present(&directory.join(format!("{key}.tombstone.json")))?;
            remove_if_present(&entry.path())?;
        }
    }
    File::open(directory)?.sync_all()
}

fn remove_if_present(path: &std::path::Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
