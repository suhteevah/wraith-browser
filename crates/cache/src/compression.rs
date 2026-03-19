//! Compression utilities for blob storage.
//!
//! Uses zstd for new data (3-5x faster decompression than gzip at similar ratios).
//! Falls back to gzip decompression for existing blobs (backward compatibility).

use std::io::Read;
use tracing::debug;

use crate::error::{CacheError, CacheResult};

/// Magic bytes: zstd frame starts with 0xFD2FB528
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Compress bytes with zstd (level 3 — fast, good ratio).
pub fn compress(data: &[u8]) -> CacheResult<Vec<u8>> {
    let compressed = zstd::encode_all(data, 3)
        .map_err(|e| CacheError::CompressionError(format!("zstd compress failed: {e}")))?;

    debug!(
        original = data.len(),
        compressed = compressed.len(),
        ratio = format!("{:.1}%", compressed.len() as f64 / data.len().max(1) as f64 * 100.0),
        "Compressed data (zstd)"
    );

    Ok(compressed)
}

/// Decompress bytes. Auto-detects zstd vs gzip for backward compatibility.
pub fn decompress(data: &[u8]) -> CacheResult<Vec<u8>> {
    if data.len() >= 4 && data[..4] == ZSTD_MAGIC {
        // zstd format
        let decompressed = zstd::decode_all(data)
            .map_err(|e| CacheError::CompressionError(format!("zstd decompress failed: {e}")))?;

        debug!(
            compressed = data.len(),
            decompressed = decompressed.len(),
            "Decompressed data (zstd)"
        );
        Ok(decompressed)
    } else {
        // Legacy gzip format (existing blobs)
        use flate2::read::GzDecoder;
        let mut decoder = GzDecoder::new(data);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed)
            .map_err(|e| CacheError::CompressionError(format!("gzip decompress failed: {e}")))?;

        debug!(
            compressed = data.len(),
            decompressed = decompressed.len(),
            "Decompressed data (gzip legacy)"
        );
        Ok(decompressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zstd_roundtrip() {
        let original = b"<html><body><h1>Hello World</h1><p>Some content here that should compress well because HTML has lots of repeated patterns and tags.</p></body></html>";
        let compressed = compress(original).unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(original.as_slice(), decompressed.as_slice());
        assert!(compressed.len() < original.len());
    }

    #[test]
    fn test_zstd_magic_detected() {
        let original = b"test data for zstd";
        let compressed = compress(original).unwrap();
        // zstd magic bytes should be present
        assert_eq!(compressed[..4], ZSTD_MAGIC);
    }

    #[test]
    fn test_gzip_backward_compat() {
        // Create gzip data the old way
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let original = b"legacy gzip compressed data";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(original).unwrap();
        let gzip_data = encoder.finish().unwrap();

        // decompress should handle it via gzip fallback
        let decompressed = decompress(&gzip_data).unwrap();
        assert_eq!(original.as_slice(), decompressed.as_slice());
    }

    #[test]
    fn test_empty_data() {
        let compressed = compress(b"").unwrap();
        let decompressed = decompress(&compressed).unwrap();
        assert!(decompressed.is_empty());
    }
}
