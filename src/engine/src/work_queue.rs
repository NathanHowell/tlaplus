//! Multi-producer, multi-consumer work queue with optional throttling.
//!
//! The queue is backed by `crossbeam::channel` so it supports both bounded (throttled) and
//! unbounded configurations. Bounded queues block producers once the configured capacity is
//! exhausted, giving the engine a simple mechanism to avoid unbounded memory growth when the
//! frontier outpaces consumers. Consumers can drain items individually or in bounded batches,
//! enabling cooperative scheduling with the `WorkerScheduler`.

use std::{fmt, num::NonZeroUsize, time::Duration};

use crossbeam::channel::{
    self, Receiver, RecvError, RecvTimeoutError, SendError, Sender, TryRecvError, TrySendError,
};

/// Capacity descriptor for the work queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueCapacity {
    /// Queue enforces a hard upper bound and throttles producers once reached.
    Bounded(NonZeroUsize),
    /// Queue grows without bound; throttling is disabled.
    Unbounded,
}

impl QueueCapacity {
    /// Construct a bounded capacity descriptor.
    pub fn bounded(capacity: NonZeroUsize) -> Self {
        Self::Bounded(capacity)
    }

    /// Construct an unbounded capacity descriptor.
    pub fn unbounded() -> Self {
        Self::Unbounded
    }

    /// Returns `true` when the queue is bounded.
    pub fn is_bounded(&self) -> bool {
        matches!(self, Self::Bounded(_))
    }

    /// Maximum number of items the queue can hold, if bounded.
    pub fn max_len(&self) -> Option<NonZeroUsize> {
        match self {
            Self::Bounded(capacity) => Some(*capacity),
            Self::Unbounded => None,
        }
    }
}

/// Snapshot describing queue utilization for telemetry and throttling decisions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QueueStats {
    len: usize,
    capacity: QueueCapacity,
    saturation: Option<f64>,
}

impl QueueStats {
    /// Number of items currently buffered in the queue.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Capacity descriptor for the queue.
    pub fn capacity(&self) -> QueueCapacity {
        self.capacity
    }

    /// Saturation ratio in `[0.0, 1.0]` when bounded, or `None` when unbounded.
    pub fn saturation(&self) -> Option<f64> {
        self.saturation
    }

    /// Remaining slots before producers throttle, if bounded.
    pub fn remaining_capacity(&self) -> Option<usize> {
        self.capacity
            .max_len()
            .map(|cap| cap.get().saturating_sub(self.len))
    }

    /// Returns `true` when the queue is at capacity.
    pub fn is_full(&self) -> bool {
        self.remaining_capacity()
            .map_or(false, |remaining| remaining == 0)
    }
}

/// Crossbeam-backed work queue supporting bounded (throttled) and unbounded modes.
pub struct WorkQueue<T> {
    sender: Sender<T>,
    receiver: Receiver<T>,
    capacity: QueueCapacity,
}

impl<T> WorkQueue<T> {
    /// Create a bounded queue enforcing the provided capacity.
    pub fn bounded(capacity: NonZeroUsize) -> Self {
        let (sender, receiver) = channel::bounded(capacity.get());
        Self {
            sender,
            receiver,
            capacity: QueueCapacity::Bounded(capacity),
        }
    }

    /// Create an unbounded queue with no throttling.
    pub fn unbounded() -> Self {
        let (sender, receiver) = channel::unbounded();
        Self {
            sender,
            receiver,
            capacity: QueueCapacity::Unbounded,
        }
    }

    /// Return a clone of the underlying sender for distributing to producers.
    pub fn sender(&self) -> Sender<T> {
        self.sender.clone()
    }

    /// Return a clone of the underlying receiver for distributing to consumers.
    pub fn receiver(&self) -> Receiver<T> {
        self.receiver.clone()
    }

    /// Block until an item is enqueued, returning an error when the queue is closed.
    pub fn dequeue(&self) -> Result<T, RecvError> {
        self.receiver.recv()
    }

    /// Attempt to dequeue an item without blocking.
    pub fn try_dequeue(&self) -> Result<T, TryRecvError> {
        self.receiver.try_recv()
    }

    /// Attempt to dequeue with a timeout, returning an error if the queue is closed or no item
    /// arrives within the duration.
    pub fn dequeue_timeout(&self, timeout: Duration) -> Result<T, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }

    /// Enqueue an item, blocking when the queue is full (for bounded queues).
    pub fn enqueue(&self, item: T) -> Result<(), SendError<T>> {
        self.sender.send(item)
    }

    /// Attempt to enqueue without blocking, returning immediately if the queue is full.
    pub fn try_enqueue(&self, item: T) -> Result<(), TrySendError<T>> {
        self.sender.try_send(item)
    }

    /// Drain at most `max` items from the queue without blocking, preserving FIFO ordering.
    pub fn drain_batch(&self, max: usize) -> Vec<T> {
        if max == 0 {
            return Vec::new();
        }

        let mut drained = Vec::with_capacity(max);
        for _ in 0..max {
            match self.receiver.try_recv() {
                Ok(item) => drained.push(item),
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }

        drained
    }

    /// Returns the approximate number of items buffered in the queue.
    pub fn len(&self) -> usize {
        self.receiver.len()
    }

    /// Returns `true` when no items are buffered.
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }

    /// Capacity descriptor.
    pub fn capacity(&self) -> QueueCapacity {
        self.capacity
    }

    /// Ratio of buffered items to capacity. Returns `None` for unbounded queues.
    pub fn saturation(&self) -> Option<f64> {
        match self.capacity {
            QueueCapacity::Bounded(capacity) => {
                let cap = capacity.get() as f64;
                if cap == 0.0 {
                    return Some(0.0);
                }
                let len = self.len() as f64;
                Some((len / cap).min(1.0))
            }
            QueueCapacity::Unbounded => None,
        }
    }

    /// Remaining capacity before blocking producers, if bounded.
    pub fn remaining_capacity(&self) -> Option<usize> {
        match self.capacity {
            QueueCapacity::Bounded(capacity) => Some(capacity.get().saturating_sub(self.len())),
            QueueCapacity::Unbounded => None,
        }
    }

    /// Returns queue statistics for telemetry.
    pub fn stats(&self) -> QueueStats {
        QueueStats {
            len: self.len(),
            capacity: self.capacity,
            saturation: self.saturation(),
        }
    }
}

impl<T> Clone for WorkQueue<T> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            receiver: self.receiver.clone(),
            capacity: self.capacity,
        }
    }
}

impl<T> fmt::Debug for WorkQueue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkQueue")
            .field("capacity", &self.capacity)
            .field("len", &self.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration as StdDuration;

    #[test]
    fn bounded_queue_reports_saturation_and_throttles() {
        let capacity = NonZeroUsize::new(2).unwrap();
        let queue = WorkQueue::bounded(capacity);

        queue.try_enqueue(1usize).expect("enqueue first");
        queue.try_enqueue(2usize).expect("enqueue second");

        assert!(matches!(
            queue.try_enqueue(3usize),
            Err(TrySendError::Full(3))
        ));

        let stats = queue.stats();
        assert_eq!(stats.len(), 2);
        assert_eq!(stats.remaining_capacity(), Some(0));
        assert!(stats.is_full());
        assert_eq!(stats.saturation(), Some(1.0));

        queue.try_dequeue().expect("dequeue one");
        assert_eq!(queue.remaining_capacity(), Some(1));
    }

    #[test]
    fn drain_batch_preserves_order_and_respects_limit() {
        let capacity = NonZeroUsize::new(8).unwrap();
        let queue = WorkQueue::bounded(capacity);

        for value in 0..5 {
            queue.try_enqueue(value).expect("enqueue value");
        }

        let drained = queue.drain_batch(3);
        assert_eq!(drained, vec![0, 1, 2]);
        assert_eq!(queue.len(), 2);

        let remaining = queue.drain_batch(10);
        assert_eq!(remaining, vec![3, 4]);
        assert!(queue.is_empty());
    }

    #[test]
    fn close_notifies_consumers() {
        let capacity = NonZeroUsize::new(1).unwrap();
        let queue: WorkQueue<()> = WorkQueue::bounded(capacity);
        let receiver = queue.receiver();

        let handle = thread::spawn(move || receiver.recv());

        thread::sleep(StdDuration::from_millis(25));
        drop(queue);

        let result = handle.join().expect("join receiver thread");
        assert!(matches!(result, Err(RecvError)));
    }

    #[test]
    fn unbounded_queue_reports_none_for_saturation() {
        let queue = WorkQueue::<usize>::unbounded();
        assert_eq!(queue.capacity(), QueueCapacity::Unbounded);
        assert_eq!(queue.saturation(), None);
        assert_eq!(queue.remaining_capacity(), None);
    }
}
