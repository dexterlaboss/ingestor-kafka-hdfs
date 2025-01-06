use {
    anyhow::Result,
    async_trait::async_trait,
    rdkafka::{
        config::{ClientConfig, RDKafkaLogLevel},
        consumer::{CommitMode, Consumer, StreamConsumer},
        Message as RDKafkaMessage,
    },
};

pub struct QueueMessage<T> {
    internal: T,
}

impl<T> QueueMessage<T> {
    pub fn new(internal: T) -> Self {
        Self { internal }
    }

    pub fn internal(&self) -> &T {
        &self.internal
    }
}

#[async_trait]
pub trait QueueConsumer: Send + Sync {
    async fn next_message(&mut self) -> Option<Result<QueueMessage<String>>>;
    async fn commit(&self, message: &QueueMessage<String>) -> Result<()>;
}


// Blanket implementation for Box<dyn QueueConsumer + Send + Sync>
// This lets `Box<dyn QueueConsumer + Send + Sync>` be treated as a `QueueConsumer`.
#[async_trait]
impl<T> QueueConsumer for Box<T>
where
    T: QueueConsumer + Send + Sync + ?Sized,
{
    async fn next_message(&mut self) -> Option<Result<QueueMessage<String>>> {
        T::next_message(self).await
    }

    async fn commit(&self, message: &QueueMessage<String>) -> Result<()> {
        T::commit(self, message).await
    }
}

/// Kafka consumer configuration.
pub struct KafkaConfig {
    pub group_id: String,
    pub bootstrap_servers: String,
    pub enable_partition_eof: bool,
    pub session_timeout_ms: u32,
    pub enable_auto_commit: bool,
    pub auto_offset_reset: String,
    pub max_partition_fetch_bytes: u32,
    pub max_in_flight_requests_per_connection: u32,
    pub log_level: RDKafkaLogLevel,
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            group_id: "default-group".to_string(),
            bootstrap_servers: "localhost:9092".to_string(),
            enable_partition_eof: false,
            session_timeout_ms: 10000,
            enable_auto_commit: true,
            auto_offset_reset: "earliest".to_string(),
            max_partition_fetch_bytes: 10 * 1024 * 1024, // 10 MiB
            max_in_flight_requests_per_connection: 1,
            log_level: RDKafkaLogLevel::Debug,
        }
    }
}

/// Kafka queue consumer.
pub struct KafkaQueueConsumer {
    kafka_consumer: StreamConsumer,
}

impl KafkaQueueConsumer {
    /// Creates a new `KafkaQueueConsumer` with the given configuration and topics.
    pub fn new(config: KafkaConfig, topics: &[&str]) -> Result<Self> {
        let consumer: StreamConsumer = ClientConfig::new()
            .set("group.id", &config.group_id)
            .set("bootstrap.servers", &config.bootstrap_servers)
            .set("enable.partition.eof", config.enable_partition_eof.to_string())
            .set("session.timeout.ms", config.session_timeout_ms.to_string())
            .set("enable.auto.commit", config.enable_auto_commit.to_string())
            .set("auto.offset.reset", &config.auto_offset_reset)
            .set(
                "max.partition.fetch.bytes",
                config.max_partition_fetch_bytes.to_string(),
            )
            .set(
                "max.in.flight.requests.per.connection",
                config.max_in_flight_requests_per_connection.to_string(),
            )
            .set_log_level(config.log_level)
            .create()?;

        consumer.subscribe(topics)?;

        Ok(Self {
            kafka_consumer: consumer,
        })
    }
}

#[async_trait]
impl QueueConsumer for KafkaQueueConsumer {
    /// Fetches the next message from Kafka, converting its payload into a `String`.
    async fn next_message(&mut self) -> Option<Result<QueueMessage<String>>> {
        match self.kafka_consumer.recv().await {
            Ok(msg) => {
                // Convert the payload bytes into a String
                let payload_str = match msg.payload() {
                    Some(bytes) => String::from_utf8_lossy(bytes).to_string(),
                    None => String::new(),
                };
                Some(Ok(QueueMessage::new(payload_str)))
            }
            Err(e) => Some(Err(anyhow::anyhow!("Kafka error: {}", e))),
        }
    }

    /// Commits the consumer state asynchronously.
    async fn commit(&self, _message: &QueueMessage<String>) -> Result<()> {
        self.kafka_consumer
            .commit_consumer_state(CommitMode::Async)
            .map_err(|e| anyhow::anyhow!("Commit error: {:?}", e))
    }
}
