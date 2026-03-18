//! Compression utilities for blob storage.
//! Raw HTML gets gzip-compressed before storage — typically 70-90% size reduction.

use std::io::{Read, Write};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tracing::debug;

use crate::error::{CacheError, CacheResult};

/// Compress bytes with gzip.
pub fn compress(data: &[u8]) -> CacheResult<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(data)
        .map_err(|e| CacheError::CompressionError(format!("Compress write failed: {}", e)))?;
    let compressed = encoder.finish()
        .map_err(|e| CacheError::CompressionError(format!("Compress finish failed: {}", e)))?;

    debug!(
        original = data.len(),
        compressed = compressed.len(),
        ratio = format!("{:.1}%", compressed.len() as f64 / data.len().max(1) as f64 * 100.0),
        "Compressed data"
    );

    Ok(compressed)
}

/// Decompress gzip bytes.
pub fn decompress(data: &[u8]) -> CacheResult<Vec<u8>> {
    let mut decoder = GzDecoder::new(data);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)
        .map_err(|e| CacheError::CompressionError(format!("Decompress failed: {}", e)))?;

    debug!(
        compressed = data.len(),
        decompressed = decompressed.len(),
        "Decompressed data"
    );

    Ok(decompressed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip() {
        let original = b"<html><body><h1>Hello World</h1><p>Some content here that should compress well because HTML has lots of repeated patterns and tags.</p></body></html>";
        let compressed = compress(original).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(original.as_slice(), decompressed.as_slice());
        assert!(compressed.len() < original.len());
    }
}
