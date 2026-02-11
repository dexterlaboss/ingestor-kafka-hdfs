//! Multi-threaded Kafka ingestor with parallel message processing.
//!
//! This module provides `ParallelIngestor`, which consumes messages from Kafka
//! and dispatches them to a pool of worker tasks for parallel processing.
//! Offset commits are handled safely via the `OffsetTracker` to ensure no data loss.
//!
//! # Architecture
//!
//! ```text
//! Kafka Consumer ──► Channel ──► Worker Pool ──► Processor
//!       │                              │
//!       └── OffsetTracker ◄────────────┘
//!              (tracks in-flight, commits watermark)
//! ```

use {
    crate::{
        file_processor::Processor,
        message_decoder::{DecodedPayload, MessageDecoder},
        offset_tracker::OffsetTracker,
        queue_producer::QueueProducer,
    },
    anyhow::Result,
    async_channel::{Receiver, Sender},
    bytes::BytesMut,
    log::{debug, error, info, warn},
    rdkafka::{Message, consumer::StreamConsumer},
    serde_json::json,
    std::sync::Arc,
};

/// A work item containing all information needed to process a message.
struct WorkItem {
    /// The topic this message came from.
    topic: String,
    /// The partition this message came from.
    partition: i32,
    /// The offset of this message.
    offset: i64,
    /// The decoded payload ready for processing.
    payload: DecodedPayload,
    /// The raw payload string (for DLQ on error).
    raw_payload: String,
}

/// Multi-threaded Kafka ingestor with parallel message processing.
///
/// Uses a bounded channel to dispatch work to a pool of workers, with
/// backpressure to prevent unbounded memory growth. Offset commits are
/// managed by `OffsetTracker` to guarantee no data loss.
pub struct ParallelIngestor {
    consumer: Arc<StreamConsumer>,
    producer: Arc<dyn QueueProducer + Send + Sync>,
    processor: Arc<dyn Processor + Send + Sync>,
    decoder: Arc<dyn MessageDecoder + Send + Sync>,
    offset_tracker: Arc<OffsetTracker>,
    num_workers: usize,
}

impl ParallelIngestor {
    /// Create a new parallel ingestor.
    ///
    /// # Arguments
    ///
    /// * `consumer` - The Kafka consumer (already subscribed to topics)
    /// * `producer` - The dead-letter queue producer
    /// * `processor` - The message processor
    /// * `decoder` - The message decoder
    /// * `num_workers` - Number of worker tasks (recommend: num_cpus)
    pub fn new<P>(
        consumer: Arc<StreamConsumer>,
        producer: P,
        processor: Arc<dyn Processor + Send + Sync>,
        decoder: Arc<dyn MessageDecoder + Send + Sync>,
        num_workers: usize,
    ) -> Self
    where
        P: QueueProducer + Send + Sync + 'static,
    {
        let num_workers = num_workers.max(1);

        let offset_tracker = Arc::new(OffsetTracker::new(consumer.clone()));

        Self {
            consumer,
            producer: Arc::new(producer),
            processor,
            decoder,
            offset_tracker,
            num_workers,
        }
    }

    /// Run the ingestor until shutdown.
    ///
    /// This spawns worker tasks and runs the consumer loop. The function
    /// returns when the consumer is closed or an unrecoverable error occurs.
    pub async fn run(self) -> Result<()> {
        info!(
            "Starting parallel ingestor with {} workers",
            self.num_workers
        );

        // Create a bounded channel for work distribution.
        let (tx, rx) = async_channel::bounded::<WorkItem>(self.num_workers * 2);

        let mut worker_handles = Vec::with_capacity(self.num_workers);
        for worker_id in 0..self.num_workers {
            let rx = rx.clone();
            let processor = self.processor.clone();
            let offset_tracker = self.offset_tracker.clone();
            let producer = self.producer.clone();

            let handle = tokio::spawn(async move {
                worker_loop(worker_id, rx, processor, offset_tracker, producer).await
            });
            worker_handles.push(handle);
        }

        let consumer_result = self.consumer_loop(tx).await;

        info!("Consumer loop ended, waiting for workers to finish...");
        for (id, handle) in worker_handles.into_iter().enumerate() {
            if let Err(e) = handle.await {
                error!("Worker {} panicked: {:?}", id, e);
            }
        }

        let in_flight = self.offset_tracker.in_flight_count();
        let pending = self.offset_tracker.pending_commit_count();
        if in_flight > 0 || pending > 0 {
            warn!(
                "Shutdown with {} in-flight and {} pending commit messages",
                in_flight, pending
            );
        }

        consumer_result
    }

    async fn consumer_loop(&self, tx: Sender<WorkItem>) -> Result<()> {
        info!("Consumer loop started");

        loop {
            let msg = match self.consumer.recv().await {
                Ok(msg) => msg,
                Err(e) => {
                    error!("Kafka receive error: {:?}", e);
                    continue;
                }
            };

            let topic = msg.topic().to_string();
            let partition = msg.partition();
            let offset = msg.offset();

            self.offset_tracker.register_partition(&topic, partition);
            if let Err(e) = self.offset_tracker.track(&topic, partition, offset) {
                error!(
                    "Failed to track offset topic={} partition={} offset={}: {:?}",
                    topic, partition, offset, e
                );
                continue;
            }

            let payload_bytes = match msg.payload() {
                Some(bytes) => bytes,
                None => {
                    warn!("Empty payload at topic={} partition={} offset={}", topic, partition, offset);
                    if let Err(e) = self.offset_tracker.complete(&topic, partition, offset) {
                        error!(
                            "Failed to complete offset topic={} partition={} offset={}: {:?}",
                            topic, partition, offset, e
                        );
                    }
                    continue;
                }
            };

            let raw_payload = String::from_utf8_lossy(payload_bytes).to_string();

            let decoded = match self.decoder.decode(payload_bytes).await {
                Ok(decoded) => decoded,
                Err(e) => {
                    error!(
                        "Failed to decode message at topic={} partition={} offset={}: {:?}",
                        topic, partition, offset, e
                    );
                    send_to_dead_letter(&*self.producer, &raw_payload, &e.to_string()).await;
                    if let Err(e) = self.offset_tracker.complete(&topic, partition, offset) {
                        error!(
                            "Failed to complete offset topic={} partition={} offset={}: {:?}",
                            topic, partition, offset, e
                        );
                    }
                    continue;
                }
            };

            let work_item = WorkItem {
                topic,
                partition,
                offset,
                payload: decoded,
                raw_payload,
            };

            // this will block if channel is full
            if tx.send(work_item).await.is_err() {
                info!("Work channel closed, shutting down consumer");
                break;
            }
        }

        Ok(())
    }
}

async fn worker_loop(
    worker_id: usize,
    rx: Receiver<WorkItem>,
    processor: Arc<dyn Processor + Send + Sync>,
    offset_tracker: Arc<OffsetTracker>,
    producer: Arc<dyn QueueProducer + Send + Sync>,
) {
    while let Ok(work_item) = rx.recv().await {
        let WorkItem {
            topic,
            partition,
            offset,
            payload,
            raw_payload,
        } = work_item;

        debug!(
            "Worker {} processing topic={} partition={} offset={}",
            worker_id, topic, partition, offset
        );

        let result = processor.process_decoded(payload).await;

        if let Err(e) = &result {
            error!(
                "Worker {} failed to process topic={} partition={} offset={}: {:?}",
                worker_id, topic, partition, offset, e
            );
            send_to_dead_letter(&*producer, &raw_payload, &e.to_string()).await;
        }

        debug!(
            "Worker {} completed topic={} partition={} offset={} success={}",
            worker_id,
            topic,
            partition,
            offset,
            result.is_ok()
        );

        if let Err(e) = offset_tracker.complete(&topic, partition, offset) {
            error!(
                "Worker {} failed to complete offset topic={} partition={} offset={}: {:?}",
                worker_id, topic, partition, offset, e
            );
        }
    }

    info!("Worker {} shutting down", worker_id);
}

async fn send_to_dead_letter(producer: &(dyn QueueProducer + Send + Sync), msg: &str, error_str: &str) {
    let dlq_payload = json!({
        "message": msg,
        "error": error_str,
    })
    .to_string();

    let payload_bytes = BytesMut::from(dlq_payload.as_str());
    if let Err(e) = producer.produce_message(payload_bytes, None).await {
        error!("Failed to send to dead-letter queue: {:?}", e);
    }
}