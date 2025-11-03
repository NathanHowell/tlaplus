use std::{
    convert::TryFrom,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use rusqlite::params;
use tempfile::TempDir;
use tlc_checkpoint::{CheckpointStore, SnapshotMetadata, StoreOptions};
use tlc_util::Blake3Digest;
use ulid::Ulid;

#[derive(Clone, Copy, Debug)]
struct PlaceholderScenario {
    label: &'static str,
    target_bytes: u64,
}

const PLACEHOLDER_SCENARIOS: [PlaceholderScenario; 2] = [
    // TODO(T065-followup): Bump to 10 GB and 100 GB targets once soak harness is optimized.
    PlaceholderScenario {
        label: "100mb",
        target_bytes: 100 * 1024 * 1024,
    },
    PlaceholderScenario {
        label: "1gb",
        target_bytes: 1 * 1024 * 1024 * 1024,
    },
];

#[derive(Debug)]
struct GeneratedCheckpoint {
    temp_dir: TempDir,
    db_path: PathBuf,
    metadata: SnapshotMetadata,
    rows_written: u64,
    bytes_written: u64,
}

const PLACEHOLDER_SPEC_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

#[test]
#[ignore = "checkpoint soak suite runs via scripts/dev/check-all.sh"]
fn placeholder_large_checkpoint_soaks_resume() -> Result<()> {
    for scenario in PLACEHOLDER_SCENARIOS {
        let checkpoint = generate_placeholder_checkpoint(scenario)
            .with_context(|| format!("generate placeholder checkpoint for {}", scenario.label))?;

        assert_eq!(
            checkpoint.bytes_written, scenario.target_bytes,
            "placeholder generator should write the full target size"
        );

        // Re-open the database to ensure resume flows observe persisted metadata.
        let store = CheckpointStore::open(&checkpoint.db_path, StoreOptions::default())
            .with_context(|| format!("re-open checkpoint {}", scenario.label))?;
        assert!(
            store.has_snapshot_metadata()?,
            "snapshot metadata should exist for {}",
            scenario.label
        );
        let loaded_metadata = store
            .snapshot_metadata()
            .with_context(|| format!("load snapshot metadata for {}", scenario.label))?;
        assert_eq!(
            loaded_metadata, checkpoint.metadata,
            "loaded metadata should match persisted values"
        );

        let payload_rows: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM state_chunks", [], |row| row.get(0))
            .with_context(|| format!("count placeholder rows for {}", scenario.label))?;
        assert_eq!(
            payload_rows as u64, checkpoint.rows_written,
            "row count should match the number of inserted chunks"
        );
        drop(store);

        let total_size = directory_size_bytes(checkpoint.temp_dir.path())
            .with_context(|| format!("compute directory size for {}", scenario.label))?;
        assert!(
            total_size >= scenario.target_bytes,
            "placeholder '{}' produced {} bytes on disk; expected at least {} bytes",
            scenario.label,
            total_size,
            scenario.target_bytes
        );
    }

    Ok(())
}

fn generate_placeholder_checkpoint(scenario: PlaceholderScenario) -> Result<GeneratedCheckpoint> {
    let temp_dir = tempfile::tempdir().context("create temporary checkpoint directory")?;
    let db_path = temp_dir
        .path()
        .join(format!("checkpoint_{}.sqlite", scenario.label));

    let mut store = CheckpointStore::open(&db_path, StoreOptions::default())
        .with_context(|| format!("open checkpoint store for {}", scenario.label))?;
    let (rows_written, bytes_written) = {
        let conn = store.connection_mut();
        populate_placeholder_payload(conn, scenario.target_bytes)
            .with_context(|| format!("populate placeholder payload for {}", scenario.label))?
    };

    let metadata =
        record_placeholder_metadata(&mut store).context("persist placeholder snapshot metadata")?;
    store
        .connection()
        .pragma_update(None, "wal_checkpoint", &"TRUNCATE")
        .context("flush WAL into main checkpoint database")?;
    drop(store);

    Ok(GeneratedCheckpoint {
        temp_dir,
        db_path,
        metadata,
        rows_written,
        bytes_written,
    })
}

fn record_placeholder_metadata(store: &mut CheckpointStore) -> Result<SnapshotMetadata> {
    let spec_hash =
        Blake3Digest::from_hex(PLACEHOLDER_SPEC_HASH).context("derive placeholder spec hash")?;
    let checkpoint_id = Ulid::new();
    let run_id = Ulid::new();
    let metadata = SnapshotMetadata::fresh(store.connection(), checkpoint_id, run_id, spec_hash)
        .context("capture snapshot metadata")?;
    store
        .write_snapshot_metadata(&metadata)
        .context("write snapshot metadata")?;
    Ok(metadata)
}

fn populate_placeholder_payload(
    conn: &mut rusqlite::Connection,
    target_bytes: u64,
) -> Result<(u64, u64)> {
    if target_bytes == 0 {
        return Ok((0, 0));
    }

    conn.execute(
        "CREATE TABLE IF NOT EXISTS state_chunks (
            chunk_index INTEGER PRIMARY KEY AUTOINCREMENT,
            payload BLOB NOT NULL
        )",
        [],
    )
    .context("create placeholder payload table")?;

    let mut remaining = target_bytes;
    let mut rows_written = 0_u64;
    let mut bytes_written = 0_u64;
    let chunk_size: u64 = 8 * 1024 * 1024;
    let tx = conn
        .transaction()
        .context("begin placeholder payload transaction")?;

    while remaining > 0 {
        let chunk = chunk_size.min(remaining);
        let chunk_len = i64::try_from(chunk).context("convert chunk length to i64")?;
        tx.execute(
            "INSERT INTO state_chunks (payload) VALUES (zeroblob(?1))",
            params![chunk_len],
        )
        .with_context(|| format!("insert zeroblob chunk {rows_written}"))?;
        remaining -= chunk;
        bytes_written += chunk;
        rows_written += 1;
    }

    tx.commit()
        .context("commit placeholder payload transaction")?;

    Ok((rows_written, bytes_written))
}

fn directory_size_bytes(path: &Path) -> Result<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path).context("read checkpoint directory entries")? {
        let entry = entry.context("enumerate checkpoint directory entry")?;
        let metadata = entry
            .metadata()
            .context("retrieve metadata for checkpoint artifact")?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        }
    }
    Ok(total)
}
