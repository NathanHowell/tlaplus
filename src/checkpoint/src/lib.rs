//! SQLite-backed checkpoint scaffolding.

use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("rusqlite error: {0}")]
    Database(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Metadata {
    spec_hash: String,
}

/// Open an in-memory checkpoint database and apply baseline pragmas.
pub fn open_ephemeral_checkpoint() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS checkpoint_metadata (spec_hash TEXT PRIMARY KEY)",
        params![],
    )?;
    let metadata = Metadata {
        spec_hash: "placeholder".to_string(),
    };
    let _payload = serde_json::to_string(&metadata)?;
    conn.execute(
        "INSERT OR REPLACE INTO checkpoint_metadata (spec_hash) VALUES (?1)",
        params![metadata.spec_hash],
    )?;
    tlc_util::initialize_runtime()?;
    tracing::debug!("checkpoint scaffold initialized");
    Ok(conn)
}
