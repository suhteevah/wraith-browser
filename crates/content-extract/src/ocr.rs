//! # OCR Interface
//!
//! Extract text from images (screenshots, PDFs rendered as images).
//! Defines a trait-based backend system so different OCR engines can be
//! swapped in without changing consumer code.
//!
//! ## Architecture
//!
//! ```text
//! Image (PNG/JPEG) ──► OcrBackend ──► OcrResult
//!                         │
//!                         ├── MockOcrBackend    (testing)
//!                         ├── PaddleOcrBackend  (needs ONNX runtime)
//!                         └── basic_image_text_detection (placeholder)
//! ```
//!
//! ## Status
//!
//! This module defines the interface and a mock backend. Real OCR requires
//! `oar-ocr` + ONNX Runtime — add via `--features ocr`.

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn, instrument};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of OCR processing on an image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrResult {
    /// Full extracted text from the image.
    pub text: String,
    /// Overall confidence score (0.0 - 1.0).
    pub confidence: f64,
    /// Detected text regions with bounding boxes.
    pub regions: Vec<OcrRegion>,
    /// Detected or assumed language (ISO 639-1).
    pub language: String,
    /// Processing time in milliseconds.
    pub duration_ms: u64,
}

/// A detected text region within an image.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrRegion {
    /// Text content within this region.
    pub text: String,
    /// Bounding box X coordinate (pixels from left).
    pub x: f64,
    /// Bounding box Y coordinate (pixels from top).
    pub y: f64,
    /// Bounding box width in pixels.
    pub width: f64,
    /// Bounding box height in pixels.
    pub height: f64,
    /// Confidence score for this region (0.0 - 1.0).
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// Backend trait
// ---------------------------------------------------------------------------

/// Trait for pluggable OCR engine backends.
///
/// Implement this to integrate different OCR models or services.
/// All backends must be `Send + Sync` for use in async contexts.
pub trait OcrBackend: Send + Sync {
    /// Extract text from raw image bytes (PNG, JPEG, etc.).
    fn extract_text(&self, image_data: &[u8]) -> Result<OcrResult, String>;

    /// Human-readable name of this backend.
    fn name(&self) -> &str;

    /// Whether this backend is available (models loaded, runtime present, etc.).
    fn is_available(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Mock backend
// ---------------------------------------------------------------------------

/// Mock OCR backend that returns synthetic results for testing.
pub struct MockOcrBackend;

impl OcrBackend for MockOcrBackend {
    #[instrument(skip(self, image_data), fields(backend = "MockOcr", data_len = image_data.len()))]
    fn extract_text(&self, image_data: &[u8]) -> Result<OcrResult, String> {
        info!(bytes = image_data.len(), "MockOcrBackend: generating synthetic OCR result");

        Ok(OcrResult {
            text: "Mock OCR extracted text: Lorem ipsum dolor sit amet".to_string(),
            confidence: 0.95,
            regions: vec![
                OcrRegion {
                    text: "Lorem ipsum".to_string(),
                    x: 10.0,
                    y: 20.0,
                    width: 200.0,
                    height: 30.0,
                    confidence: 0.97,
                },
                OcrRegion {
                    text: "dolor sit amet".to_string(),
                    x: 10.0,
                    y: 60.0,
                    width: 250.0,
                    height: 30.0,
                    confidence: 0.93,
                },
            ],
            language: "en".to_string(),
            duration_ms: 0,
        })
    }

    fn name(&self) -> &str {
        "MockOcr"
    }

    fn is_available(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// PaddleOCR backend (stub)
// ---------------------------------------------------------------------------

/// PaddleOCR backend using ONNX Runtime for inference.
///
/// Requires `--features ocr` and ONNX model files at `~/.openclaw/models/paddleocr/`.
/// Without the feature flag, this backend reports as unavailable.
pub struct PaddleOcrBackend {
    _model_dir: String,
}

impl PaddleOcrBackend {
    /// Create a new PaddleOCR backend pointing at the given model directory.
    pub fn new(model_dir: &str) -> Self {
        Self {
            _model_dir: model_dir.to_string(),
        }
    }
}

impl OcrBackend for PaddleOcrBackend {
    #[instrument(skip(self, image_data), fields(backend = "PaddleOcr", data_len = image_data.len()))]
    fn extract_text(&self, image_data: &[u8]) -> Result<OcrResult, String> {
        warn!(
            bytes = image_data.len(),
            "PaddleOCR backend not compiled — enable --features ocr"
        );
        Ok(OcrResult {
            text: String::new(),
            confidence: 0.0,
            regions: Vec::new(),
            language: "unknown".to_string(),
            duration_ms: 0,
        })
    }

    fn name(&self) -> &str {
        "PaddleOcr"
    }

    fn is_available(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// Basic image text detection (placeholder)
// ---------------------------------------------------------------------------

/// Extremely basic image text detection placeholder.
///
/// Checks whether the provided data looks like a valid PNG image and returns
/// an empty `OcrResult`. This is a stub for the real OCR pipeline; it does
/// not perform actual text recognition.
#[instrument(skip(image_data), fields(data_len = image_data.len()))]
pub fn basic_image_text_detection(image_data: &[u8]) -> OcrResult {
    // PNG magic bytes: 0x89 P N G 0x0D 0x0A 0x1A 0x0A
    let is_png = image_data.len() >= 8
        && image_data[0] == 0x89
        && image_data[1] == b'P'
        && image_data[2] == b'N'
        && image_data[3] == b'G';

    let format = if is_png { "PNG" } else { "unknown" };
    debug!(format, bytes = image_data.len(), "Basic image detection");

    OcrResult {
        text: String::new(),
        confidence: 0.0,
        regions: Vec::new(),
        language: "unknown".to_string(),
        duration_ms: 0,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_backend_returns_results() {
        let backend = MockOcrBackend;
        let result = backend.extract_text(&[0x89, b'P', b'N', b'G']).unwrap();

        assert!(!result.text.is_empty());
        assert!(result.confidence > 0.0);
        assert!(!result.regions.is_empty());
        assert_eq!(result.language, "en");
    }

    #[test]
    fn mock_backend_is_available() {
        let backend = MockOcrBackend;
        assert!(backend.is_available());
        assert_eq!(backend.name(), "MockOcr");
    }

    #[test]
    fn mock_backend_regions_have_bboxes() {
        let backend = MockOcrBackend;
        let result = backend.extract_text(&[]).unwrap();

        for region in &result.regions {
            assert!(region.width > 0.0);
            assert!(region.height > 0.0);
            assert!(region.confidence > 0.0);
            assert!(!region.text.is_empty());
        }
    }

    #[test]
    fn paddle_backend_not_available() {
        let backend = PaddleOcrBackend::new("/tmp/models/paddleocr");
        assert!(!backend.is_available());
        assert_eq!(backend.name(), "PaddleOcr");
    }

    #[test]
    fn paddle_backend_returns_empty() {
        let backend = PaddleOcrBackend::new("/tmp/models/paddleocr");
        let result = backend.extract_text(&[1, 2, 3]).unwrap();
        assert!(result.text.is_empty());
        assert_eq!(result.confidence, 0.0);
        assert!(result.regions.is_empty());
    }

    #[test]
    fn basic_detection_with_png() {
        // Minimal PNG-like header
        let png_header: Vec<u8> = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        let result = basic_image_text_detection(&png_header);

        assert!(result.text.is_empty());
        assert_eq!(result.confidence, 0.0);
        assert!(result.regions.is_empty());
        assert_eq!(result.language, "unknown");
    }

    #[test]
    fn basic_detection_with_non_png() {
        let data = b"not an image at all";
        let result = basic_image_text_detection(data);

        assert!(result.text.is_empty());
        assert_eq!(result.duration_ms, 0);
    }

    #[test]
    fn basic_detection_returns_valid_struct() {
        let result = basic_image_text_detection(&[]);
        // Ensure all fields are populated with valid defaults
        assert_eq!(result.text, "");
        assert_eq!(result.confidence, 0.0);
        assert!(result.regions.is_empty());
        assert_eq!(result.language, "unknown");
        assert_eq!(result.duration_ms, 0);
    }

    #[test]
    fn ocr_result_serialization() {
        let result = OcrResult {
            text: "Hello".to_string(),
            confidence: 0.99,
            regions: vec![OcrRegion {
                text: "Hello".to_string(),
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 20.0,
                confidence: 0.99,
            }],
            language: "en".to_string(),
            duration_ms: 42,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("Hello"));
        assert!(json.contains("0.99"));

        let deserialized: OcrResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.text, "Hello");
        assert_eq!(deserialized.regions.len(), 1);
    }
}
