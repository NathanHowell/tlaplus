use anyhow::Result;
use rusqlite::params;
use tlc_checkpoint::{version, CheckpointStore, StoreOptions};

#[test]
fn resume_rejects_stale_manifest_version() -> Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let db_path = temp_dir.path().join("stale_manifest.sqlite");

    {
        let store = CheckpointStore::open(&db_path, StoreOptions::default())?;
        let conn = store.into_inner();
        conn.execute(
            "UPDATE checkpoint_metadata SET value = ?1 WHERE key = ?2",
            params!["0.9", version::MANIFEST_VERSION_KEY],
        )?;
        conn.close().expect("close checkpoint connection");
    }

    let err = CheckpointStore::open(&db_path, StoreOptions::default())
        .expect_err("stale manifest must prevent resume");

    let version_error = err
        .downcast_ref::<version::VersionError>()
        .expect("expected manifest version error");

    match version_error {
        version::VersionError::Incompatible { expected, found } => {
            assert_eq!(
                *expected,
                version::CURRENT_MANIFEST_VERSION,
                "expected version should match current binary support"
            );
            assert_eq!(
                *found,
                version::ManifestVersion::new(0, 9),
                "stale manifest version should be detected"
            );
        }
        other => panic!("unexpected version error: {other}"),
    }

    Ok(())
}
