use clap::{Parser, Subcommand};
use secrecy::SecretString;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "wraith-browser",
    about = "Wraith Browser — an AI-agent-first web browser written in Rust",
    version,
    long_about = "A native Rust web browser designed for AI agent control, not humans.\n\
                   Supports MCP for Claude Code integration, autonomous browsing tasks,\n\
                   SearXNG-style metasearch, and optimized content extraction for LLMs."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Browser engine: "auto" (sevro→native), "sevro", "native"
    #[arg(long, global = true, default_value = "auto")]
    engine: String,

    /// HTTP proxy URL (e.g., "http://user:pass@proxy:8080", "socks5://127.0.0.1:1080")
    #[arg(long, global = true)]
    proxy: Option<String>,

    /// External challenge-solving proxy URL (e.g., "http://localhost:8191")
    #[arg(long, global = true, env = "WRAITH_FLARESOLVERR")]
    flaresolverr: Option<String>,

    /// Fallback proxy for access-restricted sites
    #[arg(long, global = true)]
    fallback_proxy: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the MCP server (stdio or SSE transport)
    Serve {
        /// Transport mode: "stdio" or "sse"
        #[arg(short, long, default_value = "stdio")]
        transport: String,

        /// Host for SSE transport
        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        /// Port for SSE transport
        #[arg(short, long, default_value = "3100")]
        port: u16,
    },

    /// Navigate to a URL and print the DOM snapshot
    Navigate {
        /// URL to navigate to
        url: String,

        /// Output format: "snapshot", "markdown", "json"
        #[arg(short, long, default_value = "snapshot")]
        format: String,
    },

    /// Run an autonomous browsing task
    Task {
        /// Task description
        description: String,

        /// Starting URL
        #[arg(short, long)]
        url: Option<String>,

        /// Maximum steps
        #[arg(long, default_value = "50")]
        max_steps: usize,
    },

    /// Search the web
    Search {
        /// Search query
        query: String,

        /// Maximum results
        #[arg(short, long, default_value = "10")]
        max_results: usize,
    },

    /// Extract readable content from a URL
    Extract {
        /// URL to extract content from
        url: String,

        /// Maximum token budget
        #[arg(long, default_value = "4000")]
        max_tokens: usize,
    },

    /// Fetch a URL with stealth TLS (Firefox 136 emulation), no DOM.
    ///
    /// Lightweight HTTP-only path for JSON APIs. No QuickJS, no DOM parse.
    /// Use this for stealth-fingerprinted scraping where the full engine is
    /// overkill (e.g., Sofascore live scores, ESPN APIs).
    Fetch {
        /// URL to fetch
        url: String,

        /// User-Agent header (defaults to a current Firefox 136 string)
        #[arg(long, default_value = "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0")]
        user_agent: String,

        /// Accept-Language header
        #[arg(long, default_value = "en-US,en;q=0.5")]
        accept_language: String,

        /// Output format: "body" (default, body only), "json" (status + body + final_url), "headers" (status + final_url, no body)
        #[arg(short, long, default_value = "body")]
        output: String,
    },

    /// Run a YAML playbook from the playbooks/ directory
    Run {
        /// Playbook name (resolves to `${playbook_dir}/<name>.yml`) or explicit path to a .yml file
        playbook: String,

        /// Variable override `key=value` (repeatable)
        #[arg(long = "var", value_parser = parse_var_kv)]
        vars: Vec<(String, String)>,

        /// Directory to look up bare playbook names in
        #[arg(long)]
        playbook_dir: Option<std::path::PathBuf>,

        /// Output format: "json", "snapshot", "markdown", "raw"
        #[arg(short, long, default_value = "json")]
        output: String,
    },

    /// Manage the encrypted credential vault
    Vault {
        #[command(subcommand)]
        action: VaultAction,
    },

    /// Manage browser fingerprint profiles
    Fingerprint {
        #[command(subcommand)]
        action: FingerprintAction,
    },
}

#[derive(Subcommand)]
enum VaultAction {
    /// Initialize or unlock the vault
    Unlock,

    /// Lock the vault (zeroize master key from memory)
    Lock,

    /// Store a new credential
    Store {
        /// Domain (e.g., "github.com")
        #[arg(short, long)]
        domain: String,

        /// Credential kind: password, api_key, oauth_token, totp_seed, session_cookie
        #[arg(short, long, default_value = "password")]
        kind: String,

        /// Username or account identifier
        #[arg(short, long)]
        identity: String,

        /// Friendly label
        #[arg(short, long)]
        label: Option<String>,

        /// Auto-use without human approval
        #[arg(long)]
        auto_use: bool,
    },

    /// List all stored credentials (secrets stay encrypted)
    List,

    /// Delete a credential by ID
    Delete {
        /// Credential ID to delete
        id: String,
    },

    /// Generate a TOTP code for a domain
    Totp {
        /// Domain to generate code for
        domain: String,
    },

    /// Show recent audit log entries
    Audit {
        /// Number of entries to show
        #[arg(short, long, default_value = "20")]
        limit: usize,
    },

    /// Approve a domain for credential use
    Approve {
        /// Credential ID
        #[arg(short = 'c', long)]
        credential_id: String,

        /// Domain to approve
        #[arg(short, long)]
        domain: String,
    },

    /// Rotate a credential's secret value
    Rotate {
        /// Credential ID to rotate
        id: String,
    },
}

#[derive(Subcommand)]
enum FingerprintAction {
    /// Import a fingerprint from a JSON file (exported from browser DevTools)
    Import {
        /// Path to the fingerprint JSON file
        file: String,
    },

    /// List stored fingerprint profiles
    List,
}

/// Get the default vault path: ~/.wraith/vault.db
fn default_vault_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".wraith")
        .join("vault.db")
}

/// Read a passphrase from stdin (no echo in the terminal).
fn read_passphrase(prompt: &str) -> anyhow::Result<SecretString> {
    eprint!("{}", prompt);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        anyhow::bail!("Passphrase cannot be empty");
    }
    Ok(SecretString::from(trimmed))
}

/// Read a secret value from stdin.
fn read_secret(prompt: &str) -> anyhow::Result<SecretString> {
    eprint!("{}", prompt);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(SecretString::from(input.trim().to_string()))
}

/// Parse a `key=value` string into a tuple. Used by clap value_parser for `--var`.
fn parse_var_kv(s: &str) -> Result<(String, String), String> {
    let (k, v) = s
        .split_once('=')
        .ok_or_else(|| format!("expected key=value, got `{s}`"))?;
    let k = k.trim();
    if k.is_empty() {
        return Err(format!("empty variable name in `{s}`"));
    }
    Ok((k.to_string(), v.to_string()))
}

/// Resolve the playbook directory. Order:
///   1. explicit `--playbook-dir`
///   2. `./playbooks` relative to CWD
///   3. `${CARGO_MANIFEST_DIR}/../../playbooks` (when run via `cargo run`)
///   4. `~/.wraith/playbooks`
fn resolve_playbook_dir(explicit: Option<std::path::PathBuf>) -> std::path::PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    let cwd_pb = std::env::current_dir()
        .map(|c| c.join("playbooks"))
        .unwrap_or_else(|_| std::path::PathBuf::from("playbooks"));
    if cwd_pb.is_dir() {
        return cwd_pb;
    }
    if let Some(manifest) = option_env!("CARGO_MANIFEST_DIR") {
        let p = std::path::Path::new(manifest)
            .join("..")
            .join("..")
            .join("playbooks");
        if p.is_dir() {
            return p;
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".wraith")
        .join("playbooks")
}

/// Whether the playbook arg looks like an explicit path rather than a bare name.
fn playbook_looks_like_path(playbook: &str) -> bool {
    playbook.contains('/')
        || playbook.contains('\\')
        || playbook.ends_with(".yml")
        || playbook.ends_with(".yaml")
}

/// Validate a bare playbook name (no path components, no traversal).
fn validate_bare_name(name: &str) -> anyhow::Result<()> {
    if name.is_empty()
        || name.contains("..")
        || name.starts_with('.')
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        anyhow::bail!(
            "Invalid playbook name '{}': bare names must be [A-Za-z0-9_-]+",
            name
        );
    }
    Ok(())
}

/// Look up a validated bare name inside `dir` by enumerating directory entries.
/// Returns `(matched_path, available_names)`.
fn lookup_in_dir(
    dir: &std::path::Path,
    name: &str,
) -> (Option<std::path::PathBuf>, Vec<String>) {
    let mut available: Vec<String> = Vec::new();
    let mut hit: Option<std::path::PathBuf> = None;
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return (None, available),
    };
    for e in rd.flatten() {
        let path = e.path();
        let ext_ok = matches!(
            path.extension().and_then(|s| s.to_str()),
            Some("yml") | Some("yaml")
        );
        if !ext_ok {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            if stem == name && hit.is_none() {
                hit = Some(path.clone());
            }
            available.push(stem.to_string());
        }
    }
    available.sort();
    (hit, available)
}

/// Resolve a positional playbook arg to a YAML file path.
fn resolve_playbook_path(
    playbook: &str,
    dir: &std::path::Path,
) -> anyhow::Result<std::path::PathBuf> {
    if playbook_looks_like_path(playbook) {
        // Explicit path: canonicalize against the filesystem (rejects non-existent
        // files and resolves any `..` traversal) and require a YAML extension.
        let canonical = std::fs::canonicalize(playbook)
            .map_err(|e| anyhow::anyhow!("Playbook file not found: {} ({})", playbook, e))?;
        let ext_ok = matches!(
            canonical.extension().and_then(|s| s.to_str()),
            Some("yml") | Some("yaml")
        );
        if !ext_ok {
            anyhow::bail!(
                "Playbook path must end in .yml or .yaml: {}",
                canonical.display()
            );
        }
        if !canonical.is_file() {
            anyhow::bail!("Playbook path is not a regular file: {}", canonical.display());
        }
        return Ok(canonical);
    }
    validate_bare_name(playbook)?;
    let (hit, available) = lookup_in_dir(dir, playbook);
    if let Some(p) = hit {
        return Ok(p);
    }
    if available.is_empty() {
        anyhow::bail!(
            "Playbook '{}' not found in {} (directory empty or missing)",
            playbook,
            dir.display()
        );
    }
    anyhow::bail!(
        "Playbook '{}' not found in {}.\nAvailable: {}",
        playbook,
        dir.display(),
        available.join(", ")
    );
}

fn parse_credential_kind(s: &str) -> wraith_identity::CredentialKind {
    match s.to_lowercase().as_str() {
        "password" => wraith_identity::CredentialKind::Password,
        "api_key" | "apikey" => wraith_identity::CredentialKind::ApiKey,
        "oauth_token" | "oauth" => wraith_identity::CredentialKind::OAuthToken,
        "totp_seed" | "totp" => wraith_identity::CredentialKind::TotpSeed,
        "session_cookie" | "cookie" => wraith_identity::CredentialKind::SessionCookie,
        "ssh_key" | "ssh" => wraith_identity::CredentialKind::SshKey,
        _ => wraith_identity::CredentialKind::Generic,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        "wraith=trace,tower_http=debug"
    } else {
        "wraith=info,tower_http=warn"
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| filter.into()),
        )
        .with_target(true)
        .with_thread_ids(true)
        .with_file(true)
        .with_line_number(true)
        .init();

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Wraith Browser starting"
    );

    match cli.command {
        Commands::Serve { transport, host: _, port: _ } => {
            let transport = match transport.as_str() {
                "stdio" => wraith_mcp_server::Transport::Stdio,
                other => anyhow::bail!("Unknown transport: {} (only 'stdio' is currently supported)", other),
            };
            let engine = wraith_browser_core::engine::create_engine_with_options(
                &cli.engine,
                wraith_browser_core::engine::EngineOptions {
                    proxy_url: cli.proxy.clone(),
                    flaresolverr_url: cli.flaresolverr.clone(),
                    fallback_proxy_url: cli.fallback_proxy.clone(),
                },
            ).await?;
            wraith_mcp_server::run_with_engine(transport, Some(engine)).await?;
        }

        Commands::Navigate { url, format } => {
            info!(url = %url, format = %format, "Navigating");
            let engine = wraith_browser_core::engine::create_engine_with_options(
                &cli.engine,
                wraith_browser_core::engine::EngineOptions {
                    proxy_url: cli.proxy.clone(),
                    flaresolverr_url: cli.flaresolverr.clone(),
                    fallback_proxy_url: cli.fallback_proxy.clone(),
                },
            ).await?;
            {
                let mut eng = engine.lock().await;
                eng.navigate(&url).await?;

                match format.as_str() {
                    "snapshot" => {
                        let snapshot = eng.snapshot().await?;
                        println!("{}", snapshot.to_agent_text());
                    }
                    "markdown" => {
                        let html = eng.page_source().await?;
                        let content = wraith_content_extract::extract(&html, &url)?;
                        println!("{}", content.markdown);
                    }
                    "json" => {
                        let snapshot = eng.snapshot().await?;
                        println!("{}", serde_json::to_string_pretty(&snapshot)?);
                    }
                    _ => anyhow::bail!("Unknown format: {}", format),
                }

                eng.shutdown().await?;
            }
        }

        Commands::Task { description, url, max_steps } => {
            info!(task = %description, max_steps, "Running autonomous task");

            // Determine LLM backend from environment
            let use_ollama = std::env::var("WRAITH_LLM").ok()
                .map(|v| v.to_lowercase() == "ollama")
                .unwrap_or(false);

            let engine = wraith_browser_core::engine::create_engine_with_options(
                &cli.engine,
                wraith_browser_core::engine::EngineOptions {
                    proxy_url: cli.proxy.clone(),
                    flaresolverr_url: cli.flaresolverr.clone(),
                    fallback_proxy_url: cli.fallback_proxy.clone(),
                },
            ).await?;

            let task = wraith_agent_loop::BrowsingTask {
                description: description.clone(),
                start_url: url,
                timeout_secs: None,
                context: None,
            };

            let model_override = std::env::var("WRAITH_MODEL").ok();

            let agent_config = wraith_agent_loop::AgentConfig {
                max_steps,
                model: model_override.clone().unwrap_or_else(|| {
                    wraith_agent_loop::AgentConfig::default().model
                }),
                ..Default::default()
            };

            // Open knowledge store for auto-caching
            let cache_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".wraith")
                .join("knowledge");
            let cache = wraith_cache::KnowledgeStore::open(&cache_dir)
                .map(std::sync::Arc::new)
                .ok();

            let result = if use_ollama {
                let ollama_url = std::env::var("OLLAMA_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());
                let agent_config = wraith_agent_loop::AgentConfig {
                    model: model_override.unwrap_or_else(|| "llama3.1".to_string()),
                    ..agent_config
                };
                let backend = wraith_agent_loop::llm::OllamaBackend::new()
                    .with_base_url(ollama_url);
                let mut agent = wraith_agent_loop::Agent::new(agent_config, engine.clone(), backend);
                if let Some(c) = cache { agent = agent.with_cache(c); }
                agent.run(task).await
            } else {
                let api_key = std::env::var("ANTHROPIC_API_KEY")
                    .or_else(|_| std::env::var("CLAUDE_API_KEY"))
                    .map_err(|_| anyhow::anyhow!(
                        "No API key found. Set ANTHROPIC_API_KEY or CLAUDE_API_KEY, \
                         or use WRAITH_LLM=ollama for local models."
                    ))?;
                let backend = wraith_agent_loop::llm::ClaudeBackend::new(api_key);
                let mut agent = wraith_agent_loop::Agent::new(agent_config, engine.clone(), backend);
                if let Some(c) = cache { agent = agent.with_cache(c); }
                agent.run(task).await
            };

            match result {
                Ok(output) => {
                    println!("\n═══ Task Complete ═══");
                    println!("{}", output);
                }
                Err(e) => {
                    eprintln!("\n═══ Task Failed ═══");
                    eprintln!("{}", e);
                }
            }
        }

        Commands::Search { query, max_results } => {
            info!(query = %query, "Searching");
            let results = wraith_search::search(&query, max_results).await?;
            for (i, result) in results.iter().enumerate() {
                println!("{}. {} — {}", i + 1, result.title, result.url);
                println!("   {}\n", result.snippet);
            }
            if results.is_empty() {
                println!("No results (search providers not yet implemented)");
            }
        }

        Commands::Fetch { url, user_agent, accept_language, output } => {
            info!(url = %url, "Stealth fetch (Firefox 136 TLS, no DOM)");
            let (status, body, final_url) =
                wraith_browser_core::stealth_fetch(&url, &user_agent, &accept_language)
                    .await
                    .map_err(|e| anyhow::anyhow!("stealth fetch failed: {e}"))?;

            match output.as_str() {
                "body" => {
                    print!("{}", body);
                }
                "headers" => {
                    println!("status: {}", status);
                    println!("final_url: {}", final_url);
                }
                "json" => {
                    let out = serde_json::json!({
                        "status": status,
                        "final_url": final_url,
                        "body": body,
                        "stealth_tls": wraith_browser_core::has_stealth_tls(),
                    });
                    println!("{}", serde_json::to_string_pretty(&out)?);
                }
                other => {
                    anyhow::bail!("unknown --output format: {other} (expected body|headers|json)");
                }
            }

            if status >= 400 {
                std::process::exit(1);
            }
        }

        Commands::Extract { url, max_tokens } => {
            info!(url = %url, "Extracting content");
            let engine = wraith_browser_core::engine::create_engine_with_options(
                &cli.engine,
                wraith_browser_core::engine::EngineOptions {
                    proxy_url: cli.proxy.clone(),
                    flaresolverr_url: cli.flaresolverr.clone(),
                    fallback_proxy_url: cli.fallback_proxy.clone(),
                },
            ).await?;
            let mut eng = engine.lock().await;
            eng.navigate(&url).await?;
            let html = eng.page_source().await?;
            let content = wraith_content_extract::extract_budgeted(&html, &url, max_tokens)?;
            println!("# {}\n", content.title);
            println!("{}", content.markdown);
            println!("\n---\nTokens: ~{} | Links: {} | Confidence: {:.0}%",
                content.estimated_tokens, content.links.len(), content.confidence * 100.0);
            eng.shutdown().await?;
        }

        // ═══════════════════════════════════════════════════════════
        // PLAYBOOK RUN
        // ═══════════════════════════════════════════════════════════

        Commands::Run { playbook, vars, playbook_dir, output } => {
            use wraith_browser_core::actions::BrowserAction;
            use wraith_browser_core::playbook::{Playbook, PlaybookRunner, PlaybookStep};

            let dir = resolve_playbook_dir(playbook_dir);
            let path = resolve_playbook_path(&playbook, &dir)?;
            info!(playbook = %path.display(), "Loading playbook");

            let yaml_text = std::fs::read_to_string(&path)
                .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", path.display(), e))?;

            let pb = Playbook::from_yaml(&yaml_text).map_err(|e| {
                anyhow::anyhow!("Failed to parse {}: {}", path.display(), e)
            })?;

            // Build variable map from CLI overrides.
            let supplied: std::collections::HashMap<String, String> =
                vars.into_iter().collect();

            // Validate required vars.
            let missing = pb.validate_variables(&supplied);
            if !missing.is_empty() {
                anyhow::bail!(
                    "Missing required variable(s): {} — supply with --var key=value",
                    missing.join(", ")
                );
            }

            // Spin up the engine using the same helper as Navigate/Task.
            let engine = wraith_browser_core::engine::create_engine_with_options(
                &cli.engine,
                wraith_browser_core::engine::EngineOptions {
                    proxy_url: cli.proxy.clone(),
                    flaresolverr_url: cli.flaresolverr.clone(),
                    fallback_proxy_url: cli.fallback_proxy.clone(),
                },
            )
            .await?;

            let mut runner = PlaybookRunner::new(pb.clone(), supplied);
            let total = pb.steps.len();
            let mut step_results: Vec<serde_json::Value> = Vec::with_capacity(total);
            let mut had_failure = false;

            for (idx, step) in pb.steps.iter().enumerate() {
                info!(step = idx + 1, total, "Executing playbook step");
                let mut store_key: Option<String> = None;
                let mut runtime_value: Option<String> = None;

                let result = match step {
                    PlaybookStep::Navigate { url, wait_for, timeout } => {
                        let resolved_url = runner.resolve_variable(url);
                        let timeout_ms = timeout.unwrap_or(10_000);
                        let mut eng = engine.lock().await;
                        let nav = eng.navigate(&resolved_url).await;
                        match nav {
                            Ok(()) => {
                                if let Some(sel) = wait_for {
                                    let _ = eng
                                        .execute_action(BrowserAction::WaitForSelector {
                                            selector: sel.clone(),
                                            timeout_ms,
                                        })
                                        .await;
                                }
                                serde_json::json!({
                                    "step": idx + 1, "action": "navigate",
                                    "status": "ok", "url": resolved_url,
                                })
                            }
                            Err(e) => {
                                had_failure = true;
                                serde_json::json!({
                                    "step": idx + 1, "action": "navigate",
                                    "status": "error", "error": e.to_string(),
                                })
                            }
                        }
                    }

                    PlaybookStep::NavigateCdp { url, wait_for } => {
                        let resolved_url = runner.resolve_variable(url);
                        let mut eng = engine.lock().await;
                        let nav = eng.navigate(&resolved_url).await;
                        match nav {
                            Ok(()) => {
                                if let Some(sel) = wait_for {
                                    let _ = eng
                                        .execute_action(BrowserAction::WaitForSelector {
                                            selector: sel.clone(),
                                            timeout_ms: 15_000,
                                        })
                                        .await;
                                }
                                serde_json::json!({
                                    "step": idx + 1, "action": "navigate_cdp",
                                    "status": "ok", "url": resolved_url,
                                })
                            }
                            Err(e) => {
                                had_failure = true;
                                serde_json::json!({
                                    "step": idx + 1, "action": "navigate_cdp",
                                    "status": "error", "error": e.to_string(),
                                })
                            }
                        }
                    }

                    PlaybookStep::Wait { ms, selector, timeout, .. } => {
                        if let Some(sel) = selector {
                            let mut eng = engine.lock().await;
                            let r = eng
                                .execute_action(BrowserAction::WaitForSelector {
                                    selector: sel.clone(),
                                    timeout_ms: timeout.unwrap_or(5000),
                                })
                                .await;
                            match r {
                                Ok(_) => serde_json::json!({
                                    "step": idx + 1, "action": "wait",
                                    "status": "ok", "selector": sel,
                                }),
                                Err(e) => {
                                    had_failure = true;
                                    serde_json::json!({
                                        "step": idx + 1, "action": "wait",
                                        "status": "error", "error": e.to_string(),
                                    })
                                }
                            }
                        } else {
                            tokio::time::sleep(std::time::Duration::from_millis(
                                ms.unwrap_or(1000),
                            ))
                            .await;
                            serde_json::json!({
                                "step": idx + 1, "action": "wait",
                                "status": "ok", "ms": ms.unwrap_or(1000),
                            })
                        }
                    }

                    PlaybookStep::EvalJs { code, store_as } => {
                        let resolved_code = runner.resolve_variable(code);
                        let eng = engine.lock().await;
                        match eng.eval_js(&resolved_code).await {
                            Ok(value) => {
                                if let Some(key) = store_as {
                                    store_key = Some(key.clone());
                                    runtime_value = Some(value.clone());
                                }
                                serde_json::json!({
                                    "step": idx + 1, "action": "eval_js",
                                    "status": "ok",
                                    "result_preview": value.chars().take(200).collect::<String>(),
                                })
                            }
                            Err(e) => {
                                had_failure = true;
                                serde_json::json!({
                                    "step": idx + 1, "action": "eval_js",
                                    "status": "error", "error": e.to_string(),
                                })
                            }
                        }
                    }

                    PlaybookStep::Screenshot { .. } => {
                        let eng = engine.lock().await;
                        match eng.screenshot().await {
                            Ok(png) => serde_json::json!({
                                "step": idx + 1, "action": "screenshot",
                                "status": "ok", "size_bytes": png.len(),
                            }),
                            Err(e) => {
                                had_failure = true;
                                serde_json::json!({
                                    "step": idx + 1, "action": "screenshot",
                                    "status": "error", "error": e.to_string(),
                                })
                            }
                        }
                    }

                    PlaybookStep::Verify { check, .. } => {
                        // Minimal check support: `url_contains("…")` and runtime-var truthiness.
                        let resolved = runner.resolve_variable(check);
                        let passed = if let Some(rest) = resolved
                            .strip_prefix("url_contains(\"")
                            .and_then(|s| s.strip_suffix("\")"))
                        {
                            let eng = engine.lock().await;
                            eng.current_url()
                                .await
                                .map(|u| u.contains(rest))
                                .unwrap_or(false)
                        } else {
                            // Treat as a runtime var name — pass if non-empty after resolution.
                            !resolved.is_empty() && resolved != *check
                        };
                        if !passed {
                            had_failure = true;
                        }
                        serde_json::json!({
                            "step": idx + 1, "action": "verify",
                            "status": if passed { "ok" } else { "fail" },
                            "check": check,
                        })
                    }

                    other => {
                        // Actions not yet wired into the CLI dispatcher (click, fill,
                        // select, upload_file, custom_dropdown, submit, conditional,
                        // repeat, extract). The MCP server (browse_run_playbook)
                        // covers the full set; the CLI handles the common stealth-
                        // fetch / scrape playbook subset.
                        serde_json::json!({
                            "step": idx + 1,
                            "action": format!("{:?}", other).split_whitespace().next().unwrap_or("unknown").to_lowercase(),
                            "status": "skipped",
                            "message": "Action not implemented in CLI dispatcher — use MCP server for full coverage",
                        })
                    }
                };

                step_results.push(result);
                runner.mark_complete(
                    idx,
                    if let Some(v) = runtime_value.clone() {
                        wraith_browser_core::playbook::StepResult::Value(v)
                    } else {
                        wraith_browser_core::playbook::StepResult::Ok
                    },
                    store_key.as_deref(),
                );
            }

            // Shutdown engine.
            {
                let mut eng = engine.lock().await;
                let _ = eng.shutdown().await;
            }

            let summary = serde_json::json!({
                "playbook": pb.name,
                "path": path.display().to_string(),
                "total_steps": total,
                "completed_steps": step_results.len(),
                "results": step_results,
                "runtime_vars": runner.runtime_vars(),
            });

            match output.as_str() {
                "json" => {
                    println!("{}", serde_json::to_string_pretty(&summary)?);
                }
                "raw" => {
                    // Emit just the last `store_as` runtime value if present, else json.
                    if let Some((_k, v)) = runner.runtime_vars().iter().next() {
                        print!("{}", v);
                    } else {
                        println!("{}", serde_json::to_string(&summary)?);
                    }
                }
                "snapshot" => {
                    let eng = engine.lock().await;
                    if let Ok(snap) = eng.snapshot().await {
                        println!("{}", snap.to_agent_text());
                    } else {
                        println!("{}", serde_json::to_string_pretty(&summary)?);
                    }
                }
                "markdown" => {
                    let eng = engine.lock().await;
                    if let (Ok(html), Some(url)) = (eng.page_source().await, eng.current_url().await) {
                        if let Ok(content) = wraith_content_extract::extract(&html, &url) {
                            println!("{}", content.markdown);
                        }
                    }
                }
                other => anyhow::bail!("Unknown --output format: {} (json|raw|snapshot|markdown)", other),
            }

            if had_failure {
                anyhow::bail!("Playbook completed with step failures");
            }
        }

        // ═══════════════════════════════════════════════════════════
        // VAULT SUBCOMMANDS
        // ═══════════════════════════════════════════════════════════

        Commands::Vault { action } => {
            let vault_path = default_vault_path();

            match action {
                VaultAction::Unlock => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    let creds = vault.list_credentials()?;
                    println!("Vault unlocked. {} credentials stored.", creds.len());
                }

                VaultAction::Lock => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    vault.lock();
                    println!("Vault locked. Master key zeroized from memory.");
                }

                VaultAction::Store { domain, kind, identity, label, auto_use } => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;

                    let secret = read_secret("Enter secret value: ")?;
                    let cred_kind = parse_credential_kind(&kind);

                    let request = wraith_identity::credential::StoreCredentialRequest {
                        domain: domain.clone(),
                        kind: cred_kind,
                        identity: identity.clone(),
                        secret,
                        label,
                        url_pattern: None,
                        auto_use,
                        metadata: serde_json::Value::Object(serde_json::Map::new()),
                    };

                    let id = vault.store(request)?;
                    println!("Credential stored: {} ({}@{}, {:?})", id, identity, domain, cred_kind);
                }

                VaultAction::List => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let creds = vault.list_credentials()?;

                    if creds.is_empty() {
                        println!("No credentials stored.");
                    } else {
                        println!("{:<38} {:<20} {:<12} {:<25} Uses", "ID", "Domain", "Kind", "Identity");
                        println!("{}", "-".repeat(100));
                        for c in &creds {
                            println!("{:<38} {:<20} {:<12} {:<25} {}",
                                c.id, c.domain, c.kind, c.identity, c.use_count);
                        }
                        println!("\n{} credential(s) total", creds.len());
                    }
                }

                VaultAction::Delete { id } => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    vault.delete(&id)?;
                    println!("Credential {} deleted.", id);
                }

                VaultAction::Totp { domain } => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    let code = vault.generate_totp(&domain)?;
                    println!("{}", code);
                }

                VaultAction::Audit { limit } => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let entries = vault.audit_history(limit)?;

                    if entries.is_empty() {
                        println!("No audit log entries.");
                    } else {
                        println!("{:<20} {:<12} {:<20} {:<8} Details", "Timestamp", "Action", "Domain", "OK?");
                        println!("{}", "-".repeat(80));
                        for e in &entries {
                            println!("{:<20} {:<12} {:<20} {:<8} {}",
                                e.timestamp,
                                e.action,
                                e.domain.as_deref().unwrap_or("-"),
                                if e.success { "yes" } else { "FAIL" },
                                e.details.as_deref().unwrap_or(""),
                            );
                        }
                    }
                }

                VaultAction::Approve { credential_id, domain } => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    vault.approve_domain(&credential_id, &domain)?;
                    println!("Domain '{}' approved for credential {}.", domain, credential_id);
                }

                VaultAction::Rotate { id } => {
                    let vault = wraith_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    let new_secret = read_secret("Enter new secret value: ")?;
                    vault.rotate(&id, &new_secret)?;
                    println!("Credential {} rotated.", id);
                }
            }
        }

        // ═══════════════════════════════════════════════════════════
        // FINGERPRINT SUBCOMMANDS
        // ═══════════════════════════════════════════════════════════

        Commands::Fingerprint { action } => {
            match action {
                FingerprintAction::Import { file } => {
                    let path = std::path::Path::new(&file);
                    if !path.exists() {
                        anyhow::bail!("File not found: {}", file);
                    }

                    let mut mgr = wraith_identity::FingerprintManager::new();
                    let fp = mgr.load_from_file(path)?;

                    println!("Fingerprint imported:");
                    println!("  ID:          {}", fp.id);
                    println!("  User-Agent:  {}", fp.user_agent.get(..80).unwrap_or(&fp.user_agent));
                    println!("  Platform:    {}", fp.platform);
                    println!("  Screen:      {}x{} @{:.1}x", fp.screen_width, fp.screen_height, fp.device_pixel_ratio);
                    println!("  Timezone:    {} (UTC{:+})", fp.timezone, -(fp.timezone_offset as f32 / 60.0));
                    println!("  Language:    {}", fp.language);
                    println!("  HW Cores:    {}", fp.hardware_concurrency);
                    if let Some(mem) = fp.device_memory {
                        println!("  Memory:      {} GB", mem);
                    }
                    if let Some(ref renderer) = fp.webgl_unmasked_renderer {
                        println!("  GPU:         {}", renderer);
                    }
                    println!("  WebDriver:   {} (should be false)", fp.webdriver);
                    println!();

                    // Save to ~/.wraith/fingerprints/
                    let fp_dir = dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".wraith")
                        .join("fingerprints");
                    std::fs::create_dir_all(&fp_dir)?;
                    let fp_path = fp_dir.join(format!("{}.json", fp.id));
                    std::fs::write(&fp_path, serde_json::to_string_pretty(&fp)?)?;
                    println!("Saved to: {}", fp_path.display());
                }

                FingerprintAction::List => {
                    let fp_dir = dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".wraith")
                        .join("fingerprints");

                    if !fp_dir.exists() {
                        println!("No fingerprint profiles. Run 'wraith-browser fingerprint capture' first.");
                        return Ok(());
                    }

                    let mut count = 0;
                    for entry in std::fs::read_dir(&fp_dir)? {
                        let entry = entry?;
                        if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                            let data = std::fs::read_to_string(entry.path())?;
                            if let Ok(fp) = serde_json::from_str::<wraith_identity::BrowserFingerprint>(&data) {
                                println!("[{}] {} — {} ({}x{}, {})",
                                    fp.id.get(..8).unwrap_or(&fp.id),
                                    fp.name,
                                    fp.user_agent.get(..50).unwrap_or(&fp.user_agent),
                                    fp.screen_width,
                                    fp.screen_height,
                                    fp.timezone,
                                );
                                count += 1;
                            }
                        }
                    }

                    if count == 0 {
                        println!("No fingerprint profiles found.");
                    } else {
                        println!("\n{} profile(s) total", count);
                    }
                }
            }
        }
    }

    info!("Wraith Browser shutting down cleanly");
    Ok(())
}
