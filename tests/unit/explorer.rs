use std::{
    error::Error,
    num::NonZeroU16,
    time::{Duration, Instant},
};

use tlc_engine::{
    EngineSizing, ExplorationError, Explorer, ModelSemantics, RunMetricsRecorder, SemanticInputs,
    State, TraversalStrategy, Value, WorkerScheduler,
};
use tlc_util::parser::{parse_module, ParserOptions};
use ulid::Ulid;

#[test]
fn breadth_first_exploration_detects_violation_after_wide_layer() -> Result<(), Box<dyn Error>> {
    let semantics = build_branching_semantics()?;
    let workers = NonZeroU16::new(2).unwrap();
    let sizing = EngineSizing::new(workers, None);
    let scheduler = WorkerScheduler::with_name_prefix(sizing.worker_threads(), "bfs-explorer")?;

    let mut explorer = Explorer::new(
        semantics,
        scheduler,
        sizing.queue_capacity(),
        TraversalStrategy::BreadthFirst,
    )?;

    let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJM")?;
    let recorder =
        RunMetricsRecorder::with_start_instant(run_id, Instant::now() - Duration::from_millis(10));

    match explorer.explore(recorder) {
        Err(ExplorationError::InvariantViolated { violation, summary }) => {
            assert_eq!(violation.name, "Inv");
            assert_eq!(violation.depth, 2);
            assert_eq!(extract_int(&violation.state, "x"), 4);

            assert_eq!(summary.metrics.run_id(), run_id);
            assert_eq!(summary.metrics.states_explored(), 5);
            assert!(summary.metrics.states_per_second() >= 0.0);
            assert_eq!(summary.distinct_states, 5);
            assert_eq!(summary.max_depth, 2);
            assert_eq!(summary.queue_stats.len(), 0);
        }
        other => panic!("expected invariant violation, received {:?}", other),
    }

    Ok(())
}

#[test]
fn depth_first_exploration_hits_deep_violation_first() -> Result<(), Box<dyn Error>> {
    let semantics = build_branching_semantics()?;
    let workers = NonZeroU16::new(2).unwrap();
    let sizing = EngineSizing::new(workers, None);
    let scheduler = WorkerScheduler::with_name_prefix(sizing.worker_threads(), "dfs-explorer")?;

    let mut explorer = Explorer::new(
        semantics,
        scheduler,
        sizing.queue_capacity(),
        TraversalStrategy::DepthFirst,
    )?;

    let run_id = Ulid::from_string("01J0Y6M4F2A5B7C8D9E0F1GHJN")?;
    let recorder =
        RunMetricsRecorder::with_start_instant(run_id, Instant::now() - Duration::from_millis(10));

    match explorer.explore(recorder) {
        Err(ExplorationError::InvariantViolated { violation, summary }) => {
            assert_eq!(violation.name, "Inv");
            assert_eq!(violation.depth, 2);
            assert_eq!(extract_int(&violation.state, "x"), 4);

            assert_eq!(summary.metrics.run_id(), run_id);
            assert_eq!(summary.metrics.states_explored(), 3);
            assert_eq!(summary.distinct_states, 4);
            assert_eq!(summary.max_depth, 2);

            // DFS stops once the deep violation is encountered, leaving the sibling branch queued.
            assert_eq!(summary.queue_stats.len(), 1);
        }
        other => panic!("expected invariant violation, received {:?}", other),
    }

    Ok(())
}

fn build_branching_semantics() -> Result<ModelSemantics, Box<dyn Error>> {
    let module_source = r#"
---- MODULE Branching ----
VARIABLE x

Init ==
    x = 0

Next ==
    \/ /\ x = 0 /\ x' = 1
    \/ /\ x = 0 /\ x' = 2
    \/ /\ x = 1 /\ x' = 3
    \/ /\ x = 2 /\ x' = 4
    \/ /\ x = 3 /\ x' = 3
    \/ /\ x = 4 /\ x' = 4

Inv == ~(x = 4)

====
"#;

    let module = parse_module(module_source, ParserOptions::default())?;
    let inputs = SemanticInputs::new(&[module])
        .with_init_operators(&["Init"])
        .with_next_operator("Next")
        .with_invariant_operators(&["Inv"]);
    Ok(ModelSemantics::new(inputs)?)
}

fn extract_int(state: &State, name: &str) -> i64 {
    match state.get(name) {
        Some(Value::Int(value)) => *value,
        other => panic!("expected integer value for {name}, got {:?}", other),
    }
}
