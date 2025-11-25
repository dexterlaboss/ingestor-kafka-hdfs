use solana_sdk::signature::Signature;
use xxhash_rust::{xxh3::xxh3_128, xxh32::xxh32};
use {
    crate::hbase::{Error as HBaseError, HBaseConnection},
    dexter_storage_proto_tx::convert::generated,
    log::{debug, error, info},
    memcache::{Client, MemcacheError},
    serde::{Deserialize, Serialize},
    solana_hash::Hash,
    solana_sdk::{
        clock::{Slot, UnixTimestamp},
        message::{v0::LoadedAddresses, VersionedMessage},
        pubkey::Pubkey,
        transaction::{TransactionError, VersionedTransaction},
    },
    agave_reserved_account_keys::ReservedAccountKeys,
    solana_message::compiled_instruction::CompiledInstruction,
    solana_storage_proto::convert::{entries, tx_by_addr},
    solana_storage_utils::compression::compress_best,
    // extract_memos::extract_and_fmt_memos,
    solana_transaction_status::{
        extract_memos::extract_and_fmt_memos, ConfirmedTransactionWithStatusMeta, EntrySummary,
        TransactionByAddrInfo, VersionedConfirmedBlock, VersionedConfirmedBlockWithEntries,
        VersionedTransactionWithStatusMeta,
    },
    // solana_block_decoder::{
    //     compression::compress_best,
    //     convert::{generated, tx_by_addr},
    //     extract_memos,
    //     transaction_status::{
    //         ConfirmedTransactionWithStatusMeta,
    //         TransactionByAddrInfo,
    //         TransactionStatusMeta,
    //         TransactionWithStatusMeta,
    //         VersionedConfirmedBlock,
    //         VersionedTransactionWithStatusMeta,
    //     },
    // },
    solana_transaction_status::{
        // ConfirmedBlock,
        TransactionStatusMeta,
        TransactionWithStatusMeta,
        // VersionedTransactionWithStatusMeta,
        // Reward,
    },
    std::{
        collections::{HashMap, HashSet},
        str::FromStr,
    },
    thiserror::Error,
    tokio::task::JoinError,
};

#[derive(Debug, Error)]
pub enum Error {
    #[error("HBase: {0}")]
    HBaseError(HBaseError),

    #[error("I/O Error: {0}")]
    IoError(std::io::Error),

    #[error("tokio error")]
    TokioJoinError(JoinError),

    #[error("Memcache error: {0}")]
    MemcacheError(MemcacheError),

    #[error("Protobuf error: {0}")]
    EncodingError(prost::EncodeError),
}

impl std::convert::From<HBaseError> for Error {
    fn from(err: HBaseError) -> Self {
        Self::HBaseError(err)
    }
}

impl std::convert::From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err)
    }
}

impl From<MemcacheError> for Error {
    fn from(err: MemcacheError) -> Self {
        Self::MemcacheError(err)
    }
}

impl From<TaskError> for Error {
    fn from(err: TaskError) -> Self {
        match err {
            TaskError::HBaseError(hbase_err) => Error::HBaseError(hbase_err),
            TaskError::MemcacheError(memcache_err) => Error::MemcacheError(memcache_err),
            TaskError::IoError(io_err) => Error::IoError(io_err),
            TaskError::EncodingError(enc_err) => Error::EncodingError(enc_err),
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

enum TaskResult {
    BytesWritten(usize),
    CachedTransactions(usize),
}

#[derive(Debug)]
enum TaskType {
    UploadTx,
    UploadTxByAddr,
    UploadFullTx,
    CacheFullTx,
    UploadEntries,
}

#[derive(Debug)]
enum TaskError {
    HBaseError(HBaseError),
    MemcacheError(MemcacheError),
    IoError(std::io::Error),
    EncodingError(prost::EncodeError),
}

impl From<std::io::Error> for TaskError {
    fn from(err: std::io::Error) -> Self {
        TaskError::IoError(err)
    }
}

impl From<HBaseError> for TaskError {
    fn from(err: HBaseError) -> Self {
        TaskError::HBaseError(err)
    }
}

impl From<MemcacheError> for TaskError {
    fn from(err: MemcacheError) -> Self {
        TaskError::MemcacheError(err)
    }
}

struct TaskErrorWithType {
    task_type: TaskType,
    err: TaskError,
}

impl TaskErrorWithType {
    fn new<T>(task_type: TaskType, err: T) -> Self
    where
        TaskError: From<T>,
    {
        Self {
            task_type,
            err: TaskError::from(err),
        }
    }
}

#[derive(Debug)]
pub enum CacheWriteError {
    MemcacheError(MemcacheError), // Error from cache client
    IoError(std::io::Error),      // Error from encoding (e.g., Protobuf)
    EncodingError(prost::EncodeError),
}

impl From<CacheWriteError> for TaskError {
    fn from(error: CacheWriteError) -> Self {
        match error {
            CacheWriteError::MemcacheError(e) => TaskError::MemcacheError(e),
            CacheWriteError::IoError(e) => TaskError::IoError(e),
            CacheWriteError::EncodingError(e) => TaskError::EncodingError(e),
        }
    }
}

// Convert a slot to its bucket representation whereby lower slots are always lexically ordered
// before higher slots
fn slot_to_key(slot: Slot) -> String {
    format!("{slot:016x}")
}

fn slot_to_blocks_key(slot: Slot, use_md5: bool) -> String {
    let slot_hex = slot_to_key(slot);

    if use_md5 {
        let hash_result = md5::compute(&slot_hex);
        let truncated_hash_hex = format!("{:x}", hash_result)[..10].to_string();

        format!("{}{}", truncated_hash_hex, slot_hex)
    } else {
        slot_hex
    }
}

fn slot_to_tx_by_addr_key(slot: Slot) -> String {
    slot_to_key(!slot)
}

fn signature_to_tx_full_key(signature: Signature, use_hash: bool) -> String {
    if use_hash {
        let signature_bytes = signature.as_ref(); // Convert signature to bytes
        let hash_128 = xxh3_128(signature_bytes);
        let hash_32 = xxh32(signature_bytes, 0);

        // Concatenate the two hashes for a 160bit hash
        format!("{:x}{:x}", hash_128, hash_32)
    } else {
        signature.to_string()
    }
}

// A serialized `TransactionInfo` that is stored in the `tx` table
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
struct TransactionInfo {
    slot: Slot, // The slot that contains the block with this transaction in it
    index: u32, // Where the transaction is located in the block
    err: Option<TransactionError>, // None if the transaction executed successfully
                // memo: Option<String>, // Transaction memo
}

#[derive(Serialize, Deserialize)]
pub struct StoredConfirmedTransactionWithStatusMeta {
    pub slot: Slot,
    pub tx_with_meta: StoredConfirmedBlockTransaction,
    pub block_time: Option<UnixTimestamp>,
}

impl From<ConfirmedTransactionWithStatusMeta> for StoredConfirmedTransactionWithStatusMeta {
    fn from(value: ConfirmedTransactionWithStatusMeta) -> Self {
        Self {
            slot: value.slot,
            tx_with_meta: value.tx_with_meta.into(),
            block_time: value.block_time,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct StoredConfirmedBlockTransaction {
    transaction: VersionedTransaction,
    meta: Option<StoredConfirmedBlockTransactionStatusMeta>,
}

impl From<TransactionWithStatusMeta> for StoredConfirmedBlockTransaction {
    fn from(value: TransactionWithStatusMeta) -> Self {
        match value {
            TransactionWithStatusMeta::MissingMetadata(transaction) => Self {
                transaction: VersionedTransaction::from(transaction),
                meta: None,
            },
            TransactionWithStatusMeta::Complete(VersionedTransactionWithStatusMeta {
                transaction,
                meta,
            }) => Self {
                transaction,
                meta: Some(meta.into()),
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct StoredConfirmedBlockTransactionStatusMeta {
    err: Option<TransactionError>,
    fee: u64,
    pre_balances: Vec<u64>,
    post_balances: Vec<u64>,
}

impl From<TransactionStatusMeta> for StoredConfirmedBlockTransactionStatusMeta {
    fn from(value: TransactionStatusMeta) -> Self {
        let TransactionStatusMeta {
            status,
            fee,
            pre_balances,
            post_balances,
            ..
        } = value;
        Self {
            err: status.err(),
            fee,
            pre_balances,
            post_balances,
        }
    }
}

pub const DEFAULT_ADDRESS: &str = "127.0.0.1:9090";
pub const BLOCKS_TABLE_NAME: &str = "blocks";
pub const TX_TABLE_NAME: &str = "tx";
pub const TX_BY_ADDR_TABLE_NAME: &str = "tx-by-addr";
pub const FULL_TX_TABLE_NAME: &str = "tx_full";
pub const ENTRIES_TABLE_NAME: &str = "entries";
pub const DEFAULT_MEMCACHE_ADDRESS: &str = "127.0.0.1:11211";
pub const DEFAULT_MEMCACHE_TIMEOUT_SECS: u64 = 1;

#[derive(Debug)]
pub struct LedgerStorageConfig {
    pub address: String,
    pub namespace: Option<String>,
    pub uploader_config: UploaderConfig,
    pub cache_config: LedgerCacheConfig,
}

impl Default for LedgerStorageConfig {
    fn default() -> Self {
        Self {
            address: DEFAULT_ADDRESS.to_string(),
            namespace: None,
            uploader_config: UploaderConfig::default(),
            cache_config: LedgerCacheConfig::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LedgerCacheConfig {
    pub enable_full_tx_cache: bool,
    pub address: String,
    pub timeout: Option<std::time::Duration>,
    pub tx_cache_expiration: Option<std::time::Duration>,
}

impl Default for LedgerCacheConfig {
    fn default() -> Self {
        Self {
            enable_full_tx_cache: false,
            address: DEFAULT_MEMCACHE_ADDRESS.to_string(),
            timeout: Some(std::time::Duration::from_secs(
                DEFAULT_MEMCACHE_TIMEOUT_SECS,
            )),
            tx_cache_expiration: Some(std::time::Duration::from_secs(60 * 60 * 24 * 14)), // 14 days
        }
    }
}

#[derive(Debug, Clone)]
pub struct UploaderConfig {
    pub tx_full_filter: Option<FilterTxIncludeExclude>,
    pub tx_by_addr_filter: Option<FilterTxIncludeExclude>,
    pub disable_tx: bool,
    pub disable_tx_by_addr: bool,
    pub disable_blocks: bool,
    pub enable_full_tx: bool,
    pub blocks_table_name: String,
    pub tx_table_name: String,
    pub tx_by_addr_table_name: String,
    pub full_tx_table_name: String,
    pub use_md5_row_key_salt: bool,
    pub hash_tx_full_row_keys: bool,
    pub filter_program_accounts: bool,
    pub filter_readonly_accounts: bool,
    pub filter_tx_voting: bool,
    pub filter_tx_by_addr_voting: bool,
    pub filter_tx_full_voting: bool,
    pub filter_all_voting: bool,
    pub filter_tx_error: bool,
    pub filter_tx_by_addr_error: bool,
    pub filter_tx_full_error: bool,
    pub filter_all_error: bool,
    pub use_blocks_compression: bool,
    pub use_tx_compression: bool,
    pub use_tx_by_addr_compression: bool,
    pub use_tx_full_compression: bool,
    pub hbase_write_to_wal: bool,
    pub write_block_entries: bool,
    pub entries_table_name: String,
}

impl Default for UploaderConfig {
    fn default() -> Self {
        Self {
            tx_full_filter: None,
            tx_by_addr_filter: None,
            disable_tx: false,
            disable_tx_by_addr: false,
            disable_blocks: false,
            enable_full_tx: false,
            blocks_table_name: BLOCKS_TABLE_NAME.to_string(),
            tx_table_name: TX_TABLE_NAME.to_string(),
            tx_by_addr_table_name: TX_BY_ADDR_TABLE_NAME.to_string(),
            full_tx_table_name: FULL_TX_TABLE_NAME.to_string(),
            use_md5_row_key_salt: false,
            hash_tx_full_row_keys: false,
            filter_program_accounts: false,
            filter_readonly_accounts: false,
            filter_tx_voting: false,
            filter_tx_by_addr_voting: false,
            filter_tx_full_voting: false,
            filter_all_voting: false,
            filter_tx_error: false,
            filter_tx_by_addr_error: false,
            filter_tx_full_error: false,
            filter_all_error: false,
            use_blocks_compression: true,
            use_tx_compression: true,
            use_tx_by_addr_compression: true,
            use_tx_full_compression: true,
            hbase_write_to_wal: true,
            write_block_entries: false,
            entries_table_name: ENTRIES_TABLE_NAME.to_string(),
        }
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct FilterTxIncludeExclude {
    pub exclude: bool,
    pub addrs: HashSet<Pubkey>,
}

#[derive(Clone)]
pub struct LedgerStorage {
    connection: HBaseConnection,
    uploader_config: UploaderConfig,
    cache_client: Option<Client>,
    enable_full_tx_cache: bool,
    tx_cache_expiration: Option<std::time::Duration>,
}

impl LedgerStorage {
    #![allow(unused)]
    pub async fn new() -> Self {
        Self::new_with_config(LedgerStorageConfig {
            ..LedgerStorageConfig::default()
        })
        .await
    }

    pub async fn new_with_config(config: LedgerStorageConfig) -> Self {
        let LedgerStorageConfig {
            address,
            namespace,
            uploader_config,
            cache_config,
        } = config;
        let connection = HBaseConnection::new(address.as_str(), namespace.as_deref()).await;

        let cache_client = if cache_config.enable_full_tx_cache {
            let memcache_timeout_secs = cache_config
                .timeout
                .map(|t| t.as_secs())
                .unwrap_or(DEFAULT_MEMCACHE_TIMEOUT_SECS);

            // Add the "memcache://" prefix
            let memcache_url = format!(
                "memcache://{}?timeout={}&protocol=ascii",
                cache_config.address, memcache_timeout_secs
            );
            Some(Client::connect(memcache_url.as_str()).unwrap())
        } else {
            None
        };

        Self {
            connection,
            uploader_config,
            cache_client,
            enable_full_tx_cache: cache_config.enable_full_tx_cache,
            tx_cache_expiration: cache_config.tx_cache_expiration,
        }
    }

    pub async fn upload_confirmed_block(
        &self,
        slot: Slot,
        confirmed_block: VersionedConfirmedBlock,
    ) -> Result<()> {
        // Delegate to entries-aware upload with empty entries to preserve old behavior
        self.upload_confirmed_block_with_entries(
            slot,
            VersionedConfirmedBlockWithEntries {
                block: confirmed_block,
                entries: vec![],
            },
        )
        .await
    }

    pub async fn upload_confirmed_block_with_entries(
        &self,
        slot: Slot,
        confirmed_block_with_entries: VersionedConfirmedBlockWithEntries,
    ) -> Result<()> {
        let VersionedConfirmedBlockWithEntries {
            block: confirmed_block,
            entries,
        } = confirmed_block_with_entries;
        let mut by_addr: HashMap<&Pubkey, Vec<TransactionByAddrInfo>> = HashMap::new();

        info!(
            "HBase: Uploading block {:?} from slot {:?}",
            confirmed_block.blockhash, slot
        );

        let reserved_account_keys = ReservedAccountKeys::new_all_activated();
        let mut tx_cells = vec![];
        let mut full_tx_cells = vec![];
        let mut full_tx_cache = vec![];
        for (index, transaction_with_meta) in confirmed_block.transactions.iter().enumerate() {
            let VersionedTransactionWithStatusMeta { meta, transaction } = transaction_with_meta;
            let err = meta.status.clone().err();
            let index = index as u32;
            let signature = transaction.signatures[0];
            let memo = extract_and_fmt_memos(transaction_with_meta);

            let mut should_skip_tx = false;
            let mut should_skip_tx_by_addr = false;
            let mut should_skip_full_tx = false;

            let is_voting = is_voting_tx(transaction_with_meta);

            if is_voting {
                if self.uploader_config.filter_tx_voting {
                    should_skip_tx = true;
                }

                if self.uploader_config.filter_tx_by_addr_voting {
                    should_skip_tx_by_addr = true;
                }

                if self.uploader_config.filter_tx_full_voting {
                    should_skip_full_tx = true;
                }

                if self.uploader_config.filter_all_voting {
                    should_skip_tx = true;
                    should_skip_tx_by_addr = true;
                    should_skip_full_tx = true;
                }
            }

            // if self.uploader_config.filter_voting_tx && is_voting {
            //     should_skip_tx_by_addr = true;
            //     should_skip_full_tx = true;
            // }

            let is_error = is_error_tx(transaction_with_meta);

            if is_error {
                if self.uploader_config.filter_tx_error {
                    should_skip_tx = true;
                }

                if self.uploader_config.filter_tx_by_addr_error {
                    should_skip_tx_by_addr = true;
                }

                if self.uploader_config.filter_tx_full_error {
                    should_skip_full_tx = true;
                }

                if self.uploader_config.filter_all_error {
                    should_skip_tx = true;
                    should_skip_tx_by_addr = true;
                    should_skip_full_tx = true;
                }
            }

            // if self.uploader_config.filter_error_tx && is_error {
            //     should_skip_full_tx = true;
            // }

            let combined_keys = get_account_keys(&transaction_with_meta);

            if !should_skip_tx_by_addr {
                for address in transaction_with_meta.account_keys().iter() {
                    // Filter program accounts from tx-by-addr index
                    if self.uploader_config.filter_program_accounts
                        && is_program_account(address, transaction_with_meta, &combined_keys)
                    {
                        continue;
                    }

                    if self.uploader_config.filter_readonly_accounts
                        && is_readonly_account(address, transaction_with_meta)
                    {
                        continue;
                    }

                    if should_skip_full_tx || !self.should_include_in_tx_full(address) {
                        should_skip_full_tx = true;
                    }

                    if !reserved_account_keys.is_reserved(address)
                        && self.should_include_in_tx_by_addr(address)
                    {
                        by_addr
                            .entry(address)
                            .or_default()
                            .push(TransactionByAddrInfo {
                                signature,
                                err: err.clone(),
                                index,
                                memo: memo.clone(),
                                block_time: confirmed_block.block_time,
                            });
                    }
                }
            }

            if self.uploader_config.enable_full_tx && !should_skip_full_tx {
                full_tx_cells.push((
                    signature_to_tx_full_key(signature, self.uploader_config.hash_tx_full_row_keys),
                    ConfirmedTransactionWithStatusMeta {
                        slot,
                        // tx_with_meta: transaction_with_meta.clone().into(),
                        tx_with_meta: convert_to_transaction_with_status_meta(
                            transaction_with_meta.clone(),
                        ),
                        block_time: confirmed_block.block_time,
                    }
                    .into(),
                ));
            }

            if self.enable_full_tx_cache
                && !is_voting
                && !transaction_with_meta.meta.status.is_err()
            {
                full_tx_cache.push((
                    signature_to_tx_full_key(signature, self.uploader_config.hash_tx_full_row_keys),
                    ConfirmedTransactionWithStatusMeta {
                        slot,
                        // tx_with_meta: transaction_with_meta.clone().into(),
                        tx_with_meta: convert_to_transaction_with_status_meta(
                            transaction_with_meta.clone(),
                        ),
                        block_time: confirmed_block.block_time,
                    },
                ));
            }

            if !self.uploader_config.disable_tx && !should_skip_tx {
                tx_cells.push((
                    signature.to_string(),
                    TransactionInfo {
                        slot,
                        index,
                        err,
                        // memo,
                    },
                ));
            }
        }

        let tx_by_addr_cells: Vec<_> = by_addr
            .into_iter()
            .map(|(address, transaction_info_by_addr)| {
                (
                    format!("{}/{}", address, slot_to_tx_by_addr_key(slot)),
                    tx_by_addr::TransactionByAddr {
                        tx_by_addrs: transaction_info_by_addr
                            .into_iter()
                            .map(|by_addr| by_addr.into())
                            .collect(),
                    },
                )
            })
            .collect();

        let mut tasks = vec![];

        if !full_tx_cells.is_empty() && self.uploader_config.enable_full_tx {
            let conn = self.connection.clone();
            let full_tx_table_name = self.uploader_config.full_tx_table_name.clone();
            let use_tx_full_compression = self.uploader_config.use_tx_full_compression.clone();
            let write_to_wal = self.uploader_config.hbase_write_to_wal.clone();
            tasks.push(tokio::spawn(async move {
                conn.put_protobuf_cells_with_retry::<generated::ConfirmedTransactionWithStatusMeta>(
                    full_tx_table_name.as_str(),
                    &full_tx_cells,
                    use_tx_full_compression,
                    write_to_wal,
                )
                .await
                .map(TaskResult::BytesWritten)
                .map_err(|e| TaskErrorWithType::new(TaskType::UploadFullTx, e))
            }));
        }

        if !full_tx_cache.is_empty() && self.enable_full_tx_cache {
            let mut cached_count = 0;
            let cache_client = self.cache_client.clone();
            let tx_cache_expiration = self.tx_cache_expiration;
            debug!("Writing block transactions to cache");
            tasks.push(tokio::spawn(async move {
                for (signature, transaction) in full_tx_cache {
                    if let Some(client) = &cache_client {
                        cache_transaction::<generated::ConfirmedTransactionWithStatusMeta>(
                            &client,
                            &signature,
                            transaction.into(),
                            tx_cache_expiration,
                        )
                        .await
                        .map_err(|e| TaskErrorWithType::new(TaskType::CacheFullTx, e))?;

                        cached_count += 1;
                        debug!("Cached transaction with signature {}", signature);
                    }
                }
                Ok::<TaskResult, TaskErrorWithType>(TaskResult::CachedTransactions(cached_count))
            }));
        }

        if !tx_cells.is_empty() && !self.uploader_config.disable_tx {
            let conn = self.connection.clone();
            let tx_table_name = self.uploader_config.tx_table_name.clone();
            let use_tx_compression = self.uploader_config.use_tx_compression.clone();
            let write_to_wal = self.uploader_config.hbase_write_to_wal.clone();
            debug!("HBase: spawning tx upload thread");
            tasks.push(tokio::spawn(async move {
                debug!("HBase: calling put_bincode_cells_with_retry for tx");
                conn.put_bincode_cells_with_retry::<TransactionInfo>(
                    tx_table_name.as_str(),
                    &tx_cells,
                    use_tx_compression,
                    write_to_wal,
                )
                .await
                .map(TaskResult::BytesWritten)
                .map_err(|e| TaskErrorWithType::new(TaskType::UploadTx, e))
            }));
        }

        if !tx_by_addr_cells.is_empty() && !self.uploader_config.disable_tx_by_addr {
            let conn = self.connection.clone();
            let tx_by_addr_table_name = self.uploader_config.tx_by_addr_table_name.clone();
            let use_tx_by_addr_compression =
                self.uploader_config.use_tx_by_addr_compression.clone();
            let write_to_wal = self.uploader_config.hbase_write_to_wal.clone();
            debug!("HBase: spawning tx-by-addr upload thread");
            tasks.push(tokio::spawn(async move {
                debug!("HBase: calling put_protobuf_cells_with_retry tx-by-addr");
                conn.put_protobuf_cells_with_retry::<tx_by_addr::TransactionByAddr>(
                    tx_by_addr_table_name.as_str(),
                    &tx_by_addr_cells,
                    use_tx_by_addr_compression,
                    write_to_wal,
                )
                .await
                .map(TaskResult::BytesWritten)
                .map_err(|e| TaskErrorWithType::new(TaskType::UploadTxByAddr, e))
            }));
        }

        // Entries upload
        if self.uploader_config.write_block_entries {
            if !entries.is_empty() {
                let conn = self.connection.clone();
                let entries_table_name = self.uploader_config.entries_table_name.clone();
                let use_entries_compression = true; // follow blocks/tx default compressed writes
                let write_to_wal = self.uploader_config.hbase_write_to_wal.clone();
                // Convert EntrySummary -> storage proto entries::Entries
                let entries_cell = (
                    slot_to_key(slot),
                    entries::Entries {
                        entries: entries.into_iter().enumerate().map(Into::into).collect(),
                    },
                );
                tasks.push(tokio::spawn(async move {
                    conn.put_protobuf_cells_with_retry::<entries::Entries>(
                        entries_table_name.as_str(),
                        &[entries_cell],
                        use_entries_compression,
                        write_to_wal,
                    )
                    .await
                    .map(TaskResult::BytesWritten)
                    .map_err(|e| TaskErrorWithType::new(TaskType::UploadEntries, e))
                }));
            }
        }

        let mut _bytes_written = 0;
        let mut total_cached_transactions = 0;
        let mut maybe_first_err: Option<Error> = None;

        debug!("HBase: waiting for all upload threads to finish...");

        let results = futures::future::join_all(tasks).await;
        debug!("HBase: got upload results");
        for result in results {
            match result {
                Err(err) => {
                    debug!("HBase: got error result {:?}", err);
                    if maybe_first_err.is_none() {
                        maybe_first_err = Some(Error::TokioJoinError(err));
                    }
                }
                Ok(Err(TaskErrorWithType { task_type, err })) => {
                    error!(
                        "HBase: got error result: type={:?}, error={:?}",
                        task_type, err
                    );
                    if maybe_first_err.is_none() {
                        match err {
                            TaskError::HBaseError(hbase_err) => {
                                maybe_first_err = Some(Error::HBaseError(hbase_err));
                            }
                            TaskError::MemcacheError(memcache_err) => {
                                maybe_first_err = Some(Error::MemcacheError(memcache_err));
                            }
                            TaskError::IoError(io_err) => {
                                maybe_first_err = Some(Error::IoError(io_err));
                            }
                            TaskError::EncodingError(enc_err) => {
                                maybe_first_err = Some(Error::EncodingError(enc_err));
                            }
                        }
                    }
                }
                Ok(Ok(task_result)) => match task_result {
                    TaskResult::BytesWritten(bytes) => _bytes_written += bytes,
                    TaskResult::CachedTransactions(count) => total_cached_transactions += count,
                },
            }
        }

        if let Some(err) = maybe_first_err {
            debug!("HBase: returning upload error result {:?}", err);
            return Err(err);
        }

        if self.enable_full_tx_cache {
            debug!(
                "Cached {} transactions from slot {}",
                slot, total_cached_transactions
            );
        }

        let _num_transactions = confirmed_block.transactions.len();

        // Store the block itself last, after all other metadata about the block has been
        // successfully stored.  This avoids partial uploaded blocks from becoming visible to
        // `get_confirmed_block()` and `get_confirmed_blocks()`
        let blocks_cells = [(
            slot_to_blocks_key(slot, self.uploader_config.use_md5_row_key_salt),
            confirmed_block.into(),
        )];

        debug!("HBase: calling put_protobuf_cells_with_retry for blocks");

        if !self.uploader_config.disable_blocks {
            _bytes_written += self
                .connection
                .put_protobuf_cells_with_retry::<generated::ConfirmedBlock>(
                    self.uploader_config.blocks_table_name.as_str(),
                    &blocks_cells,
                    self.uploader_config.use_blocks_compression,
                    self.uploader_config.hbase_write_to_wal,
                )
                .await
                .map_err(|err| {
                    error!("HBase: failed to upload block: {:?}", err);
                    err
                })?;
        }

        info!("HBase: successfully uploaded block from slot {}", slot);

        Ok(())
    }

    fn should_include_in_tx_full(&self, address: &Pubkey) -> bool {
        if let Some(ref filter) = self.uploader_config.tx_full_filter {
            if filter.exclude {
                // If exclude is true, exclude the address if it's in the set.
                !filter.addrs.contains(address)
            } else {
                // If exclude is false, include the address only if it's in the set.
                filter.addrs.contains(address)
            }
        } else {
            true
        }
    }

    fn should_include_in_tx_by_addr(&self, address: &Pubkey) -> bool {
        if let Some(ref filter) = self.uploader_config.tx_by_addr_filter {
            if filter.exclude {
                // If exclude is true, exclude the address if it's in the set.
                !filter.addrs.contains(address)
            } else {
                // If exclude is false, include the address only if it's in the set.
                filter.addrs.contains(address)
            }
        } else {
            true
        }
    }
}

pub async fn cache_transaction<T>(
    cache_client: &Client,
    signature: &str,
    transaction: T,
    tx_cache_expiration: Option<std::time::Duration>,
) -> std::result::Result<(), CacheWriteError>
where
    T: prost::Message,
{
    let mut buf = Vec::with_capacity(transaction.encoded_len());

    transaction
        .encode(&mut buf)
        .map_err(CacheWriteError::EncodingError)?;

    let compressed_tx = compress_best(&buf).map_err(CacheWriteError::IoError)?;

    let expiration = tx_cache_expiration
        .map(|d| d.as_secs().min(u32::MAX as u64) as u32)
        .unwrap_or(0);

    cache_client
        .set(signature, compressed_tx.as_slice(), expiration)
        .map_err(CacheWriteError::MemcacheError)?;

    Ok(())
}

fn get_account_keys(transaction_with_meta: &VersionedTransactionWithStatusMeta) -> Vec<Pubkey> {
    match &transaction_with_meta.transaction.message {
        VersionedMessage::V0(_) => {
            let static_keys = transaction_with_meta
                .transaction
                .message
                .static_account_keys();
            let LoadedAddresses { writable, readonly } =
                &transaction_with_meta.meta.loaded_addresses;

            static_keys
                .iter()
                .chain(writable.iter())
                .chain(readonly.iter())
                .cloned()
                .collect()
        }
        VersionedMessage::Legacy(_) => Vec::from(
            transaction_with_meta
                .transaction
                .message
                .static_account_keys(),
        ),
    }
}

fn is_error_tx(transaction_with_meta: &VersionedTransactionWithStatusMeta) -> bool {
    transaction_with_meta.meta.status.is_err()
}

fn is_voting_tx(transaction_with_meta: &VersionedTransactionWithStatusMeta) -> bool {
    let account_address = Pubkey::from_str("Vote111111111111111111111111111111111111111").unwrap();

    has_account(transaction_with_meta, &account_address)
}

fn has_account(
    transaction_with_meta: &VersionedTransactionWithStatusMeta,
    address: &Pubkey,
) -> bool {
    transaction_with_meta
        .transaction
        .message
        .static_account_keys()
        .contains(&address)
}

fn is_program_account(
    address: &Pubkey,
    transaction_with_meta: &VersionedTransactionWithStatusMeta,
    combined_keys: &[Pubkey],
) -> bool {
    // Helper to check if the address is used as a program account in a given instruction
    let check_program_id = |instruction: &CompiledInstruction, account_keys: &[Pubkey]| -> bool {
        let program_id = &account_keys[instruction.program_id_index as usize];
        program_id == address
    };

    // Check in outer instructions
    let used_in_outer = transaction_with_meta
        .transaction
        .message
        .instructions()
        .iter()
        .any(|instruction| check_program_id(instruction, combined_keys));

    // Check in inner instructions
    let used_in_inner = transaction_with_meta
        .meta
        .inner_instructions
        .as_ref()
        .map_or(false, |inner_instructions| {
            inner_instructions
                .iter()
                .flat_map(|inner| &inner.instructions)
                .any(|inner_instruction| {
                    check_program_id(&inner_instruction.instruction, combined_keys)
                })
        });

    used_in_outer || used_in_inner
}

fn is_readonly_account(
    address: &Pubkey,
    transaction_with_meta: &VersionedTransactionWithStatusMeta,
) -> bool {
    match &transaction_with_meta.transaction.message {
        VersionedMessage::V0(_) => {
            let static_keys = transaction_with_meta
                .transaction
                .message
                .static_account_keys();
            let LoadedAddresses { writable, readonly } =
                &transaction_with_meta.meta.loaded_addresses;

            // Check if the address is in the readonly list
            readonly.contains(address)
                || (static_keys.contains(address) && !writable.contains(address))
        }
        VersionedMessage::Legacy(_) => {
            // In legacy transactions, readonly accounts are determined based on the position in the list.
            let static_keys = transaction_with_meta
                .transaction
                .message
                .static_account_keys();
            let num_signers = transaction_with_meta
                .transaction
                .message
                .header()
                .num_required_signatures as usize;
            let num_readonly_signed = transaction_with_meta
                .transaction
                .message
                .header()
                .num_readonly_signed_accounts as usize;
            let num_readonly_unsigned = transaction_with_meta
                .transaction
                .message
                .header()
                .num_readonly_unsigned_accounts as usize;

            let readonly_start_signed = num_signers - num_readonly_signed;
            let readonly_start_unsigned = static_keys.len() - num_readonly_unsigned;

            static_keys.iter().enumerate().any(|(i, key)| {
                key == address && (i >= readonly_start_signed || i >= readonly_start_unsigned)
            })
        }
    }
}

pub(crate) fn convert_to_transaction_with_status_meta(
    item: VersionedTransactionWithStatusMeta,
) -> TransactionWithStatusMeta {
    TransactionWithStatusMeta::Complete(item)
}
