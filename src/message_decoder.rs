use {
    anyhow::{anyhow, Context, Result},
    serde_json::Value,
    std::str,
    // solana_block_decoder::transaction_status::EncodedConfirmedBlock,
    solana_block_decoder::{
        block::{
            encoded_block::{
                EncodedConfirmedBlock,
            }
        },
    },
    solana_transaction_status::EntrySummary,
};
use crate::entries_parser::parse_entries_from_value;

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

    /// A block ID with block data and entries summaries together.
    BlockWithEntries(u64, EncodedConfirmedBlock, Vec<EntrySummary>),
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
                // Preferred format: top-level block data with optional entries
                if let Some(block_id) = json_val["blockID"].as_u64() {
                    let entries = if let Some(entries_value) = json_val.get("entries") {
                        parse_entries_from_value(entries_value)
                            .with_context(|| "Failed to parse entries field")?
                    } else {
                        vec![]
                    };

                    // Remove entries before parsing into EncodedConfirmedBlock
                    let block_value = if let Some(mut obj) = json_val.as_object().cloned() {
                        let _ = obj.remove("entries");
                        Value::Object(obj)
                    } else {
                        json_val.clone()
                    };

                    let block: EncodedConfirmedBlock = serde_json::from_value(block_value)
                        .with_context(|| "Failed to parse EncodedConfirmedBlock")?;

                    return Ok(DecodedPayload::BlockWithEntries(block_id, block, entries));
                }

                // Fallback format support for nested { block: {...}, entries: {...} }
                if json_val.get("block").is_some() {
                    let block_value = &json_val["block"];
                    let block_id = block_value["blockID"].as_u64()
                        .ok_or_else(|| anyhow!("Missing block.blockID in payload"))?;
                    let block: EncodedConfirmedBlock = serde_json::from_value(block_value.clone())
                        .with_context(|| "Failed to parse EncodedConfirmedBlock from block field")?;

                    let entries = if let Some(entries_value) = json_val.get("entries") {
                        parse_entries_from_value(entries_value)
                            .with_context(|| "Failed to parse entries field")?
                    } else {
                        vec![]
                    };
                    return Ok(DecodedPayload::BlockWithEntries(block_id, block, entries));
                }

                // Alternatively, JSON may be a file path wrapper
                if let Some(file_path) = json_val["hdfs_path"].as_str() {
                    return Ok(DecodedPayload::FilePath(file_path.to_string()));
                }

                Err(anyhow!("Unrecognized JSON payload: {}", msg_str))
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