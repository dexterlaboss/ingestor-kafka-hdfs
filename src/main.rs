use {
    crate::{
        block_processor::BlockProcessor,
        cli::block_uploader_app,
        config::Config,
        decompressor::{Decompressor, GzipDecompressor},
        file_processor::FileProcessor,
        file_storage::HdfsStorage,
        format_parser::{FormatParser, NdJsonParser},
        ingestor::Ingestor,
        ledger_storage::{
            FilterTxIncludeExclude, LedgerCacheConfig, LedgerStorage, LedgerStorageConfig, UploaderConfig,
        },
        message_decoder::{JsonMessageDecoder, MessageDecoder},
        queue_consumer::{KafkaConfig, KafkaQueueConsumer, QueueConsumer},
        queue_producer::KafkaQueueProducer,
    },
    std::{collections::HashSet, sync::Arc},
    anyhow::{Context, Result},
    clap::{value_t_or_exit, values_t, ArgMatches},
    log::info,
    solana_sdk::pubkey::Pubkey,
    hdfs_native::Client,
    rdkafka::config::RDKafkaLogLevel,
};

mod block_processor;
mod cli;
mod config;
mod decompressor;
mod file_storage;
mod format_parser;
mod hbase;
mod ingestor;
mod ledger_storage;
mod message_decoder;
mod file_processor;
mod queue_consumer;
mod queue_producer;
mod record_stream;

const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI arguments
    let cli_app = block_uploader_app(SERVICE_VERSION);
    let matches = cli_app.get_matches();

    // Initialize logging
    env_logger::init();
    info!("Starting the Solana block ingestor (Version: {})", SERVICE_VERSION);

    // Process CLI arguments
    let uploader_config = process_uploader_arguments(&matches);
    let cache_config = process_cache_arguments(&matches);

    // Load configuration
    let config = Arc::new(Config::new());

    // Create HDFS client
    let hdfs_client = Client::new(&config.hdfs_url).context("Failed to create HDFS client")?;
    let file_storage = HdfsStorage::new(hdfs_client);

    // Create components
    let decompressor: Box<dyn Decompressor + Send + Sync> = Box::new(GzipDecompressor {});
    let message_decoder: Arc<dyn MessageDecoder + Send + Sync> = Arc::new(JsonMessageDecoder {});
    let format_parser: Arc<dyn FormatParser + Send + Sync> = Arc::new(NdJsonParser {});

    // Create ledger storage
    let ledger_storage_config = LedgerStorageConfig {
        address: config.hbase_address.clone(),
        namespace: config.namespace.clone(),
        uploader_config: uploader_config.clone(),
        cache_config: cache_config.clone(),
    };
    let ledger_storage = LedgerStorage::new_with_config(ledger_storage_config).await;

    // Create the block processor
    let block_processor = BlockProcessor::new(ledger_storage.clone());

    let file_processor = Arc::new(FileProcessor::new(
        file_storage,
        format_parser.clone(),
        block_processor,
        decompressor,
    ));

    let kafka_config = KafkaConfig {
        group_id: config.kafka_group_id.clone(),
        bootstrap_servers: config.kafka_brokers.clone(),
        enable_partition_eof: false,
        session_timeout_ms: 10000,
        enable_auto_commit: true,
        auto_offset_reset: "earliest".to_string(),
        max_partition_fetch_bytes: 10 * 1024 * 1024, // 10 MiB
        max_in_flight_requests_per_connection: 1,
        log_level: RDKafkaLogLevel::Debug,
    };

    // Create the queue consumer
    let consumer: Box<dyn QueueConsumer + Send + Sync> = Box::new(
        KafkaQueueConsumer::new(kafka_config, &[&config.kafka_consume_topic])
            .unwrap()
    );

    // Create the queue producer
    let kafka_producer = KafkaQueueProducer::new(
        &config.kafka_brokers,
        &config.kafka_produce_error_topic
    )?;

    // Create the ingestor
    let mut ingestor = Ingestor::new(
        consumer,
        kafka_producer,
        file_processor,
        message_decoder,
    );

    // Run the Ingestor
    ingestor.run().await?;

    Ok(())
}

/// Process uploader-related CLI arguments
fn process_uploader_arguments(matches: &ArgMatches) -> UploaderConfig {
    let disable_tx = matches.is_present("disable_tx");
    let disable_tx_by_addr = matches.is_present("disable_tx_by_addr");
    let disable_blocks = matches.is_present("disable_blocks");
    let enable_full_tx = matches.is_present("enable_full_tx");
    let use_md5_row_key_salt = matches.is_present("use_md5_row_key_salt");
    let filter_program_accounts = matches.is_present("filter_tx_by_addr_programs");
    let filter_voting_tx = matches.is_present("filter_voting_tx");
    let filter_error_tx = matches.is_present("filter_error_tx");
    let use_blocks_compression = !matches.is_present("disable_block_compression");
    let use_tx_compression = !matches.is_present("disable_tx_compression");
    let use_tx_by_addr_compression = !matches.is_present("disable_tx_by_addr_compression");
    let use_tx_full_compression = !matches.is_present("disable_tx_full_compression");
    let hbase_write_to_wal = !matches.is_present("hbase_skip_wal");

    let filter_tx_full_include_addrs: HashSet<Pubkey> =
        values_t!(matches, "filter_tx_full_include_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let filter_tx_full_exclude_addrs: HashSet<Pubkey> =
        values_t!(matches, "filter_tx_full_exclude_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let filter_tx_by_addr_include_addrs: HashSet<Pubkey> =
        values_t!(matches, "filter_tx_by_addr_include_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let filter_tx_by_addr_exclude_addrs: HashSet<Pubkey> =
        values_t!(matches, "filter_tx_by_addr_exclude_addr", Pubkey)
            .unwrap_or_default()
            .iter()
            .cloned()
            .collect();

    let tx_full_filter = create_filter(filter_tx_full_exclude_addrs, filter_tx_full_include_addrs);
    let tx_by_addr_filter = create_filter(filter_tx_by_addr_exclude_addrs, filter_tx_by_addr_include_addrs);

    UploaderConfig {
        tx_full_filter,
        tx_by_addr_filter,
        disable_tx,
        disable_tx_by_addr,
        disable_blocks,
        enable_full_tx,
        use_md5_row_key_salt,
        filter_program_accounts,
        filter_voting_tx,
        filter_error_tx,
        use_blocks_compression,
        use_tx_compression,
        use_tx_by_addr_compression,
        use_tx_full_compression,
        hbase_write_to_wal,
        ..Default::default()
    }
}

/// Process cache-related CLI arguments
fn process_cache_arguments(matches: &ArgMatches) -> LedgerCacheConfig {
    let enable_full_tx_cache = matches.is_present("enable_full_tx_cache");

    let address = if matches.is_present("cache_address") {
        value_t_or_exit!(matches, "cache_address", String)
    } else {
        String::new()
    };

    let timeout = if matches.is_present("cache_timeout") {
        Some(std::time::Duration::from_secs(
            value_t_or_exit!(matches, "cache_timeout", u64),
        ))
    } else {
        None
    };

    let tx_cache_expiration = if matches.is_present("tx_cache_expiration") {
        Some(std::time::Duration::from_secs(
            value_t_or_exit!(matches, "tx_cache_expiration", u64) * 24 * 60 * 60,
        ))
    } else {
        None
    };

    LedgerCacheConfig {
        enable_full_tx_cache,
        address,
        timeout,
        tx_cache_expiration,
        ..Default::default()
    }
}

/// Helper function to create a filter
fn create_filter(
    filter_tx_exclude_addrs: HashSet<Pubkey>,
    filter_tx_include_addrs: HashSet<Pubkey>,
) -> Option<FilterTxIncludeExclude> {
    let exclude_tx_addrs = !filter_tx_exclude_addrs.is_empty();
    let include_tx_addrs = !filter_tx_include_addrs.is_empty();

    if exclude_tx_addrs || include_tx_addrs {
        let filter_tx_addrs = FilterTxIncludeExclude {
            exclude: exclude_tx_addrs,
            addrs: if exclude_tx_addrs {
                filter_tx_exclude_addrs
            } else {
                filter_tx_include_addrs
            },
        };
        Some(filter_tx_addrs)
    } else {
        None
    }
}