//! # Vision-Based Grounding (Mode 3)
//!
//! Provides UI understanding for pages that can't be parsed via DOM alone:
//! canvas-based apps, PDFs rendered in-browser, Flash-like content, and
//! complex custom widgets. Uses local ML models for sub-second inference.
//!
//! ## Architecture
//!
//! ```text
//! Screenshot (PNG) ──► VisionBackend ──► Vec<UiElement>
//!                      │
//!                      ├── OmniParser (YOLOv8 + Florence-2, ~0.6s)
//!                      ├── Moondream (0.5B, sub-second on CPU)
//!                      ├── ShowUI (2B, 75.1% ScreenSpot accuracy)
//!                      └── MockVision (for testing)
//! ```
//!
//! ## Status
//!
//! This module defines the interface. Actual model backends require
//! `candle` or `ort` (ONNX Runtime) — add via `--features vision-ml`.
//! Without the feature flag, only `MockVisionBackend` is available.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn, instrument};

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A UI element detected by vision analysis.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiElement {
    /// Unique ID for referencing this element
    pub id: u32,
    /// Bounding box: (x, y, width, height) in pixels
    pub bbox: BoundingBox,
    /// What kind of UI element this is
    pub element_type: UiElementType,
    /// Visible text content (OCR'd)
    pub text: String,
    /// Confidence score from the model (0.0 - 1.0)
    pub confidence: f64,
    /// Additional attributes detected
    pub attributes: HashMap<String, String>,
}

/// Bounding box in pixel coordinates.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct BoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl BoundingBox {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self { x, y, width, height }
    }

    /// Center point of the bounding box.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    /// Area in square pixels.
    pub fn area(&self) -> f64 {
        self.width * self.height
    }

    /// Whether this box contains a point.
    pub fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.width
            && py >= self.y && py <= self.y + self.height
    }

    /// Intersection over Union with another box.
    pub fn iou(&self, other: &BoundingBox) -> f64 {
        let x1 = self.x.max(other.x);
        let y1 = self.y.max(other.y);
        let x2 = (self.x + self.width).min(other.x + other.width);
        let y2 = (self.y + self.height).min(other.y + other.height);

        if x2 <= x1 || y2 <= y1 {
            return 0.0;
        }

        let intersection = (x2 - x1) * (y2 - y1);
        let union = self.area() + other.area() - intersection;

        if union <= 0.0 {
            0.0
        } else {
            intersection / union
        }
    }
}

/// Classification of detected UI elements.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UiElementType {
    Button,
    TextInput,
    Checkbox,
    RadioButton,
    Dropdown,
    Link,
    Image,
    Icon,
    Text,
    Heading,
    Navigation,
    Menu,
    Dialog,
    Tab,
    Slider,
    Toggle,
    SearchBox,
    LoginForm,
    Unknown,
}

/// Result of vision analysis on a screenshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisionResult {
    /// Detected UI elements
    pub elements: Vec<UiElement>,
    /// Overall page description
    pub page_description: String,
    /// Detected page type
    pub page_type: String,
    /// Model used for inference
    pub model_name: String,
    /// Inference time in milliseconds
    pub inference_ms: u64,
    /// Image dimensions analyzed
    pub image_width: u32,
    pub image_height: u32,
}

impl VisionResult {
    /// Format elements for agent consumption (similar to DOM snapshot).
    #[instrument(skip(self))]
    pub fn to_agent_text(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!("[Vision Analysis — {} elements detected]", self.elements.len()));
        lines.push(format!("Page: {}", self.page_description));
        lines.push(format!("Type: {}", self.page_type));
        lines.push(String::new());

        for el in &self.elements {
            let (cx, cy) = el.bbox.center();
            let type_str = format!("{:?}", el.element_type);
            let text_preview = if el.text.len() > 50 {
                format!("{}...", &el.text[..47])
            } else {
                el.text.clone()
            };

            lines.push(format!(
                "@v{} [{}] \"{}\" at ({:.0},{:.0}) {:.0}x{:.0} ({:.0}%)",
                el.id, type_str, text_preview,
                cx, cy, el.bbox.width, el.bbox.height,
                el.confidence * 100.0
            ));
        }

        lines.join("\n")
    }

    /// Find element closest to a point.
    pub fn element_at(&self, x: f64, y: f64) -> Option<&UiElement> {
        self.elements.iter()
            .filter(|el| el.bbox.contains(x, y))
            .max_by(|a, b| a.confidence.partial_cmp(&b.confidence).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Find elements by type.
    pub fn elements_of_type(&self, element_type: UiElementType) -> Vec<&UiElement> {
        self.elements.iter()
            .filter(|el| el.element_type == element_type)
            .collect()
    }

    /// Find element by text content (case-insensitive substring).
    pub fn find_by_text(&self, query: &str) -> Vec<&UiElement> {
        let query_lower = query.to_lowercase();
        self.elements.iter()
            .filter(|el| el.text.to_lowercase().contains(&query_lower))
            .collect()
    }

    /// Apply non-maximum suppression to remove overlapping detections.
    pub fn nms(&mut self, iou_threshold: f64) {
        // Sort by confidence descending
        self.elements.sort_by(|a, b| {
            b.confidence.partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut keep = vec![true; self.elements.len()];
        #[allow(clippy::needless_range_loop)]
        for i in 0..self.elements.len() {
            if !keep[i] {
                continue;
            }
            for j in (i + 1)..self.elements.len() {
                if !keep[j] {
                    continue;
                }
                if self.elements[i].bbox.iou(&self.elements[j].bbox) > iou_threshold {
                    keep[j] = false;
                }
            }
        }

        let mut idx = 0;
        self.elements.retain(|_| {
            let k = keep[idx];
            idx += 1;
            k
        });
    }
}

// ---------------------------------------------------------------------------
// Vision backend trait
// ---------------------------------------------------------------------------

/// Trait for vision model backends.
///
/// Implement this to plug in different ML models for UI element detection.
/// The default implementation (`MockVisionBackend`) returns hardcoded results
/// for testing. Real implementations would use candle or ort.
pub trait VisionBackend: Send + Sync {
    /// Analyze a screenshot and detect UI elements.
    fn analyze(&self, png_data: &[u8], width: u32, height: u32) -> Result<VisionResult, String>;

    /// Get the name of this backend.
    fn name(&self) -> &str;

    /// Check if the backend is available (model files present, etc.).
    fn is_available(&self) -> bool;
}

// ---------------------------------------------------------------------------
// Mock backend for testing
// ---------------------------------------------------------------------------

/// Mock vision backend that returns synthetic results.
/// Useful for testing the pipeline without requiring ML model files.
pub struct MockVisionBackend;

impl VisionBackend for MockVisionBackend {
    fn analyze(&self, _png_data: &[u8], width: u32, height: u32) -> Result<VisionResult, String> {
        info!(width, height, "MockVisionBackend: generating synthetic elements");

        let elements = vec![
            UiElement {
                id: 1,
                bbox: BoundingBox::new(10.0, 10.0, 200.0, 40.0),
                element_type: UiElementType::Navigation,
                text: "Home | About | Contact".to_string(),
                confidence: 0.92,
                attributes: HashMap::new(),
            },
            UiElement {
                id: 2,
                bbox: BoundingBox::new(50.0, 100.0, 300.0, 30.0),
                element_type: UiElementType::SearchBox,
                text: "Search...".to_string(),
                confidence: 0.88,
                attributes: HashMap::new(),
            },
            UiElement {
                id: 3,
                bbox: BoundingBox::new(400.0, 100.0, 80.0, 30.0),
                element_type: UiElementType::Button,
                text: "Search".to_string(),
                confidence: 0.95,
                attributes: HashMap::new(),
            },
        ];

        Ok(VisionResult {
            elements,
            page_description: "Mock page with navigation, search box, and button".to_string(),
            page_type: "search_page".to_string(),
            model_name: "MockVision".to_string(),
            inference_ms: 0,
            image_width: width,
            image_height: height,
        })
    }

    fn name(&self) -> &str {
        "MockVision"
    }

    fn is_available(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Placeholder backends (for documentation — require feature flags to activate)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// ONNX Runtime backend (feature-gated behind `vision-ml`)
// ---------------------------------------------------------------------------

/// OmniParser v2 backend (YOLOv8 + Florence-2) via ONNX Runtime.
/// Requires `--features vision-ml` and model files at `~/.openclaw/models/omniparser/`.
///
/// Expected model files:
/// - `detector.onnx` — YOLOv8 object detection model
/// - `classifier.onnx` — Florence-2 element classification model
///
/// Performance: ~0.6s on GPU, ~2-3s on CPU.
pub struct OmniParserBackend {
    #[allow(dead_code)]
    model_dir: String,
    #[cfg(feature = "vision-ml")]
    session: Option<ort::session::Session>,
}

impl OmniParserBackend {
    pub fn new(model_dir: &str) -> Self {
        #[cfg(feature = "vision-ml")]
        {
            let detector_path = std::path::Path::new(model_dir).join("detector.onnx");
            let session = if detector_path.exists() {
                match ort::session::Session::builder()
                    .and_then(|b| b.with_model_from_file(&detector_path))
                {
                    Ok(s) => {
                        info!(model = %detector_path.display(), "OmniParser ONNX model loaded");
                        Some(s)
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to load OmniParser model");
                        None
                    }
                }
            } else {
                warn!(path = %detector_path.display(), "OmniParser model not found");
                None
            };
            return Self { model_dir: model_dir.to_string(), session };
        }
        #[cfg(not(feature = "vision-ml"))]
        Self { model_dir: model_dir.to_string() }
    }
}

impl VisionBackend for OmniParserBackend {
    fn analyze(&self, _png_data: &[u8], width: u32, height: u32) -> Result<VisionResult, String> {
        #[cfg(feature = "vision-ml")]
        {
            let start = std::time::Instant::now();

            if self.session.is_none() {
                return Err(format!(
                    "OmniParser model not loaded. Place detector.onnx in {}",
                    self.model_dir
                ));
            }

            // The session is loaded — in a full implementation we would:
            // 1. Decode PNG to raw pixels (RGB, resized to model input dims)
            // 2. Build input tensor from pixel data
            // 3. Run session.run() with the tensor
            // 4. Parse output tensors (bounding boxes + class scores)
            // 5. Map to UiElement structs
            //
            // For now, we verify the model loads and return a placeholder
            // indicating the runtime is functional.
            let inference_ms = start.elapsed().as_millis() as u64;

            info!(model = "OmniParser", inference_ms, "ONNX session ready — model loaded successfully");

            return Ok(VisionResult {
                elements: vec![],
                page_description: "OmniParser ONNX model loaded — inference pipeline ready".to_string(),
                page_type: "pending_inference".to_string(),
                model_name: "OmniParser (ort)".to_string(),
                inference_ms,
                image_width: width,
                image_height: height,
            });
        }

        #[cfg(not(feature = "vision-ml"))]
        {
            warn!("OmniParser backend not compiled — enable --features vision-ml");
            Ok(VisionResult {
                elements: vec![],
                page_description: "OmniParser not available".to_string(),
                page_type: "unknown".to_string(),
                model_name: "OmniParser (not loaded)".to_string(),
                inference_ms: 0,
                image_width: width,
                image_height: height,
            })
        }
    }

    fn name(&self) -> &str { "OmniParser" }

    fn is_available(&self) -> bool {
        #[cfg(feature = "vision-ml")]
        { return self.session.is_some(); }
        #[cfg(not(feature = "vision-ml"))]
        { false }
    }
}

/// Moondream 0.5B backend — lightweight VLM, runs on CPU in sub-second.
/// Requires `--features vision-ml` and model file at `~/.openclaw/models/moondream/model.onnx`.
pub struct MoondreamBackend {
    #[allow(dead_code)]
    model_dir: String,
    #[cfg(feature = "vision-ml")]
    session: Option<ort::session::Session>,
}

impl MoondreamBackend {
    pub fn new(model_dir: &str) -> Self {
        #[cfg(feature = "vision-ml")]
        {
            let model_path = std::path::Path::new(model_dir).join("model.onnx");
            let session = if model_path.exists() {
                match ort::session::Session::builder()
                    .and_then(|b| b.with_model_from_file(&model_path))
                {
                    Ok(s) => {
                        info!(model = %model_path.display(), "Moondream ONNX model loaded");
                        Some(s)
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to load Moondream model");
                        None
                    }
                }
            } else {
                warn!(path = %model_path.display(), "Moondream model not found");
                None
            };
            return Self { model_dir: model_dir.to_string(), session };
        }
        #[cfg(not(feature = "vision-ml"))]
        Self { model_dir: model_dir.to_string() }
    }
}

impl VisionBackend for MoondreamBackend {
    fn analyze(&self, _png_data: &[u8], width: u32, height: u32) -> Result<VisionResult, String> {
        #[cfg(feature = "vision-ml")]
        {
            let start = std::time::Instant::now();

            if self.session.is_none() {
                return Err(format!(
                    "Moondream model not loaded. Place model.onnx in {}",
                    self.model_dir
                ));
            }

            let inference_ms = start.elapsed().as_millis() as u64;

            info!(model = "Moondream", inference_ms, "ONNX session ready — model loaded successfully");

            return Ok(VisionResult {
                elements: vec![],
                page_description: "Moondream ONNX model loaded — inference pipeline ready".to_string(),
                page_type: "pending_inference".to_string(),
                model_name: "Moondream 0.5B (ort)".to_string(),
                inference_ms,
                image_width: width,
                image_height: height,
            });
        }

        #[cfg(not(feature = "vision-ml"))]
        {
            warn!("Moondream backend not compiled — enable --features vision-ml");
            Ok(VisionResult {
                elements: vec![],
                page_description: "Moondream not available".to_string(),
                page_type: "unknown".to_string(),
                model_name: "Moondream (not loaded)".to_string(),
                inference_ms: 0,
                image_width: width,
                image_height: height,
            })
        }
    }

    fn name(&self) -> &str { "Moondream" }

    fn is_available(&self) -> bool {
        #[cfg(feature = "vision-ml")]
        { return self.session.is_some(); }
        #[cfg(not(feature = "vision-ml"))]
        { false }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbox_center() {
        let b = BoundingBox::new(10.0, 20.0, 100.0, 50.0);
        assert_eq!(b.center(), (60.0, 45.0));
    }

    #[test]
    fn bbox_area() {
        let b = BoundingBox::new(0.0, 0.0, 100.0, 50.0);
        assert_eq!(b.area(), 5000.0);
    }

    #[test]
    fn bbox_contains() {
        let b = BoundingBox::new(10.0, 10.0, 100.0, 50.0);
        assert!(b.contains(50.0, 30.0));
        assert!(!b.contains(5.0, 5.0));
        assert!(!b.contains(200.0, 200.0));
    }

    #[test]
    fn bbox_iou_identical() {
        let a = BoundingBox::new(0.0, 0.0, 100.0, 100.0);
        let b = BoundingBox::new(0.0, 0.0, 100.0, 100.0);
        assert!((a.iou(&b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn bbox_iou_no_overlap() {
        let a = BoundingBox::new(0.0, 0.0, 50.0, 50.0);
        let b = BoundingBox::new(100.0, 100.0, 50.0, 50.0);
        assert_eq!(a.iou(&b), 0.0);
    }

    #[test]
    fn bbox_iou_partial() {
        let a = BoundingBox::new(0.0, 0.0, 100.0, 100.0);
        let b = BoundingBox::new(50.0, 50.0, 100.0, 100.0);
        let iou = a.iou(&b);
        assert!(iou > 0.0 && iou < 1.0);
        // Intersection: 50x50 = 2500, Union: 10000+10000-2500 = 17500
        assert!((iou - 2500.0 / 17500.0).abs() < 0.001);
    }

    #[test]
    fn mock_backend_available() {
        let backend = MockVisionBackend;
        assert!(backend.is_available());
        assert_eq!(backend.name(), "MockVision");
    }

    #[test]
    fn mock_backend_analyze() {
        let backend = MockVisionBackend;
        let result = backend.analyze(&[], 1920, 1080).unwrap();
        assert!(!result.elements.is_empty());
        assert_eq!(result.image_width, 1920);
        assert_eq!(result.image_height, 1080);
    }

    #[test]
    fn vision_result_to_agent_text() {
        let backend = MockVisionBackend;
        let result = backend.analyze(&[], 1920, 1080).unwrap();
        let text = result.to_agent_text();
        assert!(text.contains("@v1"));
        assert!(text.contains("@v2"));
        assert!(text.contains("Search"));
    }

    #[test]
    fn vision_result_find_by_text() {
        let backend = MockVisionBackend;
        let result = backend.analyze(&[], 1920, 1080).unwrap();
        let found = result.find_by_text("search");
        assert_eq!(found.len(), 2); // SearchBox and Button
    }

    #[test]
    fn vision_result_elements_of_type() {
        let backend = MockVisionBackend;
        let result = backend.analyze(&[], 1920, 1080).unwrap();
        let buttons = result.elements_of_type(UiElementType::Button);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].text, "Search");
    }

    #[test]
    fn vision_result_element_at() {
        let backend = MockVisionBackend;
        let result = backend.analyze(&[], 1920, 1080).unwrap();
        // Click on the search box area
        let el = result.element_at(100.0, 110.0);
        assert!(el.is_some());
        assert_eq!(el.unwrap().element_type, UiElementType::SearchBox);
    }

    #[test]
    fn nms_removes_overlapping() {
        let mut result = VisionResult {
            elements: vec![
                UiElement {
                    id: 1,
                    bbox: BoundingBox::new(10.0, 10.0, 100.0, 50.0),
                    element_type: UiElementType::Button,
                    text: "Click".to_string(),
                    confidence: 0.9,
                    attributes: HashMap::new(),
                },
                UiElement {
                    id: 2,
                    bbox: BoundingBox::new(15.0, 12.0, 95.0, 48.0), // overlaps heavily with id=1
                    element_type: UiElementType::Button,
                    text: "Click".to_string(),
                    confidence: 0.7,
                    attributes: HashMap::new(),
                },
                UiElement {
                    id: 3,
                    bbox: BoundingBox::new(500.0, 500.0, 80.0, 30.0), // no overlap
                    element_type: UiElementType::Link,
                    text: "More".to_string(),
                    confidence: 0.85,
                    attributes: HashMap::new(),
                },
            ],
            page_description: String::new(),
            page_type: String::new(),
            model_name: String::new(),
            inference_ms: 0,
            image_width: 1920,
            image_height: 1080,
        };

        result.nms(0.5);
        assert_eq!(result.elements.len(), 2); // id=2 removed (lower confidence, high overlap)
    }

    #[test]
    fn placeholder_backends_not_available() {
        let omni = OmniParserBackend::new("/tmp/models");
        assert!(!omni.is_available());

        let moon = MoondreamBackend::new("/tmp/models");
        assert!(!moon.is_available());
    }
}
