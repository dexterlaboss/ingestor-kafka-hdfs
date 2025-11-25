use {
    anyhow::Result,
    bytes::BytesMut,
    rdkafka::{
        message::{Header, OwnedHeaders},
        producer::{FutureProducer, FutureRecord},
    },
};

#[async_trait::async_trait]
pub trait QueueProducer {
    async fn produce_message(
        &self,
        payload: BytesMut,
        headers: Option<Vec<(&str, &str)>>,
    ) -> Result<()>;
}

pub struct KafkaQueueProducer {
    kafka_producer: FutureProducer,
    topic: String,
}

impl KafkaQueueProducer {
    pub fn new(brokers: &str, topic: &str, message_max_bytes: u32) -> Result<Self> {
        let producer: FutureProducer = rdkafka::config::ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.max.bytes", message_max_bytes.to_string())
            .create()?;
        Ok(Self {
            kafka_producer: producer,
            topic: topic.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl QueueProducer for KafkaQueueProducer {
    async fn produce_message(
        &self,
        payload: BytesMut,
        headers: Option<Vec<(&str, &str)>>,
    ) -> Result<()> {
        let payload_vec = payload.to_vec();

        let mut record: FutureRecord<'_, (), Vec<u8>> =
            FutureRecord::to(&self.topic).payload(&payload_vec);

        // Add headers if provided
        if let Some(hdrs) = headers {
            let mut owned_headers = OwnedHeaders::new();
            for (key, value) in hdrs {
                let header = Header {
                    key,
                    value: Some(value.as_bytes()),
                };
                owned_headers = owned_headers.insert(header);
            }
            record = record.headers(owned_headers);
        }

        self.kafka_producer
            .send(record, std::time::Duration::from_secs(0))
            .await
            .map_err(|(e, _)| anyhow::anyhow!(e))?;

        Ok(())
    }
}
