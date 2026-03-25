//! # Network Intelligence / API Discovery
//!
//! Captures XHR/Fetch traffic via CDP and reverse-engineers underlying APIs.
//! The module records network requests, identifies API calls (JSON, GraphQL, XML),
//! and infers endpoint templates, authentication patterns, and JSON schemas from
//! observed traffic.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use wraith_browser_core::network_intel::{NetworkCapture, ApiRequest};
//!
//! let mut capture = NetworkCapture::new();
//! // ... record requests as they come in via CDP event listeners ...
//! let endpoints = capture.discover_endpoints();
//! println!("{}", capture.summary());
//! ```

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, instrument};
use url::Url;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single captured network request/response pair.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiRequest {
    /// The full request URL.
    pub url: String,
    /// HTTP method (GET, POST, PUT, DELETE, etc.).
    pub method: String,
    /// Request headers as key-value pairs.
    pub request_headers: Vec<(String, String)>,
    /// Raw request body, if present.
    pub request_body: Option<String>,
    /// HTTP response status code.
    pub response_status: u16,
    /// Response headers as key-value pairs.
    pub response_headers: Vec<(String, String)>,
    /// Raw response body, if captured.
    pub response_body: Option<String>,
    /// Content-Type of the response.
    pub content_type: Option<String>,
    /// Timestamp when the request was recorded.
    pub timestamp: DateTime<Utc>,
    /// `true` if the response is JSON, GraphQL, or XML — i.e. likely an API call.
    pub is_api: bool,
    /// `true` if the request targets a GraphQL endpoint.
    pub is_graphql: bool,
}

/// A discovered API endpoint inferred from captured traffic.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ApiEndpoint {
    /// Templatized URL path (e.g. `/api/users/{id}`).
    pub url_template: String,
    /// HTTP method.
    pub method: String,
    /// Number of times this endpoint was observed.
    pub seen_count: u32,
    /// Detected authentication mechanism.
    pub auth_type: AuthType,
    /// JSON schema inferred from request bodies, if available.
    pub request_schema: Option<serde_json::Value>,
    /// JSON schema inferred from response bodies, if available.
    pub response_schema: Option<serde_json::Value>,
    /// One concrete URL that matched this template — useful as a curl example.
    pub example_url: String,
}

/// Authentication type detected from request headers.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub enum AuthType {
    /// No authentication detected.
    None,
    /// Bearer token in `Authorization` header.
    Bearer,
    /// Session cookie present.
    Cookie,
    /// API key header (`x-api-key` / `api-key`).
    ApiKey,
    /// HTTP Basic authentication.
    Basic,
}

// ---------------------------------------------------------------------------
// NetworkCapture
// ---------------------------------------------------------------------------

/// Collects network requests and provides API-discovery analysis.
pub struct NetworkCapture {
    /// All captured requests, regardless of type.
    requests: Vec<ApiRequest>,
}

impl NetworkCapture {
    /// Create an empty capture store.
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
        }
    }

    /// Record a captured request/response.
    #[instrument(skip(self, request), fields(url = %request.url, method = %request.method))]
    pub fn record(&mut self, request: ApiRequest) {
        debug!(
            url = %request.url,
            method = %request.method,
            status = request.response_status,
            is_api = request.is_api,
            "Recording network request"
        );
        self.requests.push(request);
    }

    /// Return only the captured entries that are API calls.
    #[instrument(skip(self))]
    pub fn api_requests(&self) -> Vec<&ApiRequest> {
        self.requests.iter().filter(|r| r.is_api).collect()
    }

    /// Analyse all captured API requests and discover endpoint templates.
    ///
    /// Groups requests by (method, url_template), infers auth types and JSON
    /// schemas, and returns one [`ApiEndpoint`] per group.
    #[instrument(skip(self))]
    pub fn discover_endpoints(&self) -> Vec<ApiEndpoint> {
        let api_reqs: Vec<&ApiRequest> = self.requests.iter().filter(|r| r.is_api).collect();

        // Group by (method, template)
        let mut groups: HashMap<(String, String), Vec<&ApiRequest>> = HashMap::new();
        for req in &api_reqs {
            let template = infer_url_template(&req.url);
            let key = (req.method.clone(), template);
            groups.entry(key).or_default().push(req);
        }

        let mut endpoints: Vec<ApiEndpoint> = groups
            .into_iter()
            .map(|((method, url_template), reqs)| {
                let seen_count = reqs.len() as u32;
                let example_url = reqs[0].url.clone();

                // Auth: use the first request that has auth headers
                let auth_type = reqs
                    .iter()
                    .map(|r| detect_auth_type(&r.request_headers))
                    .find(|a| *a != AuthType::None)
                    .unwrap_or(AuthType::None);

                // Infer request schema from the first request with a body
                let request_schema = reqs
                    .iter()
                    .filter_map(|r| r.request_body.as_ref())
                    .find_map(|body| serde_json::from_str::<serde_json::Value>(body).ok())
                    .map(|v| infer_json_schema(&v));

                // Infer response schema from the first response with a body
                let response_schema = reqs
                    .iter()
                    .filter_map(|r| r.response_body.as_ref())
                    .find_map(|body| serde_json::from_str::<serde_json::Value>(body).ok())
                    .map(|v| infer_json_schema(&v));

                ApiEndpoint {
                    url_template,
                    method,
                    seen_count,
                    auth_type,
                    request_schema,
                    response_schema,
                    example_url,
                }
            })
            .collect();

        endpoints.sort_by(|a, b| b.seen_count.cmp(&a.seen_count));

        info!(
            endpoint_count = endpoints.len(),
            "Discovered API endpoints"
        );

        endpoints
    }

    /// Return all requests that target GraphQL endpoints.
    #[instrument(skip(self))]
    pub fn detect_graphql(&self) -> Vec<&ApiRequest> {
        self.requests.iter().filter(|r| r.is_graphql).collect()
    }

    /// Return a human-readable summary of all discovered APIs.
    #[instrument(skip(self))]
    pub fn summary(&self) -> String {
        let total = self.requests.len();
        let api_count = self.requests.iter().filter(|r| r.is_api).count();
        let graphql_count = self.requests.iter().filter(|r| r.is_graphql).count();
        let endpoints = self.discover_endpoints();

        let mut lines = Vec::new();
        lines.push("=== Network Intelligence Summary ===".to_string());
        lines.push(format!(
            "Total requests: {} | API calls: {} | GraphQL: {}",
            total, api_count, graphql_count
        ));
        lines.push(format!("Discovered {} endpoint(s):", endpoints.len()));
        lines.push(String::new());

        for ep in &endpoints {
            lines.push(format!(
                "  {} {} (seen {}x, auth: {:?})",
                ep.method, ep.url_template, ep.seen_count, ep.auth_type
            ));
            if let Some(ref schema) = ep.request_schema {
                lines.push(format!("    request schema:  {}", schema));
            }
            if let Some(ref schema) = ep.response_schema {
                lines.push(format!("    response schema: {}", schema));
            }
            lines.push(format!("    example: {}", ep.example_url));
        }

        let summary = lines.join("\n");
        info!(endpoints = endpoints.len(), "Generated network summary");
        summary
    }
}

impl Default for NetworkCapture {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Infer a URL template from a concrete URL.
///
/// Replaces dynamic path segments so that requests to the same logical
/// endpoint are grouped together:
/// - Numeric segments → `{id}`
/// - UUID-shaped segments → `{uuid}`
/// - Long hex strings (32+ chars) → `{hash}`
pub fn infer_url_template(raw_url: &str) -> String {
    let path = match Url::parse(raw_url) {
        Ok(parsed) => parsed.path().to_string(),
        Err(_) => {
            // Might be a path-only string
            if let Some(idx) = raw_url.find('?') {
                raw_url[..idx].to_string()
            } else {
                raw_url.to_string()
            }
        }
    };

    // Strip query string if still present
    let path = if let Some(idx) = path.find('?') {
        &path[..idx]
    } else {
        &path
    };

    let uuid_re = Regex::new(
        r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$",
    )
    .expect("uuid regex is valid");

    let hex_re = Regex::new(r"^[0-9a-fA-F]{32,}$").expect("hex regex is valid");

    let segments: Vec<&str> = path.split('/').collect();
    let replaced: Vec<String> = segments
        .iter()
        .map(|seg| {
            if seg.is_empty() {
                return String::new();
            }
            if seg.parse::<u64>().is_ok() {
                return "{id}".to_string();
            }
            if uuid_re.is_match(seg) {
                return "{uuid}".to_string();
            }
            if hex_re.is_match(seg) {
                return "{hash}".to_string();
            }
            seg.to_string()
        })
        .collect();

    replaced.join("/")
}

/// Infer a simple JSON schema from a [`serde_json::Value`].
///
/// - Objects: each key maps to its type name (`"string"`, `"number"`, etc.).
///   Nested objects are expanded one level deep.
/// - Arrays: the schema contains an `"items"` key inferred from the first element.
/// - Scalars: returns the type name directly.
pub fn infer_json_schema(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut schema = serde_json::Map::new();
            for (key, val) in map {
                let type_desc = match val {
                    serde_json::Value::Null => serde_json::Value::String("null".into()),
                    serde_json::Value::Bool(_) => serde_json::Value::String("boolean".into()),
                    serde_json::Value::Number(_) => serde_json::Value::String("number".into()),
                    serde_json::Value::String(_) => serde_json::Value::String("string".into()),
                    serde_json::Value::Array(arr) => {
                        let mut inner = serde_json::Map::new();
                        inner.insert(
                            "type".into(),
                            serde_json::Value::String("array".into()),
                        );
                        if let Some(first) = arr.first() {
                            inner.insert("items".into(), scalar_type_name(first));
                        }
                        serde_json::Value::Object(inner)
                    }
                    serde_json::Value::Object(nested) => {
                        // One level deep: map nested keys to scalar type names
                        let mut nested_schema = serde_json::Map::new();
                        for (nk, nv) in nested {
                            nested_schema.insert(nk.clone(), scalar_type_name(nv));
                        }
                        serde_json::Value::Object(nested_schema)
                    }
                };
                schema.insert(key.clone(), type_desc);
            }
            serde_json::Value::Object(schema)
        }
        serde_json::Value::Array(arr) => {
            let mut schema = serde_json::Map::new();
            schema.insert("type".into(), serde_json::Value::String("array".into()));
            if let Some(first) = arr.first() {
                schema.insert("items".into(), infer_json_schema(first));
            }
            serde_json::Value::Object(schema)
        }
        other => scalar_type_name(other),
    }
}

/// Return the JSON-schema-style type name for a value without recursing.
fn scalar_type_name(value: &serde_json::Value) -> serde_json::Value {
    let name = match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    };
    serde_json::Value::String(name.into())
}

/// Detect the authentication type from a set of request headers.
///
/// Checks headers in priority order: Bearer → Basic → ApiKey → Cookie → None.
pub fn detect_auth_type(headers: &[(String, String)]) -> AuthType {
    for (name, value) in headers {
        let lower = name.to_lowercase();
        if lower == "authorization" {
            let val_lower = value.to_lowercase();
            if val_lower.starts_with("bearer ") {
                return AuthType::Bearer;
            }
            if val_lower.starts_with("basic ") {
                return AuthType::Basic;
            }
        }
        if lower == "x-api-key" || lower == "api-key" {
            return AuthType::ApiKey;
        }
    }

    // Cookie check last — many API calls carry cookies even when using token auth
    for (name, _) in headers {
        if name.to_lowercase() == "cookie" {
            return AuthType::Cookie;
        }
    }

    AuthType::None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    /// Build a minimal [`ApiRequest`] for testing.
    fn make_request(
        url: &str,
        method: &str,
        is_api: bool,
        headers: Vec<(&str, &str)>,
        request_body: Option<&str>,
        response_body: Option<&str>,
    ) -> ApiRequest {
        ApiRequest {
            url: url.to_string(),
            method: method.to_string(),
            request_headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            request_body: request_body.map(|s| s.to_string()),
            response_status: 200,
            response_headers: vec![],
            response_body: response_body.map(|s| s.to_string()),
            content_type: Some("application/json".to_string()),
            timestamp: Utc::now(),
            is_api,
            is_graphql: url.contains("/graphql"),
        }
    }

    // -- infer_url_template ------------------------------------------------

    #[test]
    fn test_template_numeric_id() {
        assert_eq!(
            infer_url_template("https://api.example.com/users/42"),
            "/users/{id}"
        );
    }

    #[test]
    fn test_template_uuid() {
        assert_eq!(
            infer_url_template(
                "https://api.example.com/items/550e8400-e29b-41d4-a716-446655440000"
            ),
            "/items/{uuid}"
        );
    }

    #[test]
    fn test_template_hex_hash() {
        let hash = "a".repeat(64);
        let url = format!("https://cdn.example.com/assets/{}", hash);
        assert_eq!(infer_url_template(&url), "/assets/{hash}");
    }

    #[test]
    fn test_template_preserves_literal_segments() {
        assert_eq!(
            infer_url_template("https://api.example.com/v2/users/search"),
            "/v2/users/search"
        );
    }

    #[test]
    fn test_template_mixed() {
        assert_eq!(
            infer_url_template("https://api.example.com/orgs/123/repos/456/commits"),
            "/orgs/{id}/repos/{id}/commits"
        );
    }

    #[test]
    fn test_template_strips_query_string() {
        assert_eq!(
            infer_url_template("https://api.example.com/users/99?fields=name"),
            "/users/{id}"
        );
    }

    // -- detect_auth_type --------------------------------------------------

    #[test]
    fn test_auth_bearer() {
        let headers = vec![("Authorization".into(), "Bearer tok_abc".into())];
        assert_eq!(detect_auth_type(&headers), AuthType::Bearer);
    }

    #[test]
    fn test_auth_basic() {
        let headers = vec![("Authorization".into(), "Basic dXNlcjpwYXNz".into())];
        assert_eq!(detect_auth_type(&headers), AuthType::Basic);
    }

    #[test]
    fn test_auth_api_key() {
        let headers = vec![("x-api-key".into(), "sk-12345".into())];
        assert_eq!(detect_auth_type(&headers), AuthType::ApiKey);
    }

    #[test]
    fn test_auth_api_key_alternate_header() {
        let headers = vec![("Api-Key".into(), "sk-12345".into())];
        assert_eq!(detect_auth_type(&headers), AuthType::ApiKey);
    }

    #[test]
    fn test_auth_cookie() {
        let headers = vec![("Cookie".into(), "session=abc123".into())];
        assert_eq!(detect_auth_type(&headers), AuthType::Cookie);
    }

    #[test]
    fn test_auth_none() {
        let headers: Vec<(String, String)> = vec![];
        assert_eq!(detect_auth_type(&headers), AuthType::None);
    }

    #[test]
    fn test_auth_bearer_takes_priority_over_cookie() {
        let headers = vec![
            ("Authorization".into(), "Bearer tok".into()),
            ("Cookie".into(), "sess=1".into()),
        ];
        assert_eq!(detect_auth_type(&headers), AuthType::Bearer);
    }

    // -- infer_json_schema -------------------------------------------------

    #[test]
    fn test_schema_flat_object() {
        let val = json!({"name": "Alice", "age": 30, "active": true});
        let schema = infer_json_schema(&val);
        assert_eq!(schema["name"], "string");
        assert_eq!(schema["age"], "number");
        assert_eq!(schema["active"], "boolean");
    }

    #[test]
    fn test_schema_nested_object() {
        let val = json!({"user": {"id": 1, "email": "a@b.com"}});
        let schema = infer_json_schema(&val);
        // Nested object is expanded one level
        assert_eq!(schema["user"]["id"], "number");
        assert_eq!(schema["user"]["email"], "string");
    }

    #[test]
    fn test_schema_array_top_level() {
        let val = json!([{"id": 1}, {"id": 2}]);
        let schema = infer_json_schema(&val);
        assert_eq!(schema["type"], "array");
        // items should be the schema of the first element
        assert_eq!(schema["items"]["id"], "number");
    }

    #[test]
    fn test_schema_null_field() {
        let val = json!({"deleted_at": null});
        let schema = infer_json_schema(&val);
        assert_eq!(schema["deleted_at"], "null");
    }

    #[test]
    fn test_schema_array_field() {
        let val = json!({"tags": ["a", "b"]});
        let schema = infer_json_schema(&val);
        assert_eq!(schema["tags"]["type"], "array");
        assert_eq!(schema["tags"]["items"], "string");
    }

    // -- discover_endpoints ------------------------------------------------

    #[test]
    fn test_discover_groups_by_template() {
        let mut capture = NetworkCapture::new();

        // Two requests to the same logical endpoint with different IDs
        capture.record(make_request(
            "https://api.example.com/users/1",
            "GET",
            true,
            vec![("Authorization", "Bearer tok")],
            None,
            Some(r#"{"id":1,"name":"Alice"}"#),
        ));
        capture.record(make_request(
            "https://api.example.com/users/2",
            "GET",
            true,
            vec![("Authorization", "Bearer tok")],
            None,
            Some(r#"{"id":2,"name":"Bob"}"#),
        ));

        // One request to a different endpoint
        capture.record(make_request(
            "https://api.example.com/posts",
            "POST",
            true,
            vec![("x-api-key", "key123")],
            Some(r#"{"title":"Hello"}"#),
            Some(r#"{"id":10,"title":"Hello"}"#),
        ));

        // One non-API request (should be excluded)
        capture.record(make_request(
            "https://cdn.example.com/style.css",
            "GET",
            false,
            vec![],
            None,
            None,
        ));

        let endpoints = capture.discover_endpoints();
        assert_eq!(endpoints.len(), 2, "should discover 2 endpoint groups");

        // The /users/{id} group should have seen_count == 2
        let users_ep = endpoints
            .iter()
            .find(|e| e.url_template == "/users/{id}")
            .expect("should find /users/{id}");
        assert_eq!(users_ep.seen_count, 2);
        assert_eq!(users_ep.method, "GET");
        assert_eq!(users_ep.auth_type, AuthType::Bearer);
        assert!(users_ep.response_schema.is_some());

        // The /posts group
        let posts_ep = endpoints
            .iter()
            .find(|e| e.url_template == "/posts")
            .expect("should find /posts");
        assert_eq!(posts_ep.seen_count, 1);
        assert_eq!(posts_ep.method, "POST");
        assert_eq!(posts_ep.auth_type, AuthType::ApiKey);
        assert!(posts_ep.request_schema.is_some());
    }

    #[test]
    fn test_detect_graphql() {
        let mut capture = NetworkCapture::new();
        capture.record(make_request(
            "https://api.example.com/graphql",
            "POST",
            true,
            vec![],
            Some(r#"{"query":"{ users { id } }"}"#),
            Some(r#"{"data":{"users":[]}}"#),
        ));
        capture.record(make_request(
            "https://api.example.com/rest/users",
            "GET",
            true,
            vec![],
            None,
            Some(r#"[]"#),
        ));

        let gql = capture.detect_graphql();
        assert_eq!(gql.len(), 1);
        assert!(gql[0].url.contains("/graphql"));
    }

    #[test]
    fn test_api_requests_filter() {
        let mut capture = NetworkCapture::new();
        capture.record(make_request(
            "https://api.example.com/data",
            "GET",
            true,
            vec![],
            None,
            None,
        ));
        capture.record(make_request(
            "https://cdn.example.com/img.png",
            "GET",
            false,
            vec![],
            None,
            None,
        ));

        assert_eq!(capture.api_requests().len(), 1);
    }

    #[test]
    fn test_summary_not_empty() {
        let mut capture = NetworkCapture::new();
        capture.record(make_request(
            "https://api.example.com/users/1",
            "GET",
            true,
            vec![],
            None,
            Some(r#"{"id":1}"#),
        ));

        let s = capture.summary();
        assert!(s.contains("Network Intelligence Summary"));
        assert!(s.contains("/users/{id}"));
    }

    #[test]
    fn test_empty_capture_summary() {
        let capture = NetworkCapture::new();
        let s = capture.summary();
        assert!(s.contains("Total requests: 0"));
        assert!(s.contains("Discovered 0 endpoint(s)"));
    }
}
