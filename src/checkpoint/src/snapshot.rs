//! Helpers for managing checkpoint snapshot metadata manifests.

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use thiserror::Error;
use tlc_util::{Blake3Digest, ValidationError};
use ulid::Ulid;

const METADATA_TABLE: &str = "checkpoint_metadata";
const KEY_CHECKPOINT_ID: &str = "checkpoint_id";
const KEY_RUN_ID: &str = "run_id";
const KEY_SPEC_HASH: &str = "spec_hash";
const KEY_CREATED_AT: &str = "created_at";
const KEY_PAGE_SIZE: &str = "sqlite_page_size";

/// Result type used by snapshot metadata helpers.
pub type SnapshotResult<T> = Result<T, SnapshotError>;

/// Errors that can arise while reading or writing checkpoint snapshot metadata.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("metadata key '{0}' missing from checkpoint store")]
    MissingKey(&'static str),
    #[error("invalid ULID stored for '{field}': {value}")]
    InvalidUlid {
        field: &'static str,
        value: String,
        #[source]
        source: ulid::DecodeError,
    },
    #[error("invalid timestamp stored for '{field}': {value}")]
    InvalidTimestamp {
        field: &'static str,
        value: String,
        #[source]
        source: chrono::ParseError,
    },
    #[error("invalid page size value '{value}'")]
    InvalidPageSizeValue {
        value: String,
        #[source]
        source: std::num::ParseIntError,
    },
    #[error("page size must be greater than zero (got {0})")]
    NonPositivePageSize(u32),
    #[error("page size reported by SQLite is out of range (got {0})")]
    PageSizeOutOfRange(i64),
    #[error("invalid spec hash stored in checkpoint metadata")]
    InvalidSpecHash(#[from] ValidationError),
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Immutable manifest describing a persisted checkpoint snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMetadata {
    pub checkpoint_id: Ulid,
    pub run_id: Ulid,
    pub spec_hash: Blake3Digest,
    pub created_at: DateTime<Utc>,
    pub sqlite_page_size: u32,
}

impl SnapshotMetadata {
    /// Creates a new metadata record with explicit fields.
    pub fn new(
        checkpoint_id: Ulid,
        run_id: Ulid,
        spec_hash: Blake3Digest,
        created_at: DateTime<Utc>,
        sqlite_page_size: u32,
    ) -> SnapshotResult<Self> {
        if sqlite_page_size == 0 {
            return Err(SnapshotError::NonPositivePageSize(sqlite_page_size));
        }
        Ok(Self {
            checkpoint_id,
            run_id,
            spec_hash,
            created_at,
            sqlite_page_size,
        })
    }

    /// Captures metadata for a newly created checkpoint using the active store connection.
    pub fn fresh(
        conn: &Connection,
        checkpoint_id: Ulid,
        run_id: Ulid,
        spec_hash: Blake3Digest,
    ) -> SnapshotResult<Self> {
        let page_size = query_page_size(conn)?;
        Self::new(checkpoint_id, run_id, spec_hash, Utc::now(), page_size)
    }

    /// Persists the metadata manifest to the checkpoint metadata table.
    pub fn persist(&self, conn: &mut Connection) -> SnapshotResult<()> {
        let tx = conn.transaction()?;
        upsert_metadata(&tx, KEY_CHECKPOINT_ID, &self.checkpoint_id.to_string())?;
        upsert_metadata(&tx, KEY_RUN_ID, &self.run_id.to_string())?;
        upsert_metadata(&tx, KEY_SPEC_HASH, &self.spec_hash.to_hex())?;

        let created_at = self.created_at.to_rfc3339_opts(SecondsFormat::Micros, true);
        upsert_metadata(&tx, KEY_CREATED_AT, &created_at)?;
        upsert_metadata(&tx, KEY_PAGE_SIZE, &self.sqlite_page_size.to_string())?;
        tx.commit()?;
        Ok(())
    }

    /// Loads the persisted metadata manifest from the checkpoint store.
    pub fn load(conn: &Connection) -> SnapshotResult<Self> {
        let checkpoint_id = parse_ulid(KEY_CHECKPOINT_ID, fetch_value(conn, KEY_CHECKPOINT_ID)?)?;
        let run_id = parse_ulid(KEY_RUN_ID, fetch_value(conn, KEY_RUN_ID)?)?;
        let spec_hash = parse_spec_hash(fetch_value(conn, KEY_SPEC_HASH)?)?;
        let created_at = parse_timestamp(fetch_value(conn, KEY_CREATED_AT)?)?;
        let sqlite_page_size = parse_page_size_string(fetch_value(conn, KEY_PAGE_SIZE)?)?;

        Self::new(
            checkpoint_id,
            run_id,
            spec_hash,
            created_at,
            sqlite_page_size,
        )
    }

    /// Determines whether a checkpoint manifest has been recorded.
    pub fn exists(conn: &Connection) -> SnapshotResult<bool> {
        let value: Option<i32> = conn
            .query_row(
                &format!("SELECT 1 FROM {METADATA_TABLE} WHERE key = ?1 LIMIT 1"),
                params![KEY_CHECKPOINT_ID],
                |row| row.get(0),
            )
            .optional()?;
        Ok(value.is_some())
    }
}

fn upsert_metadata(tx: &Transaction<'_>, key: &str, value: &str) -> rusqlite::Result<()> {
    tx.execute(
        &format!("REPLACE INTO {METADATA_TABLE} (key, value) VALUES (?1, ?2)"),
        params![key, value],
    )?;
    Ok(())
}

fn fetch_value(conn: &Connection, key: &'static str) -> SnapshotResult<String> {
    conn.query_row::<String, _, _>(
        &format!("SELECT value FROM {METADATA_TABLE} WHERE key = ?1"),
        params![key],
        |row| row.get(0),
    )
    .map_err(|err| match err {
        rusqlite::Error::QueryReturnedNoRows => SnapshotError::MissingKey(key),
        other => SnapshotError::from(other),
    })
}

fn parse_ulid(field: &'static str, value: String) -> SnapshotResult<Ulid> {
    Ulid::from_string(&value).map_err(|source| SnapshotError::InvalidUlid {
        field,
        value,
        source,
    })
}

fn parse_spec_hash(value: String) -> SnapshotResult<Blake3Digest> {
    Ok(Blake3Digest::from_hex(&value).map_err(SnapshotError::InvalidSpecHash)?)
}

fn parse_timestamp(value: String) -> SnapshotResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|source| SnapshotError::InvalidTimestamp {
            field: KEY_CREATED_AT,
            value,
            source,
        })
}

fn parse_page_size_string(value: String) -> SnapshotResult<u32> {
    let parsed = value
        .parse::<u32>()
        .map_err(|source| SnapshotError::InvalidPageSizeValue {
            value: value.clone(),
            source,
        })?;
    if parsed == 0 {
        return Err(SnapshotError::NonPositivePageSize(parsed));
    }
    Ok(parsed)
}

fn query_page_size(conn: &Connection) -> SnapshotResult<u32> {
    let raw: i64 = conn.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    if raw <= 0 || raw > u32::MAX as i64 {
        return Err(SnapshotError::PageSizeOutOfRange(raw));
    }
    Ok(raw as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StoreOptions;
    use chrono::TimeZone;
    use tlc_util::Blake3Digest;

    const SPEC_HASH_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn persist_and_load_round_trip() {
        let mut store =
            crate::CheckpointStore::ephemeral(StoreOptions::default()).expect("store initialized");

        let checkpoint_id = Ulid::new();
        let run_id = Ulid::new();
        let spec_hash = Blake3Digest::from_hex(SPEC_HASH_HEX).expect("valid hash");
        let created_at = Utc
            .with_ymd_and_hms(2025, 11, 2, 3, 15, 0)
            .single()
            .expect("valid timestamp");
        let sqlite_page_size = 4096;

        let metadata = SnapshotMetadata::new(
            checkpoint_id,
            run_id,
            spec_hash,
            created_at,
            sqlite_page_size,
        )
        .expect("metadata constructed");
        metadata
            .persist(store.connection_mut())
            .expect("persist metadata");

        let loaded = SnapshotMetadata::load(store.connection()).expect("load metadata");
        assert_eq!(loaded, metadata);
    }

    #[test]
    fn load_without_manifest_fails() {
        let store =
            crate::CheckpointStore::ephemeral(StoreOptions::default()).expect("store initialized");
        let conn = store.connection();

        let err = SnapshotMetadata::load(conn).expect_err("metadata missing");
        assert!(matches!(err, SnapshotError::MissingKey(KEY_CHECKPOINT_ID)));
    }

    #[test]
    fn fresh_uses_connection_page_size() {
        let store =
            crate::CheckpointStore::ephemeral(StoreOptions::default()).expect("store initialized");
        let conn = store.connection();

        let checkpoint_id = Ulid::new();
        let run_id = Ulid::new();
        let spec_hash = Blake3Digest::from_hex(SPEC_HASH_HEX).expect("valid hash");

        let metadata = SnapshotMetadata::fresh(conn, checkpoint_id, run_id, spec_hash)
            .expect("fresh metadata");
        assert!(metadata.sqlite_page_size > 0);
    }
}
