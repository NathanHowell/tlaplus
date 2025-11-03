use std::num::NonZeroUsize;

use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use tlc_engine::{partition_frontier, FrontierSlice};

fn strategy_inputs() -> impl Strategy<Value = (usize, NonZeroUsize)> {
    (0usize..=100_000usize, 1usize..=64usize).prop_map(|(frontier_len, workers)| {
        let workers = NonZeroUsize::new(workers).expect("worker strategy never yields zero");
        (frontier_len, workers)
    })
}

fn assert_contiguous_cover(
    frontier_len: usize,
    slices: &[FrontierSlice],
) -> Result<(), TestCaseError> {
    let mut cursor = 0usize;
    for slice in slices {
        prop_assert_eq!(slice.start, cursor, "slices must be contiguous");
        prop_assert!(slice.end >= slice.start, "slice end must not precede start");
        cursor = slice.end;
    }
    prop_assert_eq!(cursor, frontier_len, "slices must cover entire frontier");
    Ok(())
}

proptest! {
    /// Generated slices must cover the entire frontier without gaps or overlaps.
    #[test]
    fn slices_cover_frontier((frontier_len, workers) in strategy_inputs()) {
        let slices = partition_frontier(frontier_len, workers);
        prop_assert_eq!(slices.len(), workers.get(), "one slice per worker expected");
        prop_assert_eq!(slices.first().map(|s| s.start).unwrap_or(0), 0, "frontier must start at zero");
        assert_contiguous_cover(frontier_len, &slices)?;
    }
}

proptest! {
    /// Slice lengths may differ by at most one element to keep work balanced.
    #[test]
    fn slices_balanced((frontier_len, workers) in strategy_inputs()) {
        let slices = partition_frontier(frontier_len, workers);
        prop_assert_eq!(slices.len(), workers.get());

        let lengths: Vec<usize> = slices.iter().map(FrontierSlice::len).collect();
        prop_assert_eq!(lengths.iter().sum::<usize>(), frontier_len);

        if let (Some(min), Some(max)) = (lengths.iter().min(), lengths.iter().max()) {
            prop_assert!(max - min <= 1, "slice size imbalance should never exceed one element");
        }
    }
}
