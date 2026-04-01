//! [`HttpTransport`] implementation backed by reqwest.

use std::collections::BTreeMap;
use crate::{HttpTransport, TransportError, TransportMethod, TransportRequest, TransportResponse};

/// HTTP transport backed by [`reqwest::Client`].
pub struct ReqwestTransport {
    client: reqwest::Client,
}

impl ReqwestTransport {
    /// Create a new transport with a default reqwest client.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }

    /// Create a transport from an existing reqwest client.
    pub fn with_client(client: reqwest::Client) -> Self {
        Self { client }
    }
}

impl Default for ReqwestTransport {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl HttpTransport for ReqwestTransport {
    async fn execute(
        &self,
        request: TransportRequest,
    ) -> Result<TransportResponse, TransportError> {
        let method = match request.method {
            TransportMethod::Get => reqwest::Method::GET,
            TransportMethod::Post => reqwest::Method::POST,
        };

        let mut builder = self.client.request(method, &request.url);

        for (key, value) in &request.headers {
            builder = builder.header(key.as_str(), value.as_str());
        }

        if let Some(body) = request.body {
            builder = builder.body(body);
        }

        let response = builder.send().await.map_err(|e| {
            if e.is_timeout() {
                TransportError::Timeout
            } else if e.is_connect() {
                TransportError::ConnectionFailed(e.to_string())
            } else {
                TransportError::Other(e.to_string())
            }
        })?;

        let status = response.status().as_u16();
        let url = response.url().to_string();

        let set_cookie_headers: Vec<String> = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok().map(|s| s.to_owned()))
            .collect();

        let headers: BTreeMap<String, String> = response
            .headers()
            .iter()
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|val| (k.as_str().to_owned(), val.to_owned()))
            })
            .collect();

        let body = response.bytes().await.map_err(|e| {
            TransportError::Other(format!("failed to read response body: {e}"))
        })?;

        Ok(TransportResponse {
            status,
            headers,
            body: body.to_vec(),
            url,
            set_cookie_headers,
        })
    }
}
