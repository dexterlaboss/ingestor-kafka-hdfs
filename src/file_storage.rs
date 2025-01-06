use {
    anyhow::{Context, Result},
    bytes::Bytes,
    futures::{Stream, TryStreamExt},
    std::{pin::Pin, sync::Arc},
    tokio::io::AsyncRead,
    tokio_util::io::StreamReader,
};

#[async_trait::async_trait]
pub trait FileStorage: Send + Sync {
    async fn list_directory(&self, dir_path: &str) -> Result<Vec<FileMetadata>>;
    async fn open_file(&self, file_path: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>>;
}

#[derive(Debug, Clone)]
pub struct FileMetadata {
    pub path: String,
    pub is_dir: bool,
}


pub struct HdfsStorage {
    client: Arc<hdfs_native::Client>,
}

impl HdfsStorage {
    pub fn new(client: hdfs_native::Client) -> Self {
        Self {
            client: Arc::new(client),
        }
    }
}

#[async_trait::async_trait]
impl FileStorage for HdfsStorage {
    async fn list_directory(&self, dir_path: &str) -> Result<Vec<FileMetadata>> {
        let entries = self
            .client
            .list_status(dir_path, false)
            .await
            .with_context(|| format!("Failed to list directory: {dir_path}"))?;

        let file_metadata = entries
            .into_iter()
            .map(|entry| FileMetadata {
                path: entry.path,
                is_dir: entry.isdir,
            })
            .collect();
        Ok(file_metadata)
    }

    async fn open_file(&self, file_path: &str) -> Result<Box<dyn AsyncRead + Unpin + Send>> {
        let file = self
            .client
            .read(file_path)
            .await
            .with_context(|| format!("Failed to open file '{file_path}'"))?;

        let stream = file_reader_stream(file);
        let async_reader = StreamReader::new(stream.map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::Other, e.to_string())
        }));

        Ok(Box::new(async_reader))
    }
}

/// A helper function for HDFS to create an asynchronous stream of `Bytes`.
fn file_reader_stream(
    reader: hdfs_native::file::FileReader,
) -> Pin<Box<impl Stream<Item = Result<Bytes, hdfs_native::HdfsError>>>> {
    const BUFFER_SIZE: usize = 8 * 1024 * 1024;
    Box::pin(futures::stream::try_unfold(reader, |mut rdr| async {
        match rdr.read(BUFFER_SIZE).await {
            Ok(bytes) if bytes.is_empty() => Ok(None),
            Ok(bytes) => Ok(Some((bytes, rdr))),
            Err(e) => Err(e),
        }
    }))
}
