use {
    anyhow::{Context, Result},
    hdfs_native::Client,
    ingestor_kafka_hdfs::{
        block_processor::BlockProcessor,
        cli::{block_uploader_app, process_cache_arguments, process_uploader_arguments},
        config::Config,
        decompressor::{Decompressor, GzipDecompressor},
        file_processor::{FileProcessor, Processor},
        file_storage::HdfsStorage,
        format_parser::{FormatParser, NdJsonParser},
        ledger_storage::{LedgerStorage, LedgerStorageConfig},
        message_decoder::{JsonMessageDecoder, MessageDecoder},
    },
    log::info,
    std::sync::Arc,
    tokio::io::{AsyncBufReadExt, BufReader},
};

const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    let cli_app = block_uploader_app(SERVICE_VERSION);
    let matches = cli_app.get_matches();

    env_logger::init();
    info!(
        "Starting the Solana block ingestor (stdin) (Version: {})",
        SERVICE_VERSION
    );

    let uploader_config = process_uploader_arguments(&matches);
    let cache_config = process_cache_arguments(&matches);

    let config = Arc::new(Config::new());

    let hdfs_client = Client::new(&config.hdfs_url).context("Failed to create HDFS client")?;
    let file_storage = HdfsStorage::new(hdfs_client);

    let format_parser: Arc<dyn FormatParser + Send + Sync> = Arc::new(NdJsonParser {});
    let decompressor: Box<dyn Decompressor + Send + Sync> = Box::new(GzipDecompressor {});

    let ledger_storage_config = LedgerStorageConfig {
        address: config.hbase_address.clone(),
        namespace: config.namespace.clone(),
        uploader_config: uploader_config.clone(),
        cache_config: cache_config.clone(),
    };
    let ledger_storage = LedgerStorage::new_with_config(ledger_storage_config).await;

    let block_processor = BlockProcessor::new(ledger_storage.clone());
    let processor = FileProcessor::new(
        file_storage,
        format_parser.clone(),
        block_processor,
        decompressor,
    );
    let decoder: std::sync::Arc<dyn MessageDecoder + Send + Sync> =
        std::sync::Arc::new(JsonMessageDecoder {});

    // Read NDJSON lines from stdin and feed through the same parser/processor pipeline
    let reader = BufReader::new(tokio::io::stdin());
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match decoder.decode(trimmed.as_bytes()).await {
            Ok(decoded) => {
                if let Err(e) = processor.process_decoded(decoded).await {
                    eprintln!("Error processing input: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to decode input: {}", e);
            }
        }
    }

    Ok(())
}
