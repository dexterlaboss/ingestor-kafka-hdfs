use {
    crate::{
        ledger_storage::LedgerStorage
    },
    anyhow::{Context, Result},
    // solana_block_decoder::{
    //     transaction_status::{
    //         BlockEncodingOptions,
    //         EncodedConfirmedBlock,
    //         TransactionDetails,
    //         UiTransactionEncoding,
    //     },
    //     convert_block,
    // },
    solana_transaction_status::{
        BlockEncodingOptions,
        UiTransactionEncoding,
        TransactionDetails,
    },
    solana_block_decoder::{
        block::{
            encoded_block::{
                EncodedConfirmedBlock,
            }
        },
        convert_block,
    },
    solana_transaction_status::{EntrySummary, VersionedConfirmedBlockWithEntries},
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

    /// Handle a block that already includes entries summaries. If the --write-block-entries flag is
    /// off, we still accept and upload the block, and ignore entries at storage layer.
    pub async fn handle_block_with_entries(
        &self,
        block_id: u64,
        block: EncodedConfirmedBlock,
        entries: Vec<EntrySummary>,
    ) -> Result<()> {
        let options = BlockEncodingOptions {
            transaction_details: TransactionDetails::Full,
            show_rewards: true,
            max_supported_transaction_version: Some(0),
        };
        let versioned_block = convert_block(block, UiTransactionEncoding::Json, options)
            .map_err(|e| anyhow::anyhow!("Failed to convert block: {}", e))?;

        let with_entries = VersionedConfirmedBlockWithEntries { block: versioned_block, entries };

        self.storage
            .upload_confirmed_block_with_entries(block_id, with_entries)
            .await
            .context("Failed to upload confirmed block with entries")?;

        Ok(())
    }
}