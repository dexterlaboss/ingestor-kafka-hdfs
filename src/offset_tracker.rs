//! Thread-safe offset tracking with watermark-based commits.
//!
//! This module provides `OffsetTracker`, which ensures that Kafka offsets are only
//! committed when all prior offsets have been successfully processed. This guarantees
//! no data loss even with parallel message processing.
//!
//! # Design
//!
//! For each partition, we track:
//! - `in_flight`: offsets currently being processed
//! - `completed`: offsets that finished processing but can't be committed yet
//! - `last_committed`: the highest offset we've successfully committed
//!
//! The committable watermark is calculated as:
//! - If `in_flight` is empty: max of all completed offsets
//! - Otherwise: min(in_flight) - 1 (can only commit up to the first gap)

use {
    anyhow::Result,
    dashmap::DashMap,
    log::{error, info},
    rdkafka::{
        consumer::{Consumer, StreamConsumer},
        TopicPartitionList,
    },
    std::{collections::BTreeSet, sync::Arc},
};

/// Tracks the state of offsets for a single partition.
#[derive(Debug, Default)]
struct PartitionState {
    /// Offsets currently being processed (in-flight).
    in_flight: BTreeSet<i64>,
    /// Offsets that completed but couldn't be committed yet (waiting for earlier offsets).
    completed: BTreeSet<i64>,
    /// The last offset that was successfully committed.
    last_committed: i64,
}

impl PartitionState {
    /// Calculate the offset to commit to Kafka (i.e., the next
    /// offset to read on restart). Returns None if nothing can be committed.
    fn committable_offset(&self) -> Option<i64> {
        let max_safe = if self.in_flight.is_empty() {
            // No in-flight messages, we can commit up to the max completed
            self.completed.iter().max().copied()?
        } else {
            let min_in_flight = *self.in_flight.iter().next()?;

            // Find the highest completed offset that's below the first in-flight
            self.completed
                .iter()
                .filter(|&&offset| offset < min_in_flight)
                .max()
                .copied()?
        };

        // Return the Kafka commit offset (next message to read)
        Some(max_safe + 1)
    }
}

/// Thread-safe offset tracker for a single topic.
pub struct OffsetTracker {
    /// The topic being tracked.
    topic: String,
    /// Per-partition state.
    partitions: DashMap<i32, PartitionState>,
    /// The Kafka consumer used for committing offsets.
    consumer: Arc<StreamConsumer>,
}

impl OffsetTracker {
    pub fn new(consumer: Arc<StreamConsumer>, topic: String) -> Self {
        Self {
            topic,
            partitions: DashMap::new(),
            consumer,
        }
    }

    /// Register a partition for tracking.
    pub fn register_partition(&self, partition: i32) {
        self.partitions.entry(partition).or_default();
    }

    /// Track a new message as in-flight.
    pub fn track(&self, partition: i32, offset: i64) -> Result<()> {
        let mut state = self
            .partitions
            .get_mut(&partition)
            .ok_or_else(|| anyhow::anyhow!("Partition {} not registered", partition))?;

        state.in_flight.insert(offset);

        Ok(())
    }

    /// Mark an offset as complete and attempt to commit if watermark advances.
    pub fn complete(&self, partition: i32, offset: i64) -> Result<()> {
        // Phase 1: Update state and determine if commit is needed
        let commit_info = {
            let mut state = self
                .partitions
                .get_mut(&partition)
                .ok_or_else(|| anyhow::anyhow!("Partition {} not registered", partition))?;

            state.in_flight.remove(&offset);
            state.completed.insert(offset);

            if let Some(commit_offset) = state.committable_offset() {
                if commit_offset > state.last_committed {
                    Some(commit_offset)
                } else {
                    None
                }
            } else {
                None
            }
        };

        // Phase 2: Perform commit
        if let Some(commit_offset) = commit_info {
            let commit_result = self.commit_offset(partition, commit_offset);

            if commit_result.is_ok() {
                // Phase 3: Update state after successful commit
                if let Some(mut state) = self.partitions.get_mut(&partition) {
                    if commit_offset > state.last_committed {
                        // Retain only completed offsets >= commit_offset (not yet committed)
                        state.completed.retain(|&o| o >= commit_offset);
                        state.last_committed = commit_offset;

                        info!(
                            "Committed offset {} for partition {}",
                            commit_offset, partition
                        );
                    }
                }
            }

            return commit_result;
        }

        Ok(())
    }

    /// Commit a specific offset for a partition.
    fn commit_offset(&self, partition: i32, offset: i64) -> Result<()> {
        let mut tpl = TopicPartitionList::new();
        tpl.add_partition_offset(&self.topic, partition, rdkafka::Offset::Offset(offset))
            .map_err(|e| anyhow::anyhow!("Failed to add partition offset: {:?}", e))?;

        self.consumer
            .commit(&tpl, rdkafka::consumer::CommitMode::Async)
            .map_err(|e| {
                error!(
                    "Failed to commit offset {} for partition {}: {:?}",
                    offset, partition, e
                );
                anyhow::anyhow!("Commit error: {:?}", e)
            })?;

        Ok(())
    }

    pub fn in_flight_count(&self) -> usize {
        self.partitions
            .iter()
            .map(|entry| entry.value().in_flight.len())
            .sum()
    }

    pub fn pending_commit_count(&self) -> usize {
        self.partitions
            .iter()
            .map(|entry| entry.value().completed.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_committable_offset_empty() {
        let state = PartitionState::default();
        assert_eq!(state.committable_offset(), None);
    }

    #[test]
    fn test_committable_offset_all_completed() {
        let mut state = PartitionState::default();
        state.completed.insert(1);
        state.completed.insert(2);
        state.completed.insert(3);

        // No in-flight, commit offset is max completed + 1 = 4
        assert_eq!(state.committable_offset(), Some(4));
    }

    #[test]
    fn test_committable_offset_with_gap() {
        let mut state = PartitionState::default();
        // Offsets 1, 2, 3, 4 arrived
        // 1 and 4 completed, 2 and 3 still in-flight
        state.in_flight.insert(2);
        state.in_flight.insert(3);
        state.completed.insert(1);
        state.completed.insert(4);

        // Can only commit up to offset 1, so commit offset is 2
        assert_eq!(state.committable_offset(), Some(2));
    }

    #[test]
    fn test_committable_offset_blocked() {
        let mut state = PartitionState::default();
        // Offset 1 still in-flight, 2,3,4 completed
        state.in_flight.insert(1);
        state.completed.insert(2);
        state.completed.insert(3);
        state.completed.insert(4);

        // Can't commit anything - offset 1 blocks
        assert_eq!(state.committable_offset(), None);
    }

    #[test]
    fn test_dashmap_concurrent_partitions() {
        // Test that multiple partitions can be accessed concurrently
        use std::thread;

        // Create a DashMap directly for testing (without Kafka consumer)
        let partitions: DashMap<i32, PartitionState> = DashMap::new();

        // Pre-register all partitions
        for partition in 0..4 {
            partitions.insert(partition, PartitionState::default());
        }

        let partitions = Arc::new(partitions);
        let mut handles = vec![];

        // Spawn threads that access different partitions
        for partition in 0..4 {
            let partitions = partitions.clone();
            let handle = thread::spawn(move || {
                for offset in 0..100 {
                    // Track
                    {
                        let mut state = partitions.get_mut(&partition).unwrap();
                        state.in_flight.insert(offset);
                    }

                    // Complete
                    {
                        let mut state = partitions.get_mut(&partition).unwrap();
                        state.in_flight.remove(&offset);
                        state.completed.insert(offset);
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify all partitions have correct state
        for partition in 0..4 {
            let state = partitions.get(&partition).unwrap();
            assert_eq!(state.in_flight.len(), 0);
            assert_eq!(state.completed.len(), 100);
        }
    }

    #[test]
    fn test_dashmap_same_partition_concurrent() {
        // Test that multiple threads accessing the same partition works correctly
        use std::thread;

        let partitions: DashMap<i32, PartitionState> = DashMap::new();

        // Pre-register partition 0
        partitions.insert(0, PartitionState::default());

        let partitions = Arc::new(partitions);
        let mut handles = vec![];
        let num_threads = 4;
        let offsets_per_thread = 25;

        // Spawn threads that all access partition 0
        for thread_id in 0..num_threads {
            let partitions = partitions.clone();
            let handle = thread::spawn(move || {
                let start_offset = thread_id * offsets_per_thread;
                for i in 0..offsets_per_thread {
                    let offset = (start_offset + i) as i64;

                    // Track
                    {
                        let mut state = partitions.get_mut(&0).unwrap();
                        state.in_flight.insert(offset);
                    }

                    // Small yield to increase chance of interleaving
                    thread::yield_now();

                    // Complete
                    {
                        let mut state = partitions.get_mut(&0).unwrap();
                        state.in_flight.remove(&offset);
                        state.completed.insert(offset);
                    }
                }
            });
            handles.push(handle);
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Verify partition 0 has correct state
        let state = partitions.get(&0).unwrap();
        assert_eq!(state.in_flight.len(), 0);
        assert_eq!(
            state.completed.len(),
            (num_threads * offsets_per_thread) as usize
        );
    }

    #[test]
    fn test_out_of_order_completion() {
        // Test the commit offset calculation when offsets complete out of order
        let mut state = PartitionState::default();

        // Simulate: offsets 0,1,2,3,4 tracked as in-flight
        for i in 0..5 {
            state.in_flight.insert(i);
        }
        assert_eq!(state.committable_offset(), None);

        // Complete offset 4 first (out of order)
        state.in_flight.remove(&4);
        state.completed.insert(4);
        assert_eq!(state.committable_offset(), None); // Can't commit, 0-3 still in-flight

        // Complete offset 2
        state.in_flight.remove(&2);
        state.completed.insert(2);
        assert_eq!(state.committable_offset(), None); // Can't commit, 0,1,3 still in-flight

        // Complete offset 0
        state.in_flight.remove(&0);
        state.completed.insert(0);
        // Offset 0 done, commit offset is 1 (next to read on restart)
        assert_eq!(state.committable_offset(), Some(1));

        // Complete offset 1
        state.in_flight.remove(&1);
        state.completed.insert(1);
        // Offsets 0,1,2 done (3 still in-flight), commit offset is 3
        assert_eq!(state.committable_offset(), Some(3));

        // Complete offset 3
        state.in_flight.remove(&3);
        state.completed.insert(3);
        // All done (0,1,2,3,4), commit offset is 5
        assert_eq!(state.committable_offset(), Some(5));
    }
}
