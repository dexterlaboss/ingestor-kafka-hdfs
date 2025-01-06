use {
    crate::{
        ledger_storage::LedgerStorage
    },
    anyhow::{Context, Result},
    solana_binary_encoder::{
        transaction_status::{
            BlockEncodingOptions, EncodedConfirmedBlock, TransactionDetails, UiTransactionEncoding,
        },
        convert_block,
    },
};

pub struct BlockProcessor {
    storage: LedgerStorage,
}

impl BlockProcessor {
    pub fn new(storage: LedgerStorage) -> Self {
        Self { storage }
    }

    /// Takes a block ID and the `EncodedConfirmedBlock`, converts it, and uploads it.
    pub async fn handle_block(&self, block_id: u64, block: EncodedConfirmedBlock) -> Result<()> {
        let options = BlockEncodingOptions {
            transaction_details: TransactionDetails::Full,
            show_rewards: true,
            max_supported_transaction_version: Some(0),
        };
        let versioned_block = convert_block(block, UiTransactionEncoding::Json, options)
            .map_err(|e| anyhow::anyhow!("Failed to convert block: {}", e))?;

        self.storage
            .upload_confirmed_block(block_id, versioned_block)
            .await
            .context("Failed to upload confirmed block")?;

        Ok(())
    }
}