use {
    anyhow::{anyhow, Context, Result},
    serde_json::Value,
    std::str,
    solana_binary_encoder::transaction_status::EncodedConfirmedBlock,
};

#[async_trait::async_trait]
pub trait MessageDecoder: Send + Sync {
    /// Decode a raw message into a `DecodedPayload`.
    /// Return an error if it’s invalid or unrecognized.
    async fn decode(&self, data: &[u8]) -> Result<DecodedPayload>;
}

/// Represents what the raw payload actually decodes into.
pub enum DecodedPayload {
    /// A file path that should be processed by `Processor::process_file`.
    FilePath(String),

    /// A block ID plus the block data that should be uploaded to the storage.
    Block(u64, EncodedConfirmedBlock),
}

pub struct JsonMessageDecoder;

#[async_trait::async_trait]
impl MessageDecoder for JsonMessageDecoder {
    async fn decode(&self, data: &[u8]) -> Result<DecodedPayload> {
        // Convert bytes to string
        let msg_str = str::from_utf8(data)
            .map_err(|e| anyhow!("Invalid UTF-8 in message: {}", e))?;

        // Attempt to parse as JSON
        match serde_json::from_str::<Value>(msg_str) {
            Ok(json_val) => {
                // if there’s a "blockID", treat it as a block
                if let Some(block_id) = json_val["blockID"].as_u64() {
                    // parse entire block
                    let block: EncodedConfirmedBlock =
                        serde_json::from_value(json_val)
                            .with_context(|| "Failed to parse EncodedConfirmedBlock")?;

                    Ok(DecodedPayload::Block(block_id, block))
                } else {
                    // If there's no "blockID", maybe it's still a file path in JSON form
                    // e.g. {"filePath": "hdfs://my-file.gz"}
                    if let Some(file_path) = json_val["filePath"].as_str() {
                        Ok(DecodedPayload::FilePath(file_path.to_string()))
                    } else {
                        // unrecognized JSON structure
                        Err(anyhow!("Unrecognized JSON payload: {}", msg_str))
                    }
                }
            }
            Err(_) => {
                // If it fails to parse as JSON, maybe the entire string is a file path
                // e.g. "hdfs://my-file.gz"
                let trimmed = msg_str.trim();
                if trimmed.ends_with(".gz") || trimmed.contains("hdfs://") {
                    Ok(DecodedPayload::FilePath(trimmed.to_string()))
                } else {
                    Err(anyhow!("Unable to decode message as JSON or file path: {}", trimmed))
                }
            }
        }
    }
}