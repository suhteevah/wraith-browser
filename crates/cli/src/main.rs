use clap::{Parser, Subcommand};
use secrecy::SecretString;
use tracing::info;

#[derive(Parser)]
#[command(
    name = "openclaw-browser",
    about = "OpenClaw Browser — an AI-agent-first web browser written in Rust",
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
    /// Capture fingerprint from your real browser (opens a visible Chrome window)
    Capture {
        /// Name for the fingerprint profile
        #[arg(short, long, default_value = "My Browser")]
        name: String,
    },

    /// List stored fingerprint profiles
    List,
}

/// Get the default vault path: ~/.openclaw/vault.db
fn default_vault_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".openclaw")
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

fn parse_credential_kind(s: &str) -> openclaw_identity::CredentialKind {
    match s.to_lowercase().as_str() {
        "password" => openclaw_identity::CredentialKind::Password,
        "api_key" | "apikey" => openclaw_identity::CredentialKind::ApiKey,
        "oauth_token" | "oauth" => openclaw_identity::CredentialKind::OAuthToken,
        "totp_seed" | "totp" => openclaw_identity::CredentialKind::TotpSeed,
        "session_cookie" | "cookie" => openclaw_identity::CredentialKind::SessionCookie,
        "ssh_key" | "ssh" => openclaw_identity::CredentialKind::SshKey,
        _ => openclaw_identity::CredentialKind::Generic,
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Initialize tracing
    let filter = if cli.verbose {
        "openclaw=trace,tower_http=debug"
    } else {
        "openclaw=info,tower_http=warn"
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
        "OpenClaw Browser starting"
    );

    match cli.command {
        Commands::Serve { transport, host: _, port: _ } => {
            let transport = match transport.as_str() {
                "stdio" => openclaw_mcp_server::Transport::Stdio,
                other => anyhow::bail!("Unknown transport: {} (only 'stdio' is currently supported)", other),
            };
            openclaw_mcp_server::run(transport).await?;
        }

        Commands::Navigate { url, format } => {
            info!(url = %url, format = %format, "Navigating");
            let config = openclaw_browser_core::BrowserConfig::default();
            let session = openclaw_browser_core::BrowserSession::launch(config).await?;
            let _tab_id = session.new_tab(&url).await?;
            let tab = session.active_tab().await?;

            match format.as_str() {
                "snapshot" => {
                    let snapshot = tab.snapshot().await?;
                    println!("{}", snapshot.to_agent_text());
                }
                "markdown" => {
                    let html = tab.page_source().await?;
                    let content = openclaw_content_extract::extract(&html, &url)?;
                    println!("{}", content.markdown);
                }
                "json" => {
                    let snapshot = tab.snapshot().await?;
                    println!("{}", serde_json::to_string_pretty(&snapshot)?);
                }
                _ => anyhow::bail!("Unknown format: {}", format),
            }

            session.shutdown().await?;
        }

        Commands::Task { description, url, max_steps } => {
            info!(task = %description, max_steps, "Running autonomous task");

            // Determine LLM backend from environment
            let use_ollama = std::env::var("OPENCLAW_LLM").ok()
                .map(|v| v.to_lowercase() == "ollama")
                .unwrap_or(false);

            let config = openclaw_browser_core::BrowserConfig::default();
            let session = openclaw_browser_core::BrowserSession::launch(config).await?;

            let task = openclaw_agent_loop::BrowsingTask {
                description: description.clone(),
                start_url: url,
                timeout_secs: None,
                context: None,
            };

            let model_override = std::env::var("OPENCLAW_MODEL").ok();

            let agent_config = openclaw_agent_loop::AgentConfig {
                max_steps,
                model: model_override.clone().unwrap_or_else(|| {
                    openclaw_agent_loop::AgentConfig::default().model
                }),
                ..Default::default()
            };

            // Open knowledge store for auto-caching
            let cache_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".openclaw")
                .join("knowledge");
            let cache = openclaw_cache::KnowledgeStore::open(&cache_dir)
                .map(std::sync::Arc::new)
                .ok();

            let result = if use_ollama {
                let ollama_url = std::env::var("OLLAMA_URL")
                    .unwrap_or_else(|_| "http://localhost:11434".to_string());
                let agent_config = openclaw_agent_loop::AgentConfig {
                    model: model_override.unwrap_or_else(|| "llama3.1".to_string()),
                    ..agent_config
                };
                let backend = openclaw_agent_loop::llm::OllamaBackend::new()
                    .with_base_url(ollama_url);
                let mut agent = openclaw_agent_loop::Agent::new(agent_config, session, backend);
                if let Some(c) = cache { agent = agent.with_cache(c); }
                agent.run(task).await
            } else {
                let api_key = std::env::var("ANTHROPIC_API_KEY")
                    .or_else(|_| std::env::var("CLAUDE_API_KEY"))
                    .map_err(|_| anyhow::anyhow!(
                        "No API key found. Set ANTHROPIC_API_KEY or CLAUDE_API_KEY, \
                         or use OPENCLAW_LLM=ollama for local models."
                    ))?;
                let backend = openclaw_agent_loop::llm::ClaudeBackend::new(api_key);
                let mut agent = openclaw_agent_loop::Agent::new(agent_config, session, backend);
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
            let results = openclaw_search::search(&query, max_results).await?;
            for (i, result) in results.iter().enumerate() {
                println!("{}. {} — {}", i + 1, result.title, result.url);
                println!("   {}\n", result.snippet);
            }
            if results.is_empty() {
                println!("No results (search providers not yet implemented)");
            }
        }

        Commands::Extract { url, max_tokens } => {
            info!(url = %url, "Extracting content");
            let config = openclaw_browser_core::BrowserConfig::default();
            let session = openclaw_browser_core::BrowserSession::launch(config).await?;
            let _tab_id = session.new_tab(&url).await?;
            let tab = session.active_tab().await?;
            let html = tab.page_source().await?;
            let content = openclaw_content_extract::extract_budgeted(&html, &url, max_tokens)?;
            println!("# {}\n", content.title);
            println!("{}", content.markdown);
            println!("\n---\nTokens: ~{} | Links: {} | Confidence: {:.0}%",
                content.estimated_tokens, content.links.len(), content.confidence * 100.0);
            session.shutdown().await?;
        }

        // ═══════════════════════════════════════════════════════════
        // VAULT SUBCOMMANDS
        // ═══════════════════════════════════════════════════════════

        Commands::Vault { action } => {
            let vault_path = default_vault_path();

            match action {
                VaultAction::Unlock => {
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    let creds = vault.list_credentials()?;
                    println!("Vault unlocked. {} credentials stored.", creds.len());
                }

                VaultAction::Lock => {
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
                    vault.lock();
                    println!("Vault locked. Master key zeroized from memory.");
                }

                VaultAction::Store { domain, kind, identity, label, auto_use } => {
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;

                    let secret = read_secret("Enter secret value: ")?;
                    let cred_kind = parse_credential_kind(&kind);

                    let request = openclaw_identity::credential::StoreCredentialRequest {
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
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
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
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    vault.delete(&id)?;
                    println!("Credential {} deleted.", id);
                }

                VaultAction::Totp { domain } => {
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    let code = vault.generate_totp(&domain)?;
                    println!("{}", code);
                }

                VaultAction::Audit { limit } => {
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
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
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
                    let passphrase = read_passphrase("Enter vault passphrase: ")?;
                    vault.unlock(&passphrase)?;
                    vault.approve_domain(&credential_id, &domain)?;
                    println!("Domain '{}' approved for credential {}.", domain, credential_id);
                }

                VaultAction::Rotate { id } => {
                    let vault = openclaw_identity::CredentialVault::open(&vault_path)?;
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
                FingerprintAction::Capture { name: _ } => {
                    println!("Launching your real browser to capture fingerprint...");
                    println!("A Chrome window will open briefly — do not interact with it.");
                    println!();

                    let mut mgr = openclaw_identity::FingerprintManager::new();
                    let fp = mgr.capture_from_real_browser().await?;

                    println!("Fingerprint captured:");
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

                    // Save to ~/.openclaw/fingerprints/
                    let fp_dir = dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".openclaw")
                        .join("fingerprints");
                    std::fs::create_dir_all(&fp_dir)?;
                    let fp_path = fp_dir.join(format!("{}.json", fp.id));
                    std::fs::write(&fp_path, serde_json::to_string_pretty(&fp)?)?;
                    println!("Saved to: {}", fp_path.display());
                }

                FingerprintAction::List => {
                    let fp_dir = dirs::home_dir()
                        .unwrap_or_else(|| std::path::PathBuf::from("."))
                        .join(".openclaw")
                        .join("fingerprints");

                    if !fp_dir.exists() {
                        println!("No fingerprint profiles. Run 'openclaw-browser fingerprint capture' first.");
                        return Ok(());
                    }

                    let mut count = 0;
                    for entry in std::fs::read_dir(&fp_dir)? {
                        let entry = entry?;
                        if entry.path().extension().map(|e| e == "json").unwrap_or(false) {
                            let data = std::fs::read_to_string(entry.path())?;
                            if let Ok(fp) = serde_json::from_str::<openclaw_identity::BrowserFingerprint>(&data) {
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

    info!("OpenClaw Browser shutting down cleanly");
    Ok(())
}
