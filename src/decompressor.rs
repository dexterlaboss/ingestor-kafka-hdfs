use {
    async_compression::tokio::bufread::GzipDecoder,
    tokio::io::{AsyncRead, BufReader},
};

pub trait Decompressor: Send + Sync {
    fn decompress(
        &self,
        input: Box<dyn AsyncRead + Unpin + Send>,
    ) -> Box<dyn AsyncRead + Unpin + Send>;
}

pub struct GzipDecompressor;

impl Decompressor for GzipDecompressor {
    fn decompress(
        &self,
        input: Box<dyn AsyncRead + Unpin + Send>,
    ) -> Box<dyn AsyncRead + Unpin + Send> {
        // Wrap the input in a BufReader, as `GzipDecoder` expects an `AsyncBufRead`
        let bufreader = BufReader::new(input);
        // Use GzipDecoder with the buffered reader
        Box::new(GzipDecoder::new(bufreader))
    }
}

pub struct NoOpDecompressor;

impl Decompressor for NoOpDecompressor {
    fn decompress(
        &self,
        input: Box<dyn AsyncRead + Unpin + Send>,
    ) -> Box<dyn AsyncRead + Unpin + Send> {
        input
    }
}
