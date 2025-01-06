use {
    anyhow::Result,
    async_trait::async_trait,
    tokio::io::{AsyncBufReadExt, AsyncRead, BufReader},
};

#[async_trait]
pub trait RecordStream: Send + Unpin {
    /// Returns the next record as a `String`.
    /// `None` if end of file (EOF).
    async fn next_record(&mut self) -> Option<Result<String>>;
}

/// A line-based NDJSON record stream.
/// It just reads lines and returns them as strings.
pub struct NdJsonRecordStream {
    reader: BufReader<Box<dyn AsyncRead + Unpin + Send>>,
}

impl NdJsonRecordStream {
    pub fn new(input: Box<dyn AsyncRead + Unpin + Send>) -> Self {
        Self {
            reader: BufReader::new(input),
        }
    }
}

#[async_trait]
impl RecordStream for NdJsonRecordStream {
    async fn next_record(&mut self) -> Option<Result<String>> {
        let mut line = String::new();
        match self.reader.read_line(&mut line).await {
            Ok(0) => None, // EOF
            Ok(_) => Some(Ok(line)),
            Err(e) => Some(Err(e.into())),
        }
    }
}
