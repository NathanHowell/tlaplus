use std::path::{Path, PathBuf};

use anyhow::Error as AnyhowError;
use thiserror::Error;
use tlc_checkpoint::{
    snapshot::{SnapshotError, SnapshotMetadata},
    CheckpointStore, StoreOptions,
};
use tlc_util::{RunConfiguration, SpecificationPackage};
use ulid::Ulid;

use super::{prepare_run, EngineError, EngineOptions, RunContext};

/// Result type returned by resume helpers.
pub type Result<T> = std::result::Result<T, ResumeError>;

/// Parameters required to prepare a resume context.
#[derive(Debug, Clone)]
pub struct ResumeRequest {
    checkpoint_path: PathBuf,
    store_options: StoreOptions,
    ignore_hash_mismatch: bool,
    prior_lineage: Vec<Ulid>,
}

impl ResumeRequest {
    /// Create a request targeting a specific checkpoint file.
    pub fn new<P: Into<PathBuf>>(checkpoint_path: P) -> Self {
        Self {
            checkpoint_path: checkpoint_path.into(),
            store_options: StoreOptions::default(),
            ignore_hash_mismatch: false,
            prior_lineage: Vec::new(),
        }
    }

    /// Toggle spec hash enforcement (mirrors `--ignore-hash`).
    pub fn with_ignore_hash(mut self, ignore: bool) -> Self {
        self.ignore_hash_mismatch = ignore;
        self
    }

    /// Override the store options used when opening the checkpoint.
    pub fn with_store_options(mut self, options: StoreOptions) -> Self {
        self.store_options = options;
        self
    }

    /// Seed the lineage chain with prior run identifiers.
    pub fn with_prior_lineage<I>(mut self, lineage: I) -> Self
    where
        I: IntoIterator<Item = Ulid>,
    {
        self.prior_lineage = lineage.into_iter().collect();
        self
    }

    /// Location of the checkpoint manifest on disk.
    pub fn checkpoint_path(&self) -> &Path {
        self.checkpoint_path.as_path()
    }

    /// Whether hash mismatches should be ignored.
    pub fn ignore_hash_mismatch(&self) -> bool {
        self.ignore_hash_mismatch
    }

    /// Access the configured store options.
    pub fn store_options(&self) -> &StoreOptions {
        &self.store_options
    }

    /// Retrieve the seeded lineage chain.
    pub fn prior_lineage(&self) -> &[Ulid] {
        &self.prior_lineage
    }
}

/// Ordered chain of run identifiers representing a resume lineage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeLineage {
    chain: Vec<Ulid>,
}

impl ResumeLineage {
    fn new(mut chain: Vec<Ulid>) -> Self {
        chain.dedup();
        Self { chain }
    }

    /// Ordered run identifiers from origin → most recent.
    pub fn chain(&self) -> &[Ulid] {
        &self.chain
    }

    /// Identifier for the originating run in the lineage.
    pub fn origin(&self) -> Option<Ulid> {
        self.chain.first().copied()
    }

    /// Identifier for the most recent run in the lineage.
    pub fn latest(&self) -> Option<Ulid> {
        self.chain.last().copied()
    }
}

/// Prepared resume context containing validated run state and checkpoint metadata.
#[derive(Debug)]
pub struct ResumeContext {
    run: RunContext,
    checkpoint: CheckpointStore,
    metadata: SnapshotMetadata,
    checkpoint_path: PathBuf,
    lineage: ResumeLineage,
}

impl ResumeContext {
    /// Access the prepared run context.
    pub fn run(&self) -> &RunContext {
        &self.run
    }

    /// Borrow the checkpoint store.
    pub fn checkpoint_store(&self) -> &CheckpointStore {
        &self.checkpoint
    }

    /// Borrow the checkpoint store mutably.
    pub fn checkpoint_store_mut(&mut self) -> &mut CheckpointStore {
        &mut self.checkpoint
    }

    /// Return the checkpoint metadata manifest.
    pub fn metadata(&self) -> &SnapshotMetadata {
        &self.metadata
    }

    /// Filesystem location of the checkpoint manifest.
    pub fn checkpoint_path(&self) -> &Path {
        self.checkpoint_path.as_path()
    }

    /// Access the resume lineage chain.
    pub fn lineage(&self) -> &ResumeLineage {
        &self.lineage
    }

    /// Consume the context and return its components.
    pub fn into_parts(
        self,
    ) -> (
        RunContext,
        CheckpointStore,
        SnapshotMetadata,
        ResumeLineage,
        PathBuf,
    ) {
        (
            self.run,
            self.checkpoint,
            self.metadata,
            self.lineage,
            self.checkpoint_path,
        )
    }
}

/// Errors surfaced while preparing resume contexts.
#[derive(Debug, Error)]
pub enum ResumeError {
    #[error("resume configuration already references checkpoint {0}")]
    AlreadyConfigured(Ulid),
    #[error("failed to open checkpoint store at {path}: {source}")]
    OpenStore {
        path: PathBuf,
        #[source]
        source: AnyhowError,
    },
    #[error("checkpoint manifest at {path} has no snapshot metadata")]
    MissingSnapshot { path: PathBuf },
    #[error("failed to read checkpoint metadata at {path}: {source}")]
    Metadata {
        path: PathBuf,
        #[source]
        source: SnapshotError,
    },
    #[error("spec hash mismatch for checkpoint at {path}: expected {expected}, found {found}")]
    SpecHashMismatch {
        path: PathBuf,
        expected: String,
        found: String,
    },
    #[error(transparent)]
    Engine(#[from] EngineError),
}

/// Prepare a resume context by validating the checkpoint and seeding lineage tracking.
pub fn prepare_resume(
    specification: SpecificationPackage,
    mut configuration: RunConfiguration,
    options: EngineOptions,
    request: ResumeRequest,
) -> Result<ResumeContext> {
    if let Some(existing) = configuration.resume_from {
        return Err(ResumeError::AlreadyConfigured(existing));
    }

    let ResumeRequest {
        checkpoint_path,
        store_options,
        ignore_hash_mismatch,
        prior_lineage,
    } = request;

    let store = CheckpointStore::open(&checkpoint_path, store_options).map_err(|source| {
        ResumeError::OpenStore {
            path: checkpoint_path.clone(),
            source,
        }
    })?;
    let has_metadata =
        SnapshotMetadata::exists(store.connection()).map_err(|source| ResumeError::Metadata {
            path: checkpoint_path.clone(),
            source,
        })?;
    if !has_metadata {
        return Err(ResumeError::MissingSnapshot {
            path: checkpoint_path,
        });
    }

    let metadata =
        SnapshotMetadata::load(store.connection()).map_err(|source| ResumeError::Metadata {
            path: checkpoint_path.clone(),
            source,
        })?;
    let expected_hash_hex = specification.hash.to_hex();
    let actual_hash_hex = metadata.spec_hash.to_hex();
    if metadata.spec_hash != specification.hash {
        if ignore_hash_mismatch {
            tracing::warn!(
                checkpoint = %checkpoint_path.display(),
                checkpoint_id = %metadata.checkpoint_id,
                expected = %expected_hash_hex,
                actual = %actual_hash_hex,
                "spec hash mismatch ignored for resume"
            );
        } else {
            return Err(ResumeError::SpecHashMismatch {
                path: checkpoint_path,
                expected: expected_hash_hex,
                found: actual_hash_hex,
            });
        }
    } else {
        tracing::debug!(
            checkpoint = %checkpoint_path.display(),
            checkpoint_id = %metadata.checkpoint_id,
            origin_run = %metadata.run_id,
            "validated checkpoint metadata for resume"
        );
    }

    configuration.resume_from = Some(metadata.checkpoint_id);
    let run_context = prepare_run(specification, configuration, options)?;

    let mut chain = prior_lineage;
    if chain.last().copied() != Some(metadata.run_id) {
        chain.push(metadata.run_id);
    }
    chain.push(run_context.configuration().run_id);
    let lineage = ResumeLineage::new(chain);

    tracing::info!(
        checkpoint = %checkpoint_path.display(),
        checkpoint_id = %metadata.checkpoint_id,
        origin_run = %metadata.run_id,
        resume_run = %run_context.configuration().run_id,
        "prepared TLC resume context"
    );

    Ok(ResumeContext {
        run: run_context,
        checkpoint: store,
        metadata,
        checkpoint_path,
        lineage,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blake3::hash;
    use chrono::Utc;
    use std::{
        collections::BTreeMap,
        fs::{self, File},
        io::Write,
        num::NonZeroU16,
        path::Path,
    };
    use tempfile::tempdir;
    use tlc_util::{Blake3Digest, ProgressMode, TelemetryMode};
    use ulid::Ulid;
    use uuid::Uuid;

    fn write_file(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent directory");
        }
        let mut file = File::create(path).expect("create file");
        file.write_all(contents.as_bytes()).expect("write contents");
    }

    fn build_specification(temp_dir: &Path) -> SpecificationPackage {
        let spec_path = temp_dir.join("Main.tla");
        let cfg_path = temp_dir.join("MC.cfg");
        write_file(&spec_path, "---- MODULE Main ----\n====");
        write_file(&cfg_path, "CONSTANTS Foo = 1");

        SpecificationPackage::new(
            Uuid::new_v4(),
            vec![spec_path],
            cfg_path,
            BTreeMap::new(),
            Blake3Digest::from_hash(hash(b"resume-demo")),
        )
        .expect("create specification package")
    }

    fn build_configuration(telemetry: TelemetryMode) -> RunConfiguration {
        RunConfiguration::new(
            Ulid::new(),
            NonZeroU16::new(4).expect("non-zero workers"),
            Some(2 * 1024 * 1024 * 1024),
            telemetry,
            ProgressMode::Tty,
            None,
        )
        .expect("create run configuration")
    }

    fn base_options(temp_dir: &Path) -> EngineOptions {
        EngineOptions {
            checkpoint_dir: Some(temp_dir.join("checkpoints")),
            checkpoint_interval: Some(std::time::Duration::ZERO),
            coverage_interval: Some(std::time::Duration::from_secs(300)),
            continue_on_violation: true,
            skip_deadlock: false,
            cleanup: false,
            suppress_warnings: false,
            diff_trace: false,
            tty_use_color: true,
            dump_trace: None,
            post_conditions: vec!["Foo!Bar".into()],
            telemetry_endpoint: None,
        }
    }

    fn install_metadata(path: &Path, spec_hash: &Blake3Digest) -> (Ulid, Ulid) {
        let mut store =
            CheckpointStore::open(path, StoreOptions::default()).expect("open checkpoint");
        let checkpoint_id = Ulid::new();
        let source_run = Ulid::new();
        let metadata = SnapshotMetadata::new(
            checkpoint_id,
            source_run,
            spec_hash.clone(),
            Utc::now(),
            4096,
        )
        .expect("metadata new");
        store
            .write_snapshot_metadata(&metadata)
            .expect("persist metadata");
        (checkpoint_id, source_run)
    }

    #[test]
    fn prepare_resume_validates_metadata_and_builds_lineage() {
        let temp_dir = tempdir().expect("tempdir");
        let spec = build_specification(temp_dir.path());
        let config = build_configuration(TelemetryMode::Local);
        let options = base_options(temp_dir.path());
        let checkpoint_path = temp_dir.path().join("resume.chk");
        let (checkpoint_id, parent_run) = install_metadata(&checkpoint_path, &spec.hash);

        let request = ResumeRequest::new(&checkpoint_path);
        let context =
            prepare_resume(spec.clone(), config.clone(), options, request).expect("resume context");

        assert_eq!(
            context.run().configuration().resume_from,
            Some(checkpoint_id)
        );
        assert_eq!(context.metadata().checkpoint_id, checkpoint_id);
        assert_eq!(context.metadata().run_id, parent_run);
        assert_eq!(
            context.lineage().chain(),
            &[parent_run, context.run().configuration().run_id]
        );
    }

    #[test]
    fn hash_mismatch_rejected_without_ignore_flag() {
        let temp_dir = tempdir().expect("tempdir");
        let spec = build_specification(temp_dir.path());
        let config = build_configuration(TelemetryMode::Local);
        let options = base_options(temp_dir.path());
        let checkpoint_path = temp_dir.path().join("resume.chk");
        install_metadata(
            &checkpoint_path,
            &Blake3Digest::from_hash(hash(b"other-hash")),
        );

        let err = prepare_resume(spec, config, options, ResumeRequest::new(&checkpoint_path))
            .expect_err("mismatch expected");
        match err {
            ResumeError::SpecHashMismatch { path, .. } => assert_eq!(path, checkpoint_path),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn hash_mismatch_allowed_with_ignore_flag() {
        let temp_dir = tempdir().expect("tempdir");
        let spec = build_specification(temp_dir.path());
        let config = build_configuration(TelemetryMode::Local);
        let options = base_options(temp_dir.path());
        let checkpoint_path = temp_dir.path().join("resume.chk");
        install_metadata(
            &checkpoint_path,
            &Blake3Digest::from_hash(hash(b"other-hash")),
        );

        let request = ResumeRequest::new(&checkpoint_path).with_ignore_hash(true);
        let context =
            prepare_resume(spec, config, options, request).expect("resume context with ignore");
        assert_eq!(
            context.metadata().checkpoint_id,
            context.run().configuration().resume_from.unwrap()
        );
    }

    #[test]
    fn errors_when_resume_already_configured() {
        let temp_dir = tempdir().expect("tempdir");
        let spec = build_specification(temp_dir.path());
        let mut config = build_configuration(TelemetryMode::Local);
        config.resume_from = Some(Ulid::new());
        let options = base_options(temp_dir.path());
        let checkpoint_path = temp_dir.path().join("resume.chk");
        install_metadata(&checkpoint_path, &spec.hash);

        let err = prepare_resume(spec, config, options, ResumeRequest::new(&checkpoint_path))
            .expect_err("existing resume guard");
        assert!(matches!(err, ResumeError::AlreadyConfigured(_)));
    }

    #[test]
    fn errors_when_metadata_missing() {
        let temp_dir = tempdir().expect("tempdir");
        let spec = build_specification(temp_dir.path());
        let config = build_configuration(TelemetryMode::Local);
        let options = base_options(temp_dir.path());
        let checkpoint_path = temp_dir.path().join("empty.chk");
        CheckpointStore::open(&checkpoint_path, StoreOptions::default()).expect("open store");

        let err = prepare_resume(spec, config, options, ResumeRequest::new(&checkpoint_path))
            .expect_err("missing metadata");
        assert!(matches!(err, ResumeError::MissingSnapshot { .. }));
    }
}
