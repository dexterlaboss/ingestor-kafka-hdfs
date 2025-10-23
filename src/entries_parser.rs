use {
    anyhow::{Context, Result},
    serde_json::Value,
    solana_hash::Hash,
    solana_transaction_status::EntrySummary,
};

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
            .map(|e| parse_entry_summary(e))
            .collect();
    }
    if let Some(obj) = v.as_object() {
        if let Some(inner) = obj.get("entries") {
            return parse_entries_from_value(inner);
        }
        return Ok(vec![parse_entry_summary(v)?]);
    }
    anyhow::bail!("entries must be an array or object");
}

pub fn parse_entry_summary(v: &Value) -> Result<EntrySummary> {
    let je: JsonEntrySummary = serde_json::from_value(v.clone())
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


