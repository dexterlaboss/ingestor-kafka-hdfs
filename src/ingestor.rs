use {
    crate::{
        file_processor::Processor, message_decoder::MessageDecoder, queue_consumer::QueueConsumer,
        queue_producer::QueueProducer,
    },
    anyhow::Result,
    bytes::BytesMut,
    log::{info, warn},
    std::sync::Arc,
};

pub struct Ingestor<C, P> {
    consumer: C,
    producer: P,
    processor: Arc<dyn Processor + Send + Sync>,
    decoder: Arc<dyn MessageDecoder + Send + Sync>,
}

impl<C, P> Ingestor<C, P>
where
    C: QueueConsumer + Send + Sync,
    P: QueueProducer + Send + Sync,
{
    pub fn new(
        consumer: C,
        producer: P,
        processor: Arc<dyn Processor + Send + Sync>,
        decoder: Arc<dyn MessageDecoder + Send + Sync>,
    ) -> Self {
        Self {
            consumer,
            producer,
            processor,
            decoder,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        info!("Ingestor started");

        while let Some(msg_result) = self.consumer.next_message().await {
            match msg_result {
                Ok(queue_message) => {
                    let payload_str = queue_message.internal();

                    if !payload_str.is_empty() {
                        match self.decoder.decode(payload_str.as_bytes()).await {
                            Ok(decoded) => {
                                // Process the decoded payload
                                if let Err(e) = self.processor.process_decoded(decoded).await {
                                    warn!("Error processing payload: {:?}", e);
                                    self.send_to_dead_letter(
                                        payload_str.as_bytes(),
                                        &e.to_string(),
                                    )
                                    .await;
                                }

                                if let Err(e) = self.consumer.commit(&queue_message).await {
                                    warn!("Failed to commit offset: {:?}", e);
                                }
                            }
                            Err(decode_err) => {
                                warn!("Failed to decode payload: {:?}", decode_err);
                                self.send_to_dead_letter(
                                    payload_str.as_bytes(),
                                    &decode_err.to_string(),
                                )
                                .await;
                            }
                        }
                    } else {
                        warn!("Received empty payload from queue");
                    }
                }
                Err(e) => {
                    warn!("Error retrieving message from queue: {:?}", e);
                }
            }
        }

        Ok(())
    }

    async fn send_to_dead_letter(&self, msg: &[u8], error_str: &str) {
        let dlq_payload = format!(
            "Failed to process '{}': {}",
            String::from_utf8_lossy(msg),
            error_str
        );
        let payload_bytes = BytesMut::from(dlq_payload.as_str());
        let _ = self.producer.produce_message(payload_bytes, None).await;
    }
}
