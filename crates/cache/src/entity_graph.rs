//! Cross-Site Entity Resolution
//!
//! Extracts entities from visited pages, builds a knowledge graph, and merges
//! information about the same entity observed across different sites. The graph
//! is backed by [`petgraph`] so traversal, neighbour queries and diagram
//! generation are cheap.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Graph;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// The type (category) of an extracted entity.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum EntityType {
    Person,
    Organization,
    Product,
    Location,
    Event,
    Technology,
    Unknown,
}

/// Records where an entity was discovered.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntitySource {
    /// Full URL of the page.
    pub url: String,
    /// Domain extracted from the URL.
    pub domain: String,
    /// Surrounding text where the entity was found.
    pub context: String,
    /// When the entity was observed on this page.
    pub found_at: DateTime<Utc>,
}

/// A single entity in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// Lowercased, normalised canonical name used for deduplication.
    pub canonical_name: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Category of the entity.
    pub entity_type: EntityType,
    /// Arbitrary key-value attributes (e.g. `"price" → "$99"`).
    pub attributes: HashMap<String, String>,
    /// Pages on which this entity was mentioned.
    pub sources: Vec<EntitySource>,
    /// Timestamp of the first observation.
    pub first_seen: DateTime<Utc>,
    /// Timestamp of the most recent observation.
    pub last_seen: DateTime<Utc>,
}

/// The kind of relationship between two entities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RelationKind {
    MentionedWith,
    WorksAt,
    LocatedIn,
    MadeBy,
    PartOf,
    CompetesWith,
    RelatedTo,
}

impl std::fmt::Display for RelationKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            Self::MentionedWith => "mentioned with",
            Self::WorksAt => "works at",
            Self::LocatedIn => "located in",
            Self::MadeBy => "made by",
            Self::PartOf => "part of",
            Self::CompetesWith => "competes with",
            Self::RelatedTo => "related to",
        };
        f.write_str(label)
    }
}

/// An edge in the entity graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    /// What kind of relationship this edge represents.
    pub kind: RelationKind,
    /// Confidence score in the range `[0.0, 1.0]`.
    pub confidence: f64,
}

// ---------------------------------------------------------------------------
// EntityGraph
// ---------------------------------------------------------------------------

/// A knowledge graph that stores entities and the relationships between them.
///
/// Entities are deduplicated by their [`canonical_name`](Entity::canonical_name)
/// so the same real-world thing observed on multiple sites is represented by a
/// single node.
pub struct EntityGraph {
    /// The underlying directed graph.
    graph: Graph<Entity, Relationship>,
    /// Maps canonical names to their node indices for O(1) lookup.
    entity_index: HashMap<String, NodeIndex>,
}

impl EntityGraph {
    // -- construction -------------------------------------------------------

    /// Create an empty entity graph.
    #[instrument(level = "debug")]
    pub fn new() -> Self {
        debug!("creating new EntityGraph");
        Self {
            graph: Graph::new(),
            entity_index: HashMap::new(),
        }
    }

    // -- mutation ------------------------------------------------------------

    /// Add an entity to the graph.
    ///
    /// If an entity with the same canonical name already exists the two are
    /// **merged**: attributes are combined (new values win), sources are
    /// appended, and the time window is expanded.
    #[instrument(skip(self, entity), fields(name = %entity.canonical_name))]
    pub fn add_entity(&mut self, entity: Entity) -> NodeIndex {
        let canonical = entity.canonical_name.clone();

        if let Some(&idx) = self.entity_index.get(&canonical) {
            // Merge into the existing node.
            let existing = &mut self.graph[idx];
            for (k, v) in &entity.attributes {
                existing.attributes.insert(k.clone(), v.clone());
            }
            existing.sources.extend(entity.sources);
            if entity.first_seen < existing.first_seen {
                existing.first_seen = entity.first_seen;
            }
            if entity.last_seen > existing.last_seen {
                existing.last_seen = entity.last_seen;
            }
            debug!(idx = idx.index(), "merged into existing entity");
            idx
        } else {
            let idx = self.graph.add_node(entity);
            self.entity_index.insert(canonical, idx);
            debug!(idx = idx.index(), "added new entity");
            idx
        }
    }

    /// Add a directed relationship between two entities identified by their
    /// canonical names. Both entities must already exist in the graph.
    #[instrument(skip(self))]
    pub fn add_relationship(
        &mut self,
        from: &str,
        to: &str,
        kind: RelationKind,
        confidence: f64,
    ) {
        let from_key = normalize_entity_name(from);
        let to_key = normalize_entity_name(to);

        let from_idx = match self.entity_index.get(&from_key) {
            Some(&idx) => idx,
            None => {
                debug!(name = from, "source entity not found – skipping");
                return;
            }
        };
        let to_idx = match self.entity_index.get(&to_key) {
            Some(&idx) => idx,
            None => {
                debug!(name = to, "target entity not found – skipping");
                return;
            }
        };

        self.graph
            .add_edge(from_idx, to_idx, Relationship { kind, confidence });
        info!(from = from, to = to, "relationship added");
    }

    /// Merge two entities into one.
    ///
    /// The entity identified by `name_b` is folded into `name_a`: attributes
    /// and sources are combined, all edges that pointed to/from `name_b` are
    /// redirected to `name_a`, and the `name_b` node is removed.
    #[instrument(skip(self))]
    pub fn merge_entities(&mut self, name_a: &str, name_b: &str) -> Result<(), String> {
        let key_a = normalize_entity_name(name_a);
        let key_b = normalize_entity_name(name_b);

        let &idx_a = self
            .entity_index
            .get(&key_a)
            .ok_or_else(|| format!("entity '{}' not found", name_a))?;
        let &idx_b = self
            .entity_index
            .get(&key_b)
            .ok_or_else(|| format!("entity '{}' not found", name_b))?;

        if idx_a == idx_b {
            return Ok(());
        }

        // Collect data from B.
        let b_attrs = self.graph[idx_b].attributes.clone();
        let b_sources = self.graph[idx_b].sources.clone();
        let b_first = self.graph[idx_b].first_seen;
        let b_last = self.graph[idx_b].last_seen;

        // Merge into A.
        {
            let a = &mut self.graph[idx_a];
            for (k, v) in b_attrs {
                a.attributes.entry(k).or_insert(v);
            }
            a.sources.extend(b_sources);
            if b_first < a.first_seen {
                a.first_seen = b_first;
            }
            if b_last > a.last_seen {
                a.last_seen = b_last;
            }
        }

        // Redirect edges.
        let edges: Vec<_> = self
            .graph
            .edges_directed(idx_b, petgraph::Direction::Outgoing)
            .map(|e| (e.target(), e.weight().clone()))
            .collect();
        for (target, rel) in edges {
            if target != idx_a {
                self.graph.add_edge(idx_a, target, rel);
            }
        }
        let edges: Vec<_> = self
            .graph
            .edges_directed(idx_b, petgraph::Direction::Incoming)
            .map(|e| (e.source(), e.weight().clone()))
            .collect();
        for (source, rel) in edges {
            if source != idx_a {
                self.graph.add_edge(source, idx_a, rel);
            }
        }

        // Remove the old node. petgraph may swap the last node into the
        // removed slot, so we must update the index.
        let last_idx: NodeIndex<u32> = NodeIndex::new(self.graph.node_count() - 1);
        self.graph.remove_node(idx_b);
        self.entity_index.remove(&key_b);

        // If petgraph swapped the last node into idx_b's slot, update the map.
        if idx_b.index() < last_idx.index() {
            // The node that was at `last_idx` is now at `idx_b`.
            if let Some(entity) = self.graph.node_weight(idx_b) {
                let moved_key = entity.canonical_name.clone();
                self.entity_index.insert(moved_key, idx_b);
            }
        }

        info!(kept = name_a, removed = name_b, "entities merged");
        Ok(())
    }

    // -- queries -------------------------------------------------------------

    /// Look up an entity by its canonical name.
    #[instrument(skip(self))]
    pub fn get_entity(&self, name: &str) -> Option<&Entity> {
        let key = normalize_entity_name(name);
        self.entity_index
            .get(&key)
            .map(|&idx| &self.graph[idx])
    }

    /// Return all direct neighbours of an entity together with the connecting
    /// relationship.
    #[instrument(skip(self))]
    pub fn find_related(&self, name: &str) -> Vec<(&Entity, &Relationship)> {
        let key = normalize_entity_name(name);
        let idx = match self.entity_index.get(&key) {
            Some(&i) => i,
            None => return Vec::new(),
        };

        self.graph
            .edges(idx)
            .map(|edge| {
                let neighbour = if edge.source() == idx {
                    edge.target()
                } else {
                    edge.source()
                };
                (&self.graph[neighbour], edge.weight())
            })
            .collect()
    }

    /// Number of entities in the graph.
    pub fn entity_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Number of relationships (edges) in the graph.
    pub fn relationship_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Return all entities that match a given [`EntityType`].
    #[instrument(skip(self))]
    pub fn entities_by_type(&self, entity_type: EntityType) -> Vec<&Entity> {
        self.graph
            .node_weights()
            .filter(|e| e.entity_type == entity_type)
            .collect()
    }

    /// Fuzzy search for entities whose canonical or display name contains the
    /// query (case-insensitive substring match).
    #[instrument(skip(self))]
    pub fn search_entities(&self, query: &str) -> Vec<&Entity> {
        let q = query.to_lowercase();
        self.graph
            .node_weights()
            .filter(|e| {
                e.canonical_name.contains(&q) || e.display_name.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Answer a simple natural-language question about the graph.
    ///
    /// Recognises patterns like *"tell me everything about X"* and
    /// *"what do you know about X"*. Falls back to a fuzzy entity search.
    #[instrument(skip(self))]
    pub fn query(&self, question: &str) -> String {
        let q = question.to_lowercase();

        // Try to extract the subject from common patterns.
        let subject = extract_query_subject(&q);

        if let Some(subject) = subject {
            if let Some(entity) = self.get_entity(&subject) {
                return self.describe_entity(entity);
            }
            // Fall back to fuzzy search.
            let results = self.search_entities(&subject);
            if results.is_empty() {
                return format!("No entity found matching '{}'.", subject);
            }
            return results
                .iter()
                .map(|e| self.describe_entity(e))
                .collect::<Vec<_>>()
                .join("\n---\n");
        }

        format!(
            "The graph contains {} entities and {} relationships.",
            self.entity_count(),
            self.relationship_count()
        )
    }

    /// Generate a [Mermaid](https://mermaid.js.org/) graph diagram of the
    /// entire entity graph.
    #[instrument(skip(self))]
    pub fn to_mermaid(&self) -> String {
        let mut out = String::from("graph LR\n");

        for idx in self.graph.node_indices() {
            let entity = &self.graph[idx];
            let label = entity.display_name.replace('"', "'");
            out.push_str(&format!(
                "    {}[\"{} ({:?})\"]\n",
                idx.index(),
                label,
                entity.entity_type
            ));
        }

        for edge in self.graph.edge_references() {
            out.push_str(&format!(
                "    {} -->|\"{}\"| {}\n",
                edge.source().index(),
                edge.weight().kind,
                edge.target().index()
            ));
        }

        out
    }

    // -- helpers (private) ---------------------------------------------------

    fn describe_entity(&self, entity: &Entity) -> String {
        let mut parts: Vec<String> = Vec::new();

        parts.push(format!(
            "{} ({:?})",
            entity.display_name, entity.entity_type
        ));

        if !entity.attributes.is_empty() {
            let attrs: Vec<String> = entity
                .attributes
                .iter()
                .map(|(k, v)| format!("  {}: {}", k, v))
                .collect();
            parts.push(format!("Attributes:\n{}", attrs.join("\n")));
        }

        if !entity.sources.is_empty() {
            let sources: Vec<String> = entity
                .sources
                .iter()
                .map(|s| format!("  {} ({})", s.url, s.domain))
                .collect();
            parts.push(format!("Sources:\n{}", sources.join("\n")));
        }

        let related = self.find_related(&entity.canonical_name);
        if !related.is_empty() {
            let rels: Vec<String> = related
                .iter()
                .map(|(e, r)| format!("  {} {} (confidence {:.2})", r.kind, e.display_name, r.confidence))
                .collect();
            parts.push(format!("Related:\n{}", rels.join("\n")));
        }

        parts.join("\n")
    }
}

impl Default for EntityGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Normalise an entity name for deduplication.
///
/// Lowercases, trims whitespace, collapses runs of whitespace to a single
/// space, and strips common corporate suffixes (Inc, LLC, Ltd, Corp).
pub fn normalize_entity_name(name: &str) -> String {
    let name = name.trim().to_lowercase();
    // Collapse whitespace.
    let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
    // Strip common suffixes.
    let suffixes = [
        " inc.", " inc", " llc", " ltd.", " ltd", " corp.", " corp",
        " co.", " co", " gmbh", " plc",
    ];
    let mut result = name;
    for suffix in &suffixes {
        if let Some(stripped) = result.strip_suffix(suffix) {
            result = stripped.to_string();
            break; // only strip once
        }
    }
    result
}

/// Heuristic entity extraction from raw page text.
///
/// This is intentionally simple — it looks for:
/// - Capitalised word sequences (2–4 words) → Person or Organization
/// - `$` followed by digits → Product with a `price` attribute
/// - Email addresses → Person
/// - `@handles` → Person
/// - Known technology keywords → Technology
#[instrument(skip(text))]
pub fn extract_entities_from_text(text: &str, url: &str) -> Vec<Entity> {
    let now = Utc::now();
    let domain = url_domain(url);
    let mut entities: Vec<Entity> = Vec::new();
    let mut seen_canonical: HashMap<String, usize> = HashMap::new();

    let known_tech: &[&str] = &[
        "Python", "Rust", "JavaScript", "TypeScript", "React", "Vue", "Angular",
        "AWS", "Azure", "Docker", "Kubernetes", "Linux", "PostgreSQL", "Redis",
        "Node.js", "Go", "Java", "Swift", "Kotlin", "GraphQL", "WebAssembly",
    ];

    // -- technology keywords ------------------------------------------------
    for &tech in known_tech {
        if text.contains(tech) {
            let canonical = normalize_entity_name(tech);
            if seen_canonical.contains_key(&canonical) {
                continue;
            }
            let entity = make_entity(
                tech,
                EntityType::Technology,
                HashMap::new(),
                url,
                &domain,
                tech,
                now,
            );
            seen_canonical.insert(canonical, entities.len());
            entities.push(entity);
        }
    }

    // -- price patterns ($NNN) ----------------------------------------------
    for word in text.split_whitespace() {
        if word.starts_with('$') && word.len() > 1 {
            let price_part = &word[1..];
            if price_part.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                let mut attrs = HashMap::new();
                attrs.insert("price".to_string(), word.to_string());
                let canonical = normalize_entity_name(word);
                if let std::collections::hash_map::Entry::Vacant(e) = seen_canonical.entry(canonical) {
                    let entity = make_entity(
                        word,
                        EntityType::Product,
                        attrs,
                        url,
                        &domain,
                        word,
                        now,
                    );
                    e.insert(entities.len());
                    entities.push(entity);
                }
            }
        }
    }

    // -- email addresses ----------------------------------------------------
    for word in text.split_whitespace() {
        let clean = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '.' && c != '_' && c != '-');
        if clean.contains('@') && clean.contains('.') && clean.len() > 5 {
            let canonical = normalize_entity_name(clean);
            if let std::collections::hash_map::Entry::Vacant(e) = seen_canonical.entry(canonical) {
                let mut attrs = HashMap::new();
                attrs.insert("email".to_string(), clean.to_string());
                let entity = make_entity(
                    clean,
                    EntityType::Person,
                    attrs,
                    url,
                    &domain,
                    clean,
                    now,
                );
                e.insert(entities.len());
                entities.push(entity);
            }
        }
    }

    // -- @handles -----------------------------------------------------------
    for word in text.split_whitespace() {
        if word.starts_with('@') && word.len() > 2 {
            let handle = word.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '_');
            let canonical = normalize_entity_name(handle);
            if let std::collections::hash_map::Entry::Vacant(e) = seen_canonical.entry(canonical) {
                let mut attrs = HashMap::new();
                attrs.insert("handle".to_string(), handle.to_string());
                let entity = make_entity(
                    handle,
                    EntityType::Person,
                    attrs,
                    url,
                    &domain,
                    handle,
                    now,
                );
                e.insert(entities.len());
                entities.push(entity);
            }
        }
    }

    // -- capitalised word sequences (2–4 words) -----------------------------
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut i = 0;
    while i < words.len() {
        // Skip short words or those starting lowercase.
        let first_char = words[i].chars().next().unwrap_or('a');
        if !first_char.is_uppercase() {
            i += 1;
            continue;
        }

        // Collect consecutive capitalised words (up to 4).
        let start = i;
        let mut end = i + 1;
        while end < words.len()
            && end - start < 4
            && words[end]
                .chars()
                .next()
                .is_some_and(|c| c.is_uppercase())
        {
            end += 1;
        }

        let span_len = end - start;
        if span_len >= 2 {
            let name: String = words[start..end].join(" ");
            // Strip trailing punctuation.
            let name = name.trim_end_matches(|c: char| c.is_ascii_punctuation());
            if !name.is_empty() {
                let canonical = normalize_entity_name(name);
                if let std::collections::hash_map::Entry::Vacant(e) = seen_canonical.entry(canonical) {
                    let entity = make_entity(
                        name,
                        EntityType::Organization, // default for capitalised sequences
                        HashMap::new(),
                        url,
                        &domain,
                        name,
                        now,
                    );
                    e.insert(entities.len());
                    entities.push(entity);
                }
            }
            i = end;
        } else {
            i += 1;
        }
    }

    debug!(count = entities.len(), "entities extracted from text");
    entities
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build an [`Entity`] with sensible defaults.
fn make_entity(
    display: &str,
    entity_type: EntityType,
    attributes: HashMap<String, String>,
    url: &str,
    domain: &str,
    context: &str,
    now: DateTime<Utc>,
) -> Entity {
    Entity {
        id: Uuid::new_v4().to_string(),
        canonical_name: normalize_entity_name(display),
        display_name: display.to_string(),
        entity_type,
        attributes,
        sources: vec![EntitySource {
            url: url.to_string(),
            domain: domain.to_string(),
            context: context.to_string(),
            found_at: now,
        }],
        first_seen: now,
        last_seen: now,
    }
}

/// Extract the domain from a URL (best-effort, no external crate).
fn url_domain(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string()
}

/// Try to extract the subject entity name from a natural-language question.
fn extract_query_subject(q: &str) -> Option<String> {
    let patterns: &[&str] = &[
        "tell me everything about ",
        "tell me about ",
        "what do you know about ",
        "what is ",
        "who is ",
        "describe ",
        "info on ",
        "about ",
    ];

    for pat in patterns {
        if let Some(rest) = q.strip_prefix(pat) {
            let subject = rest.trim_end_matches(['?', '.']);
            if !subject.is_empty() {
                return Some(subject.to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_entity(name: &str, etype: EntityType) -> Entity {
        let now = Utc::now();
        Entity {
            id: Uuid::new_v4().to_string(),
            canonical_name: normalize_entity_name(name),
            display_name: name.to_string(),
            entity_type: etype,
            attributes: HashMap::new(),
            sources: vec![EntitySource {
                url: "https://example.com".to_string(),
                domain: "example.com".to_string(),
                context: "test context".to_string(),
                found_at: now,
            }],
            first_seen: now,
            last_seen: now,
        }
    }

    #[test]
    fn test_add_and_get_entity() {
        let mut g = EntityGraph::new();
        let e = make_test_entity("Acme Corp", EntityType::Organization);
        g.add_entity(e);

        let found = g.get_entity("acme corp").unwrap();
        assert_eq!(found.display_name, "Acme Corp");
    }

    #[test]
    fn test_merge_entities_combines_attributes() {
        let mut g = EntityGraph::new();

        let mut e1 = make_test_entity("Acme", EntityType::Organization);
        e1.attributes.insert("founded".to_string(), "2020".to_string());
        g.add_entity(e1);

        let mut e2 = make_test_entity("Acme Global", EntityType::Organization);
        e2.attributes.insert("ceo".to_string(), "Alice".to_string());
        g.add_entity(e2);

        // Both should exist separately before merge.
        assert_eq!(g.entity_count(), 2);

        g.merge_entities("acme", "acme global").unwrap();
        assert_eq!(g.entity_count(), 1);

        let merged = g.get_entity("acme").unwrap();
        assert_eq!(merged.attributes.get("founded").unwrap(), "2020");
        assert_eq!(merged.attributes.get("ceo").unwrap(), "Alice");
    }

    #[test]
    fn test_add_relationship_and_find_related() {
        let mut g = EntityGraph::new();
        g.add_entity(make_test_entity("Alice Smith", EntityType::Person));
        g.add_entity(make_test_entity("Acme Corp", EntityType::Organization));

        g.add_relationship("Alice Smith", "Acme Corp", RelationKind::WorksAt, 0.9);

        let related = g.find_related("Alice Smith");
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].0.display_name, "Acme Corp");
        assert_eq!(related[0].1.kind, RelationKind::WorksAt);
    }

    #[test]
    fn test_entities_by_type() {
        let mut g = EntityGraph::new();
        g.add_entity(make_test_entity("Alice", EntityType::Person));
        g.add_entity(make_test_entity("Bob", EntityType::Person));
        g.add_entity(make_test_entity("Acme", EntityType::Organization));

        let people = g.entities_by_type(EntityType::Person);
        assert_eq!(people.len(), 2);

        let orgs = g.entities_by_type(EntityType::Organization);
        assert_eq!(orgs.len(), 1);
    }

    #[test]
    fn test_search_entities_fuzzy() {
        let mut g = EntityGraph::new();
        g.add_entity(make_test_entity("John Doe", EntityType::Person));
        g.add_entity(make_test_entity("Jane Doe", EntityType::Person));
        g.add_entity(make_test_entity("Acme Corp", EntityType::Organization));

        let results = g.search_entities("doe");
        assert_eq!(results.len(), 2);

        let results = g.search_entities("acme");
        assert_eq!(results.len(), 1);

        let results = g.search_entities("xyz");
        assert!(results.is_empty());
    }

    #[test]
    fn test_normalize_entity_name_strips_suffixes() {
        assert_eq!(normalize_entity_name("Acme Inc."), "acme");
        assert_eq!(normalize_entity_name("Widgets LLC"), "widgets");
        assert_eq!(normalize_entity_name("BigCo Ltd"), "bigco");
        assert_eq!(normalize_entity_name("  Foo  Corp  "), "foo");
        assert_eq!(normalize_entity_name("Hello World"), "hello world");
    }

    #[test]
    fn test_extract_entities_finds_capitalised_names() {
        let text = "We spoke with John Smith and Alice Johnson about the project.";
        let entities = extract_entities_from_text(text, "https://example.com/article");

        let names: Vec<&str> = entities.iter().map(|e| e.display_name.as_str()).collect();
        assert!(names.iter().any(|n| n.contains("John Smith")));
        assert!(names.iter().any(|n| n.contains("Alice Johnson")));
    }

    #[test]
    fn test_extract_entities_finds_technology() {
        let text = "The service is written in Rust and uses React on the frontend.";
        let entities = extract_entities_from_text(text, "https://example.com");

        let names: Vec<&str> = entities.iter().map(|e| e.canonical_name.as_str()).collect();
        assert!(names.contains(&"rust"));
        assert!(names.contains(&"react"));
    }

    #[test]
    fn test_entity_count_and_relationship_count() {
        let mut g = EntityGraph::new();
        assert_eq!(g.entity_count(), 0);
        assert_eq!(g.relationship_count(), 0);

        g.add_entity(make_test_entity("A", EntityType::Unknown));
        g.add_entity(make_test_entity("B", EntityType::Unknown));
        assert_eq!(g.entity_count(), 2);

        g.add_relationship("A", "B", RelationKind::RelatedTo, 0.5);
        assert_eq!(g.relationship_count(), 1);
    }

    #[test]
    fn test_query_produces_meaningful_output() {
        let mut g = EntityGraph::new();
        let mut e = make_test_entity("Rust Language", EntityType::Technology);
        e.attributes.insert("type".to_string(), "systems programming".to_string());
        g.add_entity(e);

        let answer = g.query("tell me everything about rust language");
        assert!(answer.contains("Rust Language"));
        assert!(answer.contains("systems programming"));

        // Unknown entity.
        let answer = g.query("tell me about nothing");
        assert!(answer.contains("No entity found"));

        // No subject pattern.
        let answer = g.query("hello");
        assert!(answer.contains("1 entities"));
    }

    #[test]
    fn test_to_mermaid() {
        let mut g = EntityGraph::new();
        g.add_entity(make_test_entity("Alice", EntityType::Person));
        g.add_entity(make_test_entity("Acme", EntityType::Organization));
        g.add_relationship("Alice", "Acme", RelationKind::WorksAt, 0.95);

        let diagram = g.to_mermaid();
        assert!(diagram.starts_with("graph LR"));
        assert!(diagram.contains("Alice"));
        assert!(diagram.contains("Acme"));
        assert!(diagram.contains("works at"));
    }
}
