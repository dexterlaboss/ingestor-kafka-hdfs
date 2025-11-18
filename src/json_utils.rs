use {
    anyhow::{anyhow, Result},
    serde::de::DeserializeOwned,
    serde_json::{self, Value},
};

/// Deserialize a serde_json::Value into T and include a precise JSON path on error.
pub fn from_value_with_path<T>(value: Value, type_name: &'static str) -> Result<T>
where
    T: DeserializeOwned,
{
    // Use serde_path_to_error to track the path where deserialization fails
    let json_str = match serde_json::to_string(&value) {
        Ok(s) => s,
        Err(e) => {
            return Err(anyhow!(
                "Failed to serialize intermediate JSON value for {type_name}: {e}"
            ))
        }
    };
    let mut deserializer = serde_json::Deserializer::from_str(&json_str);
    match serde_path_to_error::deserialize::<_, T>(&mut deserializer) {
        Ok(v) => Ok(v),
        Err(err) => {
            let path = err.path().to_string();
            let tx_idx_hint = extract_tx_index_from_path_string(&path)
                .map(|i| format!(" (transaction index: {i})"))
                .unwrap_or_default();
            Err(anyhow!(
                "Deserialization error for {type_name} at path `{path}`{tx_idx_hint}: {}",
                err.inner()
            ))
        }
    }
}

/// Best-effort helper to extract a transaction index from a JSON path like `.transactions[5].meta`
fn extract_tx_index_from_path_string(path: &str) -> Option<usize> {
    // Look for "transactions[" and parse the following number
    if let Some(start) = path.find("transactions[") {
        let after = &path[start + "transactions[".len()..];
        let mut digits = String::new();
        for ch in after.chars() {
            if ch.is_ascii_digit() {
                digits.push(ch);
            } else {
                break;
            }
        }
        if !digits.is_empty() {
            return digits.parse::<usize>().ok();
        }
    }
    None
}


