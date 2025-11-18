use {
    anyhow::{Context, Result},
    clap::ArgMatches,
    hdfs_native::Client,
    ingestor_kafka_hdfs::{
        block_processor::BlockProcessor,
        cli::{block_uploader_app, process_cache_arguments, process_uploader_arguments},
        config::Config,
        decompressor::{Decompressor, GzipDecompressor},
        file_processor::FileProcessor,
        file_storage::HdfsStorage,
        format_parser::{FormatParser, NdJsonParser},
        ingestor::Ingestor,
        ledger_storage::{LedgerCacheConfig, LedgerStorage, LedgerStorageConfig, UploaderConfig},
        message_decoder::{JsonMessageDecoder, MessageDecoder},
        queue_consumer::{KafkaConfig, KafkaQueueConsumer, QueueConsumer},
        queue_producer::KafkaQueueProducer,
    },
    log::info,
    rdkafka::config::RDKafkaLogLevel,
    std::sync::Arc,
};

const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    let cli_app = block_uploader_app(SERVICE_VERSION);
    let matches = cli_app.get_matches();

    env_logger::init();
    info!("Starting the Solana block ingestor (Version: {})", SERVICE_VERSION);

    if matches.is_present("add_empty_tx_metadata_if_missing") {
        std::env::set_var("ADD_EMPTY_TX_METADATA_IF_MISSING", "1");
    }

    let uploader_config = process_uploader_arguments(&matches);
    let cache_config = process_cache_arguments(&matches);

    let config = Arc::new(Config::new());

    let hdfs_client = Client::new(&config.hdfs_url).context("Failed to create HDFS client")?;
    let file_storage = HdfsStorage::new(hdfs_client);

    let decompressor: Box<dyn Decompressor + Send + Sync> = Box::new(GzipDecompressor {});
    let message_decoder: Arc<dyn MessageDecoder + Send + Sync> = Arc::new(JsonMessageDecoder {});
    let format_parser: Arc<dyn FormatParser + Send + Sync> = Arc::new(NdJsonParser {});

    let ledger_storage_config = LedgerStorageConfig {
        address: config.hbase_address.clone(),
        namespace: config.namespace.clone(),
        uploader_config: uploader_config.clone(),
        cache_config: cache_config.clone(),
    };
    let ledger_storage = LedgerStorage::new_with_config(ledger_storage_config).await;

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
        max_partition_fetch_bytes: 10 * 1024 * 1024,
        max_in_flight_requests_per_connection: 1,
        log_level: RDKafkaLogLevel::Debug,
    };

    let consumer: Box<dyn QueueConsumer + Send + Sync> = Box::new(
        KafkaQueueConsumer::new(kafka_config, &[&config.kafka_consume_topic]).unwrap(),
    );

    let kafka_producer = KafkaQueueProducer::new(
        &config.kafka_brokers,
        &config.kafka_produce_error_topic,
    )?;

    let mut ingestor = Ingestor::new(
        consumer,
        kafka_producer,
        file_processor,
        message_decoder,
    );

    ingestor.run().await?;

    Ok(())
}


