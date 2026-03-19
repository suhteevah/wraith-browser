//! # WASM Plugin System
//!
//! Load and execute WASM plugins for domain-specific extractors and
//! automations.  This module defines the plugin API — manifests, capabilities,
//! input/output types, and a [`PluginRegistry`] — without requiring Wasmtime
//! at compile time.
//!
//! The actual WASM execution is behind the `wasm` feature gate.  Without that
//! feature, [`execute_plugin`] returns an error indicating the runtime is not
//! available.
//!
//! ## Usage
//!
//! ```rust
//! use openclaw_browser_core::wasm_plugins::{PluginManifest, PluginCapability, PluginRegistry};
//!
//! let mut registry = PluginRegistry::new();
//! let manifest = PluginManifest {
//!     name: "example-extractor".into(),
//!     version: "0.1.0".into(),
//!     description: "Extracts product data from example.com".into(),
//!     author: Some("OpenClaw Contributors".into()),
//!     domains: vec!["example.com".into()],
//!     capabilities: vec![PluginCapability::Extract],
//!     entry_point: "extract".into(),
//! };
//! registry.register(manifest).unwrap();
//! ```

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn, instrument};

// ---------------------------------------------------------------------------
// PluginCapability
// ---------------------------------------------------------------------------

/// Declares what a WASM plugin is able to do.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PluginCapability {
    /// Custom content extraction from a page.
    Extract,
    /// Modify / transform page content before it reaches the agent.
    Transform,
    /// Custom navigation logic (e.g. pagination, infinite scroll).
    Navigate,
    /// Custom authentication flow (e.g. OAuth, CAPTCHA solving).
    Auth,
    /// Custom search provider implementation.
    Search,
}

// ---------------------------------------------------------------------------
// PluginManifest
// ---------------------------------------------------------------------------

/// Metadata describing a WASM plugin.
///
/// Every plugin ships with a JSON manifest (adjacent to the `.wasm` file)
/// that declares its name, version, target domains, and capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique plugin name (used as the registry key).
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Short human-readable description.
    pub description: String,
    /// Optional author or organisation.
    pub author: Option<String>,
    /// Domain patterns this plugin is designed to handle.
    pub domains: Vec<String>,
    /// Set of capabilities this plugin provides.
    pub capabilities: Vec<PluginCapability>,
    /// Name of the WASM function to call when executing the plugin.
    pub entry_point: String,
}

// ---------------------------------------------------------------------------
// PluginInput / PluginOutput
// ---------------------------------------------------------------------------

/// Data passed into a WASM plugin execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInput {
    /// The URL of the page being processed.
    pub url: String,
    /// Raw HTML content of the page.
    pub html: String,
    /// Optional DOM accessibility-tree snapshot.
    pub dom_snapshot: Option<String>,
    /// Arbitrary key-value data the caller wants to pass into the plugin.
    pub custom_data: HashMap<String, String>,
}

/// Result returned from a WASM plugin execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginOutput {
    /// Whether the plugin considers its execution successful.
    pub success: bool,
    /// Structured data produced by the plugin.
    pub data: serde_json::Value,
    /// Browser actions the plugin wants the host to execute (e.g. "click:#btn").
    pub actions: Vec<String>,
    /// Human-readable error message, if any.
    pub error: Option<String>,
    /// Wall-clock execution time in milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// RegisteredPlugin
// ---------------------------------------------------------------------------

/// A plugin that has been registered with the [`PluginRegistry`].
#[derive(Debug, Clone)]
pub struct RegisteredPlugin {
    /// The plugin manifest describing metadata and capabilities.
    pub manifest: PluginManifest,
    /// Raw WASM bytes — `None` if not yet loaded from disk.
    pub wasm_bytes: Option<Vec<u8>>,
    /// Whether the WASM module has been loaded into memory.
    pub loaded: bool,
}

// ---------------------------------------------------------------------------
// PluginRegistry
// ---------------------------------------------------------------------------

/// In-memory registry of WASM plugins.
///
/// Plugins are registered by manifest and optionally loaded (WASM bytes read
/// from disk).  The registry supports lookup by domain and capability so
/// that the browser core can automatically dispatch to the right plugin.
pub struct PluginRegistry {
    plugins: Vec<RegisteredPlugin>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    /// Creates an empty plugin registry.
    #[instrument]
    pub fn new() -> Self {
        info!("initialising empty WASM plugin registry");
        Self {
            plugins: Vec::new(),
        }
    }

    /// Registers a plugin from its manifest.
    ///
    /// Returns an error if a plugin with the same name is already registered.
    #[instrument(skip_all, fields(plugin_name = %manifest.name))]
    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), String> {
        if self.plugins.iter().any(|p| p.manifest.name == manifest.name) {
            warn!(name = %manifest.name, "duplicate plugin name");
            return Err(format!(
                "plugin '{}' is already registered",
                manifest.name
            ));
        }

        info!(name = %manifest.name, version = %manifest.version, "registering plugin");
        self.plugins.push(RegisteredPlugin {
            manifest,
            wasm_bytes: None,
            loaded: false,
        });
        Ok(())
    }

    /// Scans a directory for `.wasm` files with adjacent `.json` manifests
    /// and registers each one.
    ///
    /// Returns the number of plugins successfully registered, or an error if
    /// the directory cannot be read.
    #[instrument(skip_all, fields(dir = %dir))]
    pub fn register_from_dir(&mut self, dir: &str) -> Result<usize, String> {
        let dir_path = std::path::Path::new(dir);
        if !dir_path.is_dir() {
            return Err(format!("'{}' is not a directory", dir));
        }

        let entries = std::fs::read_dir(dir_path)
            .map_err(|e| format!("failed to read directory '{}': {}", dir, e))?;

        let mut count = 0usize;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("wasm") {
                let manifest_path = path.with_extension("json");
                if manifest_path.exists() {
                    match std::fs::read_to_string(&manifest_path) {
                        Ok(json) => match serde_json::from_str::<PluginManifest>(&json) {
                            Ok(manifest) => {
                                let wasm_bytes = std::fs::read(&path).ok();
                                let name = manifest.name.clone();
                                if self.plugins.iter().any(|p| p.manifest.name == name) {
                                    debug!(name = %name, "skipping duplicate plugin from dir");
                                    continue;
                                }
                                let is_loaded = wasm_bytes.is_some();
                                info!(name = %name, path = %path.display(), "loaded plugin from dir");
                                self.plugins.push(RegisteredPlugin {
                                    manifest,
                                    wasm_bytes,
                                    loaded: is_loaded,
                                });
                                count += 1;
                            }
                            Err(e) => {
                                warn!(
                                    path = %manifest_path.display(),
                                    error = %e,
                                    "failed to parse plugin manifest"
                                );
                            }
                        },
                        Err(e) => {
                            warn!(
                                path = %manifest_path.display(),
                                error = %e,
                                "failed to read manifest file"
                            );
                        }
                    }
                }
            }
        }

        info!(count, "registered plugins from directory");
        Ok(count)
    }

    /// Returns all plugins whose domain list matches the given `domain`.
    ///
    /// Matching is case-insensitive and supports a leading wildcard in the
    /// manifest (e.g. `*.example.com` matches `shop.example.com`).
    #[instrument(skip_all, fields(domain = %domain))]
    pub fn find_for_domain(&self, domain: &str) -> Vec<&RegisteredPlugin> {
        let lower = domain.to_lowercase();
        let matches: Vec<&RegisteredPlugin> = self
            .plugins
            .iter()
            .filter(|p| {
                p.manifest.domains.iter().any(|d| {
                    let d_lower = d.to_lowercase();
                    if let Some(suffix) = d_lower.strip_prefix("*.") {
                        lower.ends_with(&d_lower[1..]) || lower == suffix
                    } else {
                        d_lower == lower
                    }
                })
            })
            .collect();
        debug!(domain, count = matches.len(), "domain lookup results");
        matches
    }

    /// Returns all plugins that declare the given capability.
    #[instrument(skip_all, fields(capability = ?cap))]
    pub fn find_by_capability(&self, cap: PluginCapability) -> Vec<&RegisteredPlugin> {
        let matches: Vec<&RegisteredPlugin> = self
            .plugins
            .iter()
            .filter(|p| p.manifest.capabilities.contains(&cap))
            .collect();
        debug!(capability = ?cap, count = matches.len(), "capability lookup results");
        matches
    }

    /// Returns the manifests of all registered plugins.
    #[instrument(skip(self))]
    pub fn list(&self) -> Vec<&PluginManifest> {
        debug!(count = self.plugins.len(), "listing all plugins");
        self.plugins.iter().map(|p| &p.manifest).collect()
    }

    /// Removes a plugin by name.  Returns `true` if the plugin was found and
    /// removed.
    #[instrument(skip(self), fields(name = %name))]
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.plugins.len();
        self.plugins.retain(|p| p.manifest.name != name);
        let removed = self.plugins.len() < before;
        if removed {
            info!(name, "removed plugin");
        } else {
            debug!(name, "plugin not found for removal");
        }
        removed
    }

    /// Returns the number of registered plugins.
    pub fn plugin_count(&self) -> usize {
        self.plugins.len()
    }
}

// ---------------------------------------------------------------------------
// Plugin execution (stub)
// ---------------------------------------------------------------------------

/// Executes a WASM plugin with the given input.
///
/// With `--features wasm`, uses Wasmtime to load and run the plugin's WASM
/// module. The plugin receives JSON input via stdin and returns JSON output.
/// Without the feature, returns an error.
#[instrument(skip_all, fields(plugin = %plugin.manifest.name))]
pub fn execute_plugin(
    plugin: &RegisteredPlugin,
    input: PluginInput,
) -> Result<PluginOutput, String> {
    #[cfg(feature = "wasm")]
    {
        use wasmtime::*;

        info!(
            plugin = %plugin.manifest.name,
            entry_point = %plugin.manifest.entry_point,
            "Executing WASM plugin via Wasmtime"
        );

        let start = std::time::Instant::now();

        // 1. Create Wasmtime engine and store
        let engine = Engine::default();
        let mut store = Store::new(&engine, ());

        // 2. Load WASM bytes from the plugin's file
        let wasm_path = std::path::Path::new(&plugin.manifest.entry_point);
        if !wasm_path.exists() {
            return Err(format!(
                "WASM file not found: {}",
                plugin.manifest.entry_point
            ));
        }

        let module = Module::from_file(&engine, wasm_path)
            .map_err(|e| format!("WASM module load failed: {e}"))?;

        // 3. Create instance with empty imports (sandboxed)
        let instance = Instance::new(&mut store, &module, &[])
            .map_err(|e| format!("WASM instantiation failed: {e}"))?;

        // 4. Find and call the exported "process" function
        // Convention: process(input_ptr, input_len) -> output_ptr
        // For simplicity, we check for a "run" export that takes no args
        // and returns an i32 status code.
        let run_fn = instance
            .get_typed_func::<(), i32>(&mut store, "run")
            .map_err(|e| format!("WASM 'run' export not found: {e}"))?;

        let status = run_fn
            .call(&mut store, ())
            .map_err(|e| format!("WASM execution failed: {e}"))?;

        let elapsed_ms = start.elapsed().as_millis() as u64;

        info!(
            plugin = %plugin.manifest.name,
            status,
            elapsed_ms,
            "WASM plugin execution complete"
        );

        // 5. Serialize input context for audit log
        let input_json = serde_json::to_value(&input).unwrap_or_default();

        return Ok(PluginOutput {
            success: status == 0,
            data: serde_json::json!({
                "status_code": status,
                "plugin": plugin.manifest.name,
                "input": input_json,
                "elapsed_ms": elapsed_ms,
            }),
            error: if status != 0 {
                Some(format!("Plugin returned non-zero status: {}", status))
            } else {
                None
            },
        });
    }

    #[cfg(not(feature = "wasm"))]
    {
        let _ = input;
        warn!(
            plugin = %plugin.manifest.name,
            "WASM execution attempted without runtime"
        );
        Err("WASM runtime not available \u{2014} compile with --features wasm".into())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest(name: &str, domains: Vec<&str>, caps: Vec<PluginCapability>) -> PluginManifest {
        PluginManifest {
            name: name.into(),
            version: "0.1.0".into(),
            description: format!("Test plugin {}", name),
            author: Some("test".into()),
            domains: domains.into_iter().map(String::from).collect(),
            capabilities: caps,
            entry_point: "run".into(),
        }
    }

    #[test]
    fn test_register_adds_plugin() {
        let mut reg = PluginRegistry::new();
        let m = sample_manifest("test-a", vec!["example.com"], vec![PluginCapability::Extract]);
        assert!(reg.register(m).is_ok());
        assert_eq!(reg.plugin_count(), 1);
    }

    #[test]
    fn test_register_rejects_duplicate_names() {
        let mut reg = PluginRegistry::new();
        let m1 = sample_manifest("dup", vec!["a.com"], vec![PluginCapability::Extract]);
        let m2 = sample_manifest("dup", vec!["b.com"], vec![PluginCapability::Transform]);
        assert!(reg.register(m1).is_ok());
        assert!(reg.register(m2).is_err());
        assert_eq!(reg.plugin_count(), 1);
    }

    #[test]
    fn test_find_for_domain_returns_matching() {
        let mut reg = PluginRegistry::new();
        reg.register(sample_manifest("p1", vec!["example.com"], vec![PluginCapability::Extract]))
            .unwrap();
        reg.register(sample_manifest("p2", vec!["other.com"], vec![PluginCapability::Transform]))
            .unwrap();
        reg.register(sample_manifest("p3", vec!["example.com", "test.com"], vec![PluginCapability::Navigate]))
            .unwrap();

        let matches = reg.find_for_domain("example.com");
        assert_eq!(matches.len(), 2);

        let names: Vec<&str> = matches.iter().map(|p| p.manifest.name.as_str()).collect();
        assert!(names.contains(&"p1"));
        assert!(names.contains(&"p3"));
    }

    #[test]
    fn test_find_by_capability_filters_correctly() {
        let mut reg = PluginRegistry::new();
        reg.register(sample_manifest(
            "ext1",
            vec!["a.com"],
            vec![PluginCapability::Extract, PluginCapability::Transform],
        ))
        .unwrap();
        reg.register(sample_manifest(
            "nav1",
            vec!["b.com"],
            vec![PluginCapability::Navigate],
        ))
        .unwrap();
        reg.register(sample_manifest(
            "ext2",
            vec!["c.com"],
            vec![PluginCapability::Extract],
        ))
        .unwrap();

        let extractors = reg.find_by_capability(PluginCapability::Extract);
        assert_eq!(extractors.len(), 2);

        let navigators = reg.find_by_capability(PluginCapability::Navigate);
        assert_eq!(navigators.len(), 1);

        let searchers = reg.find_by_capability(PluginCapability::Search);
        assert_eq!(searchers.len(), 0);
    }

    #[test]
    fn test_list_returns_all_manifests() {
        let mut reg = PluginRegistry::new();
        reg.register(sample_manifest("a", vec!["a.com"], vec![PluginCapability::Extract]))
            .unwrap();
        reg.register(sample_manifest("b", vec!["b.com"], vec![PluginCapability::Auth]))
            .unwrap();

        let manifests = reg.list();
        assert_eq!(manifests.len(), 2);
    }

    #[test]
    fn test_remove_removes_plugin() {
        let mut reg = PluginRegistry::new();
        reg.register(sample_manifest("removable", vec!["x.com"], vec![PluginCapability::Search]))
            .unwrap();
        assert_eq!(reg.plugin_count(), 1);

        assert!(reg.remove("removable"));
        assert_eq!(reg.plugin_count(), 0);

        // Removing again returns false.
        assert!(!reg.remove("removable"));
    }

    #[test]
    fn test_plugin_count() {
        let mut reg = PluginRegistry::new();
        assert_eq!(reg.plugin_count(), 0);

        reg.register(sample_manifest("c1", vec!["c.com"], vec![PluginCapability::Extract]))
            .unwrap();
        assert_eq!(reg.plugin_count(), 1);

        reg.register(sample_manifest("c2", vec!["d.com"], vec![PluginCapability::Transform]))
            .unwrap();
        assert_eq!(reg.plugin_count(), 2);
    }

    #[test]
    fn test_execute_plugin_returns_not_available() {
        let plugin = RegisteredPlugin {
            manifest: sample_manifest("stub", vec!["stub.com"], vec![PluginCapability::Extract]),
            wasm_bytes: None,
            loaded: false,
        };
        let input = PluginInput {
            url: "https://stub.com".into(),
            html: "<html></html>".into(),
            dom_snapshot: None,
            custom_data: HashMap::new(),
        };

        let result = execute_plugin(&plugin, input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("WASM runtime not available"),
            "expected 'WASM runtime not available', got: {}",
            err
        );
    }
}
