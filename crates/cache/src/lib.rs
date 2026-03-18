//! # openclaw-cache
//!
//! The persistent AI knowledge store for OpenClaw Browser.
//!
//! Every URL the agent visits, every page it extracts, every search it runs,
//! every DOM snapshot it takes — all cached, indexed, and instantly retrievable.
//! The agent never asks the web the same question twice unless the answer is stale.
//!
//! ## Storage Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     KnowledgeStore                          │
//! │                                                             │
//! │  ┌─────────────┐  ┌──────────────┐  ┌───────────────────┐  │
//! │  │   SQLite     │  │   Tantivy    │  │   Blob Store      │  │
//! │  │  (metadata)  │  │  (fulltext)  │  │  (compressed)     │  │
//! │  │             │  │              │  │                   │  │
//! │  │ • pages     │  │ • page text  │  │ • raw HTML (gzip) │  │
//! │  │ • searches  │  │ • titles     │  │ • screenshots     │  │
//! │  │ • snapshots │  │ • URLs       │  │ • large payloads  │  │
//! │  │ • sessions  │  │ • snippets   │  │                   │  │
//! │  │ • domains   │  │ • queries    │  │                   │  │
//! │  │ • ttl rules │  │              │  │                   │  │
//! │  └─────────────┘  └──────────────┘  └───────────────────┘  │
//! │                                                             │
//! │  Cache hit flow:                                            │
//! │  Agent wants URL → check SQLite → fresh? return cached      │
//! │                                   stale? fetch + update     │
//! │                                   miss?  fetch + store      │
//! │                                                             │
//! │  AI search flow:                                            │
//! │  Agent asks question → Tantivy search local knowledge       │
//! │                      → if good hits, skip web entirely      │
//! │                      → if weak/no hits, metasearch + cache  │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Staleness Model
//!
//! Not all content goes stale at the same rate. The cache uses per-domain
//! and per-content-type TTLs:
//!
//! | Content Type        | Default TTL  | Rationale                        |
//! |---------------------|-------------|----------------------------------|
//! | Static docs/wikis   | 7 days      | Reference content changes slowly |
//! | News articles       | 1 hour      | Time-sensitive                   |
//! | Search results      | 6 hours     | Rankings shift throughout day    |
//! | API docs            | 24 hours    | Versioned, semi-stable           |
//! | Social media        | 15 minutes  | Rapidly changing                 |
//! | E-commerce/pricing  | 1 hour      | Prices change frequently         |
//! | Government/legal    | 30 days     | Rarely updated                   |
//! | DOM snapshots       | 0 (session) | Only valid within agent session  |
//! | User-pinned         | ∞ (manual)  | Agent explicitly saved this      |

pub mod store;
pub mod schema;
pub mod query;
pub mod staleness;
pub mod compression;
pub mod fulltext;
pub mod error;
pub mod stats;
pub mod diffing;
pub mod site_capability;
pub mod entity_graph;
pub mod embeddings;

pub use store::KnowledgeStore;
pub use schema::{CachedPage, CachedSearch, CachedSnapshot, DomainProfile, ContentType};
pub use query::{CacheQuery, CacheResult};
pub use staleness::StalenessPolicy;
pub use error::CacheError;
