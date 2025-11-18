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
    info!("Starting the Solana block ingestor (stdin) (Version: {})", SERVICE_VERSION);

    if matches.is_present("add_empty_tx_metadata_if_missing") {
        std::env::set_var("ADD_EMPTY_TX_METADATA_IF_MISSING", "1");
    }

    let uploader_config = process_uploader_arguments(&matches);
    let cache_config = process_cache_arguments(&matches);
    let validate_only = matches.is_present("validate_only");

    let config = Arc::new(Config::new());

    let decoder: std::sync::Arc<dyn MessageDecoder + Send + Sync> = std::sync::Arc::new(JsonMessageDecoder {});

    // Read NDJSON lines from stdin and either validate-only or process normally
    let reader = BufReader::new(tokio::io::stdin());
    let mut lines = reader.lines();

    if validate_only {
        while let Some(line) = lines.next_line().await? {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match decoder.decode(trimmed.as_bytes()).await {
                Ok(decoded) => {
                    match decoded {
                        ingestor_kafka_hdfs::message_decoder::DecodedPayload::Block(block_id, _) => {
                            eprintln!("Parsed block (no entries): blockID={}", block_id);
                        }
                        ingestor_kafka_hdfs::message_decoder::DecodedPayload::BlockWithEntries(block_id, _, _) => {
                            eprintln!("Parsed block with entries: blockID={}", block_id);
                        }
                        ingestor_kafka_hdfs::message_decoder::DecodedPayload::FilePath(path) => {
                            eprintln!("Parsed file path payload (unexpected in validate-only): {}", path);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Failed to decode input: {}", e);
                    for cause in e.chain().skip(1) {
                        eprintln!("  caused by: {}", cause);
                    }
                }
            }
        }
        return Ok(());
    }

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

    while let Some(line) = lines.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match decoder.decode(trimmed.as_bytes()).await {
            Ok(decoded) => {
                if let Err(e) = processor.process_decoded(decoded).await {
                    eprintln!("Error processing input: {:#?}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to decode input: {}", e);
                for cause in e.chain().skip(1) {
                    eprintln!("  caused by: {}", cause);
                }
            }
        }
    }

    Ok(())
}


