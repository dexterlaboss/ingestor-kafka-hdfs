use {
    crate::{
        block_processor::BlockProcessor,
        decompressor::Decompressor,
        file_storage::FileStorage,
        format_parser::FormatParser,
        message_decoder::DecodedPayload,
        record_stream::{NdJsonRecordStream, RecordStream},
    },
    anyhow::{Context, Result},
    log::{error, info},
    std::{sync::Arc, time::Instant},
};

#[async_trait::async_trait]
pub trait Processor {
    /// Process a single decoded payload.
    async fn process_decoded(&self, decoded: DecodedPayload) -> Result<()>;
}

/// High-level processor that ties storage, decompression, parsing, and block uploading logic together.
pub struct FileProcessor<S> {
    storage: S,
    parser: Arc<dyn FormatParser + Send + Sync>, // Updated to use trait object
    block_processor: BlockProcessor,
    decompressor: Box<dyn Decompressor + Send + Sync>, // Boxed for dynamic dispatch
}

#[async_trait::async_trait]
impl<S> Processor for FileProcessor<S>
where
    S: FileStorage + Send + Sync,
{
    async fn process_decoded(&self, decoded: DecodedPayload) -> Result<()> {
        match decoded {
            DecodedPayload::FilePath(path) => self.process_file(&path).await,
            DecodedPayload::Block(block_id, block) => {
                self.block_processor.handle_block(block_id, block).await
            }
        }
    }
}

impl<S> FileProcessor<S>
where
    S: FileStorage + Send + Sync,
{
    pub fn new(
        storage: S,
        parser: Arc<dyn FormatParser + Send + Sync>, // Fixed number of arguments
        block_processor: BlockProcessor,
        decompressor: Box<dyn Decompressor + Send + Sync>,
    ) -> Self {
        Self {
            storage,
            parser,
            block_processor,
            decompressor,
        }
    }

    /// Process all files in a directory.
    #[allow(unused)]
    pub async fn process_directory(&self, dir_path: &str) -> Result<()> {
        let entries = self
            .storage
            .list_directory(dir_path)
            .await
            .with_context(|| format!("Failed to list directory '{dir_path}'"))?;

        info!("Processing files in directory '{}':", dir_path);
        for entry in entries {
            if !entry.is_dir {
                if let Err(e) = self.process_file(&entry.path).await {
                    error!("Error processing file '{}': {}", entry.path, e);
                }
            }
        }

        Ok(())
    }

    /// Process a single file:
    ///  1. Open it from storage
    ///  2. Decompress (if needed)
    ///  3. Read lines from the record stream
    ///  4. Parse each line into a block
    ///  5. Pass each block to the BlockProcessor
    pub async fn process_file(&self, file_path: &str) -> Result<()> {
        info!("Reading file: {file_path}");
        let start_time = Instant::now();

        // Open a file
        let raw_file = self.storage.open_file(file_path).await?;

        // Decompress
        let decompressed_reader = self.decompressor.decompress(raw_file);

        // Create a line-based NDJSON record stream
        let mut record_stream = NdJsonRecordStream::new(decompressed_reader);

        // Read + parse lines
        while let Some(line_result) = record_stream.next_record().await {
            match line_result {
                Ok(line) => self.process_line(&line).await,
                Err(e) => error!("Error reading line: {e}"),
            }
        }

        let duration = start_time.elapsed();
        info!(
            "Finished processing file '{file_path}'. Total time: {} ms",
            duration.as_millis()
        );
        Ok(())
    }

    /// Process a single line from the record stream.
    pub async fn process_line(&self, line: &str) {
        match self.parser.parse_record(line) {
            Ok(Some((block_id, block))) => {
                if let Err(err) = self.block_processor.handle_block(block_id, block).await {
                    error!("Error handling block: {err}");
                }
            }
            Ok(None) => {
                // empty line or no blockID
            }
            Err(e) => {
                error!("Failed to parse record: {e}");
            }
        }
    }
}