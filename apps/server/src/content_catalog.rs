use std::sync::Arc;

use game_content::{ContentManifest, EntryKind, ManifestEntry};
use serde::Serialize;
use sqlx::PgPool;

#[derive(Clone)]
pub(crate) struct ContentCatalog {
    manifests: Arc<[ContentManifest]>,
}

impl ContentCatalog {
    pub(crate) fn new(manifests: Vec<ContentManifest>) -> Self {
        Self {
            manifests: manifests.into(),
        }
    }

    pub(crate) fn selection(
        &self,
        adventure_id: &str,
        manifest_digest: &str,
        ruleset_version: &str,
    ) -> Option<SelectedContent> {
        let manifest = self.manifests.iter().find(|manifest| {
            manifest.digest == manifest_digest && manifest.ruleset_version == ruleset_version
        })?;
        let adventure = manifest.entries.iter().find(|entry| {
            entry.kind == EntryKind::Adventure && entry.catalog_id.as_str() == adventure_id
        })?;

        Some(SelectedContent {
            adventure_id: adventure.catalog_id.as_str().to_owned(),
            adventure_name: entry_name(adventure),
            content_version: manifest.content_version.clone(),
            ruleset_version: manifest.ruleset_version.clone(),
            manifest_digest: manifest.digest.clone(),
            manifest_version: manifest.manifest_version,
            playable: manifest.playable && adventure.playable,
        })
    }

    pub(crate) fn options(&self) -> Vec<ContentManifestOption> {
        self.manifests
            .iter()
            .map(|manifest| ContentManifestOption {
                manifest_digest: manifest.digest.clone(),
                manifest_version: manifest.manifest_version,
                content_version: manifest.content_version.clone(),
                ruleset_version: manifest.ruleset_version.clone(),
                playable: manifest.playable,
                adventures: manifest
                    .entries
                    .iter()
                    .filter(|entry| entry.kind == EntryKind::Adventure)
                    .map(|entry| AdventureOption {
                        id: entry.catalog_id.as_str().to_owned(),
                        name: entry_name(entry),
                        playable: manifest.playable && entry.playable,
                    })
                    .collect(),
            })
            .collect()
    }

    pub(crate) async fn publish(&self, database: &PgPool) -> Result<(), sqlx::Error> {
        for manifest in self.manifests.iter() {
            let document = serde_json::to_string(manifest)
                .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
            publish_manifest(database, manifest, &document).await?;
        }
        Ok(())
    }
}

#[derive(Serialize)]
pub(crate) struct ContentManifestOption {
    manifest_digest: String,
    manifest_version: u16,
    content_version: String,
    ruleset_version: String,
    playable: bool,
    adventures: Vec<AdventureOption>,
}

#[derive(Serialize)]
struct AdventureOption {
    id: String,
    name: String,
    playable: bool,
}

#[derive(Clone)]
pub(crate) struct SelectedContent {
    pub(crate) adventure_id: String,
    pub(crate) adventure_name: String,
    pub(crate) content_version: String,
    pub(crate) ruleset_version: String,
    pub(crate) manifest_digest: String,
    pub(crate) manifest_version: u16,
    pub(crate) playable: bool,
}

async fn publish_manifest(
    database: &PgPool,
    manifest: &ContentManifest,
    document: &str,
) -> Result<(), sqlx::Error> {
    let manifest_version = i16::try_from(manifest.manifest_version).map_err(|_| {
        sqlx::Error::Protocol("manifest version does not fit PostgreSQL SMALLINT".to_owned())
    })?;
    let inserted = sqlx::query_scalar::<_, String>(
        r"
        INSERT INTO content_manifests (
            digest,
            manifest_version,
            content_version,
            ruleset_version,
            playable,
            document
        )
        VALUES ($1, $2, $3, $4, $5, $6::jsonb)
        ON CONFLICT (digest) DO NOTHING
        RETURNING digest
        ",
    )
    .bind(&manifest.digest)
    .bind(manifest_version)
    .bind(&manifest.content_version)
    .bind(&manifest.ruleset_version)
    .bind(manifest.playable)
    .bind(document)
    .fetch_optional(database)
    .await?;

    if inserted.is_none() {
        verify_immutable_manifest(database, manifest, manifest_version, document).await?;
    }

    Ok(())
}

async fn verify_immutable_manifest(
    database: &PgPool,
    manifest: &ContentManifest,
    manifest_version: i16,
    document: &str,
) -> Result<(), sqlx::Error> {
    let stored = sqlx::query_as::<_, (i16, String, String, bool, String)>(
        r"
        SELECT
            manifest_version,
            content_version,
            ruleset_version,
            playable,
            document::text
        FROM content_manifests
        WHERE digest = $1
        ",
    )
    .bind(&manifest.digest)
    .fetch_one(database)
    .await?;
    let requested_document: serde_json::Value =
        serde_json::from_str(document).map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    let stored_document: serde_json::Value = serde_json::from_str(&stored.4)
        .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
    if stored.0 != manifest_version
        || stored.1 != manifest.content_version
        || stored.2 != manifest.ruleset_version
        || stored.3 != manifest.playable
        || stored_document != requested_document
    {
        return Err(sqlx::Error::Protocol(
            "content manifest digest collision or immutable document mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn entry_name(entry: &ManifestEntry) -> String {
    entry
        .names
        .get("pt-BR")
        .or_else(|| entry.names.get("en"))
        .or_else(|| entry.names.values().next())
        .cloned()
        .unwrap_or_else(|| entry.catalog_id.as_str().to_owned())
}
