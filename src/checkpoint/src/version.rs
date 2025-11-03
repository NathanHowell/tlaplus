//! Manifest version management for checkpoint metadata.

use std::{fmt, num::ParseIntError};

use rusqlite::{params, Connection, OptionalExtension};
use thiserror::Error;

const METADATA_TABLE: &str = "checkpoint_metadata";
/// Metadata key storing the manifest version string.
pub const MANIFEST_VERSION_KEY: &str = "manifest_version";

/// Current manifest version supported by this binary.
pub const CURRENT_MANIFEST_VERSION: ManifestVersion = ManifestVersion::new(1, 0);

/// Result alias for manifest version operations.
pub type VersionResult<T> = Result<T, VersionError>;

/// Structured manifest version consisting of major and minor components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ManifestVersion {
    pub major: u16,
    pub minor: u16,
}

impl ManifestVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }

    fn parse(raw: String) -> VersionResult<Self> {
        let (major, minor) = raw
            .split_once('.')
            .ok_or_else(|| VersionError::InvalidFormat { value: raw.clone() })?;
        let major_value = major
            .parse::<u16>()
            .map_err(|source| VersionError::ParseError {
                value: raw.clone(),
                source,
            })?;
        let minor_value = minor
            .parse::<u16>()
            .map_err(|source| VersionError::ParseError {
                value: raw.clone(),
                source,
            })?;
        Ok(Self::new(major_value, minor_value))
    }

    fn serialize(self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

impl fmt::Display for ManifestVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Errors emitted while working with manifest versions.
#[derive(Debug, Error)]
pub enum VersionError {
    #[error("checkpoint manifest version not recorded")]
    Missing,
    #[error("invalid checkpoint manifest version format '{value}'")]
    InvalidFormat { value: String },
    #[error("failed to parse checkpoint manifest version '{value}'")]
    ParseError {
        value: String,
        #[source]
        source: ParseIntError,
    },
    #[error("checkpoint manifest version {found} is not supported (expected {expected})")]
    Incompatible {
        expected: ManifestVersion,
        found: ManifestVersion,
    },
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
}

/// Writes the current manifest version for freshly created checkpoint stores.
pub fn persist_current_manifest(conn: &Connection) -> VersionResult<()> {
    persist_version(conn, CURRENT_MANIFEST_VERSION)
}

/// Ensures the stored manifest version matches the version supported by this binary.
pub fn ensure_current_manifest(conn: &Connection) -> VersionResult<()> {
    let stored = load_manifest_version(conn)?;
    if stored == CURRENT_MANIFEST_VERSION {
        Ok(())
    } else {
        Err(VersionError::Incompatible {
            expected: CURRENT_MANIFEST_VERSION,
            found: stored,
        })
    }
}

/// Convenience helper used during store bootstrap.
pub fn bootstrap_manifest_version(conn: &Connection, is_new: bool) -> VersionResult<()> {
    if is_new {
        persist_current_manifest(conn)?;
    } else {
        ensure_current_manifest(conn)?;
    }
    Ok(())
}

fn persist_version(conn: &Connection, version: ManifestVersion) -> VersionResult<()> {
    conn.execute(
        &format!("REPLACE INTO {METADATA_TABLE} (key, value) VALUES (?1, ?2)"),
        params![MANIFEST_VERSION_KEY, version.serialize()],
    )?;
    Ok(())
}

fn load_manifest_version(conn: &Connection) -> VersionResult<ManifestVersion> {
    let raw: Option<String> = conn
        .query_row(
            &format!("SELECT value FROM {METADATA_TABLE} WHERE key = ?1"),
            params![MANIFEST_VERSION_KEY],
            |row| row.get(0),
        )
        .optional()?;

    let value = raw.ok_or(VersionError::Missing)?;
    ManifestVersion::parse(value)
}
