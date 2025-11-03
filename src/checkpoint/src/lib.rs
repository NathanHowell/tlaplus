//! SQLite-backed checkpoint scaffolding.

pub mod snapshot;
pub mod version;

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use thiserror::Error;
use ulid::Ulid;

const DEFAULT_SCHEMA_VERSION: u32 = 1;

/// Default WAL pages between automatic checkpoints.
const DEFAULT_AUTOCHECKPOINT_PAGES: i64 = 32_768; // ~128 MiB at 4 KiB pages.
/// Default mmap window recommended for large checkpoint blobs.
const DEFAULT_MMAP_SIZE_BYTES: i64 = 256 * 1024 * 1024; // 256 MiB.
/// Default cache size (negative values interpreted as kibibytes).
const DEFAULT_CACHE_SIZE_KIB: i32 = -256 * 1024; // 256 MiB cache.

/// High-level errors produced while preparing a checkpoint store.
#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("checkpoint path must not be empty")]
    EmptyPath,
    #[error("failed to create checkpoint directory '{path}'")]
    CreateDirectory {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("rusqlite error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Tuning parameters applied when opening a SQLite checkpoint database.
#[derive(Debug, Clone)]
pub struct StoreOptions {
    /// WAL pages written before SQLite checkpoints in-process.
    pub wal_autocheckpoint_pages: i64,
    /// Busy timeout enforced to smooth concurrent access.
    pub busy_timeout: Duration,
    /// SQLite synchronous mode (see [`SynchronousMode`]).
    pub synchronous: SynchronousMode,
    /// Optional page size applied to freshly created databases.
    pub page_size: Option<i64>,
    /// Optional mmap window in bytes.
    pub mmap_size: Option<i64>,
    /// Cache size expressed in kibibytes (negative values indicate kibibytes).
    pub cache_size_kib: Option<i32>,
    /// Temporary store target (`FILE` or `MEMORY`).
    pub temp_store: TempStore,
    /// Journal size limit in bytes.
    pub journal_size_limit: Option<i64>,
}

impl Default for StoreOptions {
    fn default() -> Self {
        Self {
            wal_autocheckpoint_pages: DEFAULT_AUTOCHECKPOINT_PAGES,
            busy_timeout: Duration::from_secs(30),
            synchronous: SynchronousMode::Normal,
            page_size: Some(4096),
            mmap_size: Some(DEFAULT_MMAP_SIZE_BYTES),
            cache_size_kib: Some(DEFAULT_CACHE_SIZE_KIB),
            temp_store: TempStore::Memory,
            journal_size_limit: None,
        }
    }
}

/// SQLite synchronous pragma options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SynchronousMode {
    Off,
    Normal,
    Full,
    Extra,
}

impl SynchronousMode {
    fn as_str(self) -> &'static str {
        match self {
            SynchronousMode::Off => "OFF",
            SynchronousMode::Normal => "NORMAL",
            SynchronousMode::Full => "FULL",
            SynchronousMode::Extra => "EXTRA",
        }
    }
}

/// SQLite temp store pragma values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TempStore {
    Default,
    File,
    Memory,
}

impl TempStore {
    fn as_pragma_value(self) -> i64 {
        match self {
            TempStore::Default => 0,
            TempStore::File => 1,
            TempStore::Memory => 2,
        }
    }
}

pub use snapshot::{SnapshotError, SnapshotMetadata, SnapshotResult};

/// Handle to a checkpoint database connection with normalized tuning.
#[derive(Debug)]
pub struct CheckpointStore {
    conn: Connection,
    path: Option<PathBuf>,
}

impl CheckpointStore {
    /// Open (or create) a SQLite checkpoint database on disk.
    pub fn open<P: AsRef<Path>>(path: P, options: StoreOptions) -> Result<Self> {
        let path = path.as_ref();
        if path.as_os_str().is_empty() {
            return Err(CheckpointError::EmptyPath.into());
        }

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|source| CheckpointError::CreateDirectory {
                    path: parent.display().to_string(),
                    source,
                })?;
            }
        }

        let is_new = !path.exists();
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)
            .map_err(CheckpointError::from)
            .context("failed to open checkpoint database")?;
        prepare_connection(&conn, &options, is_new)?;
        install_base_schema(&conn, is_new).with_context(|| {
            format!("unable to prepare checkpoint store at '{}'", path.display())
        })?;

        tracing::debug!(
            path = %path.display(),
            schema_version = DEFAULT_SCHEMA_VERSION,
            journal_mode = %pragma_string(&conn, "journal_mode")?,
            page_size = %pragma_integer(&conn, "page_size")?,
            wal_autocheckpoint = %pragma_integer(&conn, "wal_autocheckpoint")?,
            "checkpoint store ready"
        );

        Ok(Self {
            conn,
            path: Some(path.to_path_buf()),
        })
    }

    /// Create an in-memory checkpoint store useful for testing.
    pub fn ephemeral(options: StoreOptions) -> Result<Self> {
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let unique = Ulid::new();
        let dsn = format!(
            "file:tlc_ephemeral_{unique}?mode=memory&cache=shared",
            unique = unique
        );
        let conn = Connection::open_with_flags(&dsn, flags)
            .map_err(CheckpointError::from)
            .context("failed to open in-memory checkpoint database")?;
        prepare_connection(&conn, &options, true)?;
        install_base_schema(&conn, true)?;

        tracing::debug!(
            schema_version = DEFAULT_SCHEMA_VERSION,
            journal_mode = %pragma_string(&conn, "journal_mode")?,
            page_size = %pragma_integer(&conn, "page_size")?,
            wal_autocheckpoint = %pragma_integer(&conn, "wal_autocheckpoint")?,
            "ephemeral checkpoint store ready"
        );

        Ok(Self { conn, path: None })
    }

    /// Borrow the underlying SQLite connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Borrow the underlying SQLite connection mutably.
    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    /// Consume the store and return the raw connection.
    pub fn into_inner(self) -> Connection {
        self.conn
    }

    /// Location of the on-disk database, when available.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Persists a checkpoint snapshot manifest to the metadata table.
    pub fn write_snapshot_metadata(&mut self, metadata: &SnapshotMetadata) -> Result<()> {
        Ok(metadata.persist(self.connection_mut())?)
    }

    /// Loads the active checkpoint snapshot manifest from the metadata table.
    pub fn snapshot_metadata(&self) -> Result<SnapshotMetadata> {
        Ok(SnapshotMetadata::load(self.connection())?)
    }

    /// Returns true if checkpoint snapshot metadata has been recorded.
    pub fn has_snapshot_metadata(&self) -> Result<bool> {
        Ok(SnapshotMetadata::exists(self.connection())?)
    }
}

fn prepare_connection(conn: &Connection, options: &StoreOptions, is_new: bool) -> Result<()> {
    conn.busy_timeout(options.busy_timeout)?;

    if is_new {
        if let Some(page_size) = options.page_size {
            conn.pragma_update(None, "page_size", &page_size)?;
        }
    }

    conn.pragma_update(None, "journal_mode", &"WAL")?;
    conn.pragma_update(None, "foreign_keys", &1)?;
    conn.pragma_update(None, "synchronous", &options.synchronous.as_str())?;
    conn.pragma_update(
        None,
        "wal_autocheckpoint",
        &options.wal_autocheckpoint_pages,
    )?;

    if let Some(mmap_size) = options.mmap_size {
        conn.pragma_update(None, "mmap_size", &mmap_size)?;
    }
    if let Some(cache_size) = options.cache_size_kib {
        conn.pragma_update(None, "cache_size", &cache_size)?;
    }
    if let Some(journal_size_limit) = options.journal_size_limit {
        conn.pragma_update(None, "journal_size_limit", &journal_size_limit)?;
    }

    let temp_store = options.temp_store.as_pragma_value();
    conn.pragma_update(None, "temp_store", &temp_store)?;
    Ok(())
}

fn install_base_schema(conn: &Connection, is_new: bool) -> Result<()> {
    conn.execute(
        r#"
        CREATE TABLE IF NOT EXISTS checkpoint_metadata (
            key TEXT PRIMARY KEY,
            value BLOB NOT NULL
        )
        "#,
        [],
    )?;

    conn.execute(
        "REPLACE INTO checkpoint_metadata (key, value) VALUES ('schema_version', ?1)",
        params![DEFAULT_SCHEMA_VERSION.to_string()],
    )?;
    version::bootstrap_manifest_version(conn, is_new)?;
    Ok(())
}

fn pragma_string(conn: &Connection, name: &str) -> Result<String> {
    conn.query_row::<String, _, _>(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(Into::into)
}

fn pragma_integer(conn: &Connection, name: &str) -> Result<i64> {
    conn.query_row::<i64, _, _>(&format!("PRAGMA {name}"), [], |row| row.get(0))
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version;
    use rusqlite::Connection;

    #[test]
    fn ephemeral_store_configures_wal() {
        let store = CheckpointStore::ephemeral(StoreOptions::default()).expect("store initialized");
        let conn = store.connection();

        let mode = pragma_string(conn, "journal_mode").expect("query journal mode");
        let normalized = mode.to_ascii_uppercase();
        assert!(
            normalized == "WAL" || normalized == "MEMORY",
            "unexpected journal mode for ephemeral store: {normalized}"
        );

        let wal_autocheckpoint =
            pragma_integer(conn, "wal_autocheckpoint").expect("wal autocheckpoint");
        assert_eq!(wal_autocheckpoint, DEFAULT_AUTOCHECKPOINT_PAGES);
    }

    #[test]
    fn schema_version_written() {
        let store = CheckpointStore::ephemeral(StoreOptions::default()).expect("store initialized");
        let version: String = store
            .connection()
            .query_row(
                "SELECT value FROM checkpoint_metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema version exists");
        assert_eq!(version, DEFAULT_SCHEMA_VERSION.to_string());
    }

    #[test]
    fn manifest_version_written_for_new_store() {
        let store = CheckpointStore::ephemeral(StoreOptions::default()).expect("store initialized");
        let version_value: String = store
            .connection()
            .query_row(
                "SELECT value FROM checkpoint_metadata WHERE key = ?1",
                params![version::MANIFEST_VERSION_KEY],
                |row| row.get(0),
            )
            .expect("manifest version exists");
        assert_eq!(version_value, version::CURRENT_MANIFEST_VERSION.to_string());
    }

    #[test]
    fn install_schema_rejects_incompatible_manifest_version() {
        let conn = Connection::open_in_memory().expect("open in-memory database");
        conn.execute(
            r#"
            CREATE TABLE checkpoint_metadata (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            )
            "#,
            [],
        )
        .expect("create metadata table");
        conn.execute(
            "INSERT INTO checkpoint_metadata (key, value) VALUES (?1, ?2)",
            params![version::MANIFEST_VERSION_KEY, "2.0"],
        )
        .expect("insert mismatched manifest version");
        conn.execute(
            "INSERT INTO checkpoint_metadata (key, value) VALUES ('schema_version', '1')",
            [],
        )
        .expect("insert schema version");

        let err = install_base_schema(&conn, false).expect_err("manifest version mismatch");
        let version_error = err
            .downcast_ref::<version::VersionError>()
            .expect("expected version error");
        assert!(matches!(
            version_error,
            version::VersionError::Incompatible { found, .. }
            if found.major == 2 && found.minor == 0
        ));
    }
}
