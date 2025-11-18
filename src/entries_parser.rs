use {
    anyhow::{Context, Result},
    serde_json::Value,
    solana_hash::Hash,
    solana_transaction_status::EntrySummary,
};
use crate::json_utils::from_value_with_path;

#[derive(serde::Deserialize)]
struct JsonEntrySummary {
    #[serde(alias = "numHashes")]
    num_hashes: u64,
    #[serde(alias = "hash")]
    hash: String,
    #[serde(alias = "numTransactions")]
    num_transactions: u64,
    #[serde(alias = "startingTransactionIndex")]
    starting_transaction_index: usize,
}

pub fn parse_entries_from_value(v: &Value) -> Result<Vec<EntrySummary>> {
    if v.is_null() {
        return Ok(vec![]);
    }
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .enumerate()
            .map(|(i, e)| {
                parse_entry_summary(e)
                    .with_context(|| format!("Invalid entries element at entries[{i}]"))
            })
            .collect();
    }
    if let Some(obj) = v.as_object() {
        if let Some(inner) = obj.get("entries") {
            return parse_entries_from_value(inner);
        }
        return Ok(vec![parse_entry_summary(v)
            .with_context(|| "Invalid entries object at entries[0]")?]);
    }
    anyhow::bail!("entries must be an array or object");
}

pub fn parse_entry_summary(v: &Value) -> Result<EntrySummary> {
    let je: JsonEntrySummary = from_value_with_path(v.clone(), "EntrySummary")
        .with_context(|| "Invalid entry summary object")?;
    let hash: Hash = je
        .hash
        .parse()
        .with_context(|| format!("Invalid entry hash: {}", je.hash))?;
    Ok(EntrySummary {
        num_hashes: je.num_hashes,
        hash,
        num_transactions: je.num_transactions,
        starting_transaction_index: je.starting_transaction_index,
    })
}
