use crate::entries_parser::parse_entries_from_value;
use {
    anyhow::{Context, Result},
    serde_json::Value,
    solana_block_decoder::block::encoded_block::EncodedConfirmedBlock,
    solana_transaction_status::EntrySummary,
};

pub trait FormatParser: Send + Sync {
    /// Parse a single record (line) into `(block_id, EncodedConfirmedBlock, entries)` or `None` if invalid.
    fn parse_record(
        &self,
        record: &str,
    ) -> Result<Option<(u64, EncodedConfirmedBlock, Vec<EntrySummary>)>>;
}

pub struct NdJsonParser;

impl FormatParser for NdJsonParser {
    fn parse_record(
        &self,
        record: &str,
    ) -> Result<Option<(u64, EncodedConfirmedBlock, Vec<EntrySummary>)>> {
        let trimmed = record.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }

        // We assume a "blockID" field exists in the JSON.
        let value: Value = serde_json::from_str(trimmed)
            .with_context(|| format!("Failed to parse JSON line: {}", trimmed))?;

        // Preferred format: top-level block data with optional entries
        let block_id = if let Some(id) = value["blockID"].as_u64() {
            id
        } else if let Some(id_str) = value["blockID"].as_str() {
            id_str
                .parse::<u64>()
                .context("Failed to parse blockID string as u64")?
        } else {
            // Fallback format: nested { block: {...}, entries: {...} }
            if let Some(block_value) = value.get("block") {
                let block_id = block_value["blockID"]
                    .as_u64()
                    .context("Missing block.blockID in record")?;
                let entries = if let Some(entries_value) = value.get("entries") {
                    parse_entries_from_value(entries_value)?
                } else {
                    vec![]
                };
                let block: EncodedConfirmedBlock = serde_json::from_value(block_value.clone())
                    .context("Failed to parse EncodedConfirmedBlock from block field")?;
                return Ok(Some((block_id, block, entries)));
            } else {
                return Ok(None);
            }
        };

        // Extract optional entries then remove before parsing block
        let entries = if let Some(entries_value) = value.get("entries") {
            parse_entries_from_value(entries_value)?
        } else {
            vec![]
        };

        let block_value = if let Some(mut obj) = value.as_object().cloned() {
            let _ = obj.remove("entries");
            Value::Object(obj)
        } else {
            value.clone()
        };

        let block: EncodedConfirmedBlock =
            serde_json::from_value(block_value).context("Failed to parse EncodedConfirmedBlock")?;

        Ok(Some((block_id, block, entries)))
    }
}
