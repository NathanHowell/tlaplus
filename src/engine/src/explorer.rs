use std::{cmp::Ordering, collections::HashSet, convert::TryFrom, sync::Arc};

use rayon::prelude::*;
use tlc_util::{FingerprintBuilder, StateFingerprint};

use crate::{
    ModelSemantics, QueueCapacity, QueueStats, RunMetrics, RunMetricsRecorder, SemanticError,
    State, Value, WorkQueue, WorkerScheduler,
};

/// Exploration strategy applied to pending frontier states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalStrategy {
    BreadthFirst,
    DepthFirst,
}

/// Core exploration loop coordinating the scheduler, work queue, and semantic evaluator.
pub struct Explorer {
    semantics: Arc<ModelSemantics>,
    scheduler: WorkerScheduler,
    queue: WorkQueue<PendingState>,
    strategy: TraversalStrategy,
    visited: HashSet<u128>,
}

impl Explorer {
    /// Construct a new explorer seeded with the initial states emitted by the provided semantics.
    pub fn new(
        semantics: ModelSemantics,
        scheduler: WorkerScheduler,
        capacity: QueueCapacity,
        strategy: TraversalStrategy,
    ) -> Result<Self, ExplorationError> {
        let queue = match capacity {
            QueueCapacity::Bounded(capacity) => WorkQueue::bounded(capacity),
            QueueCapacity::Unbounded => WorkQueue::unbounded(),
        };

        let semantics = Arc::new(semantics);
        let mut explorer = Self {
            semantics,
            scheduler,
            queue,
            strategy,
            visited: HashSet::new(),
        };

        explorer.seed_initial_states()?;
        if explorer.visited.is_empty() {
            return Err(ExplorationError::NoInitialStates);
        }

        Ok(explorer)
    }

    fn seed_initial_states(&mut self) -> Result<(), ExplorationError> {
        for state in self.semantics.initial_states() {
            let fingerprint = fingerprint_state(state, 0);
            if self.visited.insert(fingerprint.value()) {
                let entry = PendingState::new(state.clone(), fingerprint, 0);
                self.queue
                    .enqueue(entry)
                    .map_err(|_| ExplorationError::QueueClosed)?;
            }
        }
        Ok(())
    }

    /// Execute the exploration loop until the frontier is exhausted or an invariant violation occurs.
    pub fn explore(
        &mut self,
        recorder: RunMetricsRecorder,
    ) -> Result<ExplorationSummary, ExplorationError> {
        let mut states_processed: u128 = 0;
        let mut max_depth: u64 = 0;

        loop {
            let batch = self.next_batch()?;
            if batch.is_empty() {
                break;
            }

            let outcomes = self.process_batch(batch)?;
            for outcome in outcomes {
                states_processed = states_processed.saturating_add(1);
                max_depth = max_depth.max(outcome.entry.depth);

                if let Some(violation) = outcome.violation {
                    let metrics = recorder.finish(states_processed);
                    let summary = self.build_summary(metrics, max_depth);
                    return Err(ExplorationError::InvariantViolated { violation, summary });
                }

                let successor_depth = outcome.entry.depth.saturating_add(1);
                for successor in outcome.successors {
                    self.enqueue_state(successor, successor_depth)?;
                }
            }
        }

        let metrics = recorder.finish(states_processed);
        Ok(self.build_summary(metrics, max_depth))
    }

    fn enqueue_state(&mut self, state: State, depth: u64) -> Result<(), ExplorationError> {
        let fingerprint = fingerprint_state(&state, depth);
        if !self.visited.insert(fingerprint.value()) {
            return Ok(());
        }

        let entry = PendingState::new(state, fingerprint, depth);
        self.queue
            .enqueue(entry)
            .map_err(|_| ExplorationError::QueueClosed)
    }

    fn build_summary(&self, metrics: RunMetrics, max_depth: u64) -> ExplorationSummary {
        ExplorationSummary {
            metrics,
            distinct_states: self.visited.len() as u128,
            max_depth,
            queue_stats: self.queue.stats(),
        }
    }

    fn next_batch(&self) -> Result<Vec<PendingState>, ExplorationError> {
        match self.strategy {
            TraversalStrategy::BreadthFirst => {
                let pending = self.queue.len();
                if pending == 0 {
                    return Ok(Vec::new());
                }
                let batch_size = self.batch_size(pending);
                Ok(self.queue.drain_batch(batch_size))
            }
            TraversalStrategy::DepthFirst => {
                let mut drained = self.queue.drain_batch(usize::MAX);
                if drained.is_empty() {
                    return Ok(Vec::new());
                }
                let next = drained.pop().expect("drained vec guaranteed non-empty");
                for entry in drained {
                    self.queue
                        .enqueue(entry)
                        .map_err(|_| ExplorationError::QueueClosed)?;
                }
                Ok(vec![next])
            }
        }
    }

    fn batch_size(&self, pending: usize) -> usize {
        if pending == 0 {
            return 0;
        }

        let workers = self.scheduler.worker_count().get();
        let base = workers.max(1);
        let preferred = base.saturating_mul(4);
        pending.min(preferred.max(1))
    }

    fn process_batch(
        &self,
        batch: Vec<PendingState>,
    ) -> Result<Vec<StateOutcome>, ExplorationError> {
        if batch.is_empty() {
            return Ok(Vec::new());
        }

        let semantics = Arc::clone(&self.semantics);

        let results: Result<Vec<_>, SemanticError> = self.scheduler.install(|| {
            batch
                .into_par_iter()
                .map(|entry| {
                    let violation = detect_violation(&semantics, &entry)?;
                    if let Some(violation) = violation {
                        return Ok(StateOutcome {
                            entry,
                            successors: Vec::new(),
                            violation: Some(violation),
                        });
                    }

                    let successors = semantics.successors(&entry.state)?;
                    Ok(StateOutcome {
                        entry,
                        successors,
                        violation: None,
                    })
                })
                .collect()
        });

        results.map_err(ExplorationError::from)
    }
}

fn detect_violation(
    semantics: &ModelSemantics,
    entry: &PendingState,
) -> Result<Option<InvariantViolation>, SemanticError> {
    for invariant in semantics.invariants() {
        let holds = invariant.evaluate(&entry.state)?;
        if !holds {
            let violation = InvariantViolation {
                name: invariant.name().to_string(),
                state: entry.state.clone(),
                fingerprint: entry.fingerprint,
                depth: entry.depth,
            };
            return Ok(Some(violation));
        }
    }
    Ok(None)
}

#[derive(Clone)]
struct PendingState {
    state: State,
    fingerprint: StateFingerprint,
    depth: u64,
}

impl PendingState {
    fn new(state: State, fingerprint: StateFingerprint, depth: u64) -> Self {
        Self {
            state,
            fingerprint,
            depth,
        }
    }
}

struct StateOutcome {
    entry: PendingState,
    successors: Vec<State>,
    violation: Option<InvariantViolation>,
}

/// Summary describing the outcome of an exploration run.
#[derive(Debug, Clone)]
pub struct ExplorationSummary {
    pub metrics: RunMetrics,
    pub distinct_states: u128,
    pub max_depth: u64,
    pub queue_stats: QueueStats,
}

/// Invariant failure surfaced by the exploration loop.
#[derive(Debug, Clone)]
pub struct InvariantViolation {
    pub name: String,
    pub state: State,
    pub fingerprint: StateFingerprint,
    pub depth: u64,
}

/// Errors surfaced while exploring the state space.
#[derive(Debug)]
pub enum ExplorationError {
    NoInitialStates,
    QueueClosed,
    Semantics(SemanticError),
    InvariantViolated {
        violation: InvariantViolation,
        summary: ExplorationSummary,
    },
}

impl From<SemanticError> for ExplorationError {
    fn from(error: SemanticError) -> Self {
        Self::Semantics(error)
    }
}

impl std::fmt::Display for ExplorationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExplorationError::NoInitialStates => {
                write!(f, "no initial states produced by semantic evaluator")
            }
            ExplorationError::QueueClosed => write!(f, "work queue closed unexpectedly"),
            ExplorationError::Semantics(error) => write!(f, "semantic evaluation failed: {error}"),
            ExplorationError::InvariantViolated { violation, .. } => {
                write!(f, "invariant '{}' violated", violation.name)
            }
        }
    }
}

impl std::error::Error for ExplorationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExplorationError::Semantics(err) => Some(err),
            _ => None,
        }
    }
}

fn fingerprint_state(state: &State, depth: u64) -> StateFingerprint {
    let mut builder = FingerprintBuilder::with_context(b"tlc-state");
    let len = u64::try_from(state.len()).expect("state variable count fits in u64");
    builder = builder.update_u64(len);

    for (name, value) in state {
        let name_len = u32::try_from(name.len()).expect("variable name length fits in u32");
        builder = builder.update(&name_len.to_be_bytes());
        builder = builder.update(name.as_bytes());

        let encoded = canonical_value_bytes(value);
        let value_len = u32::try_from(encoded.len()).expect("value encoding length fits in u32");
        builder = builder.update(&value_len.to_be_bytes());
        builder = builder.update(&encoded);
    }

    builder.finalize(depth)
}

fn canonical_value_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Int(v) => {
            let mut bytes = vec![0x01];
            bytes.extend_from_slice(&v.to_be_bytes());
            bytes
        }
        Value::Bool(b) => vec![0x02, if *b { 1 } else { 0 }],
        Value::Set(items) => {
            let mut encoded_items: Vec<Vec<u8>> = items.iter().map(canonical_value_bytes).collect();
            encoded_items.sort_by(|a, b| compare_slices(a, b));

            let mut bytes = vec![0x03];
            let count = u32::try_from(encoded_items.len()).expect("set size fits in u32");
            bytes.extend_from_slice(&count.to_be_bytes());
            for item in encoded_items {
                let len = u32::try_from(item.len()).expect("set item length fits in u32");
                bytes.extend_from_slice(&len.to_be_bytes());
                bytes.extend_from_slice(&item);
            }
            bytes
        }
    }
}

fn compare_slices(lhs: &[u8], rhs: &[u8]) -> Ordering {
    lhs.len()
        .cmp(&rhs.len())
        .then_with(|| lhs.iter().cmp(rhs.iter()))
}
