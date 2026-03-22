use std::time::Instant;

use axum::{
    extract::{
        ws::{Message, WebSocket},
        Path, Query, State, WebSocketUpgrade,
    },
    response::Response,
    routing::get,
    Router,
};
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AppError;
use crate::middleware::Claims;
use crate::AppState;

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct WsConnectParams {
    /// JWT passed as a query parameter since browsers cannot set headers on
    /// WebSocket upgrade requests.
    pub token: String,
}

// ---------------------------------------------------------------------------
// Client → Server messages
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Navigate { url: String },
    Click { ref_id: u64 },
    Fill { ref_id: u64, text: String },
    EvalJs { code: String },
    Ping,
}

// ---------------------------------------------------------------------------
// Server → Client messages
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ServerMessage {
    Snapshot {
        dom_snapshot: serde_json::Value,
    },
    ActionResult {
        action: String,
        result: serde_json::Value,
        duration_ms: u64,
    },
    Navigation {
        url: String,
        title: String,
    },
    Error {
        code: String,
        message: String,
    },
    SessionEnded,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Returns the WebSocket router. Mount under `/ws` in the top-level router.
///
/// ```text
/// GET /ws/sessions/:id?token=<jwt>
/// ```
pub fn router() -> Router<AppState> {
    Router::new().route("/sessions/{id}", get(ws_upgrade))
}

// ---------------------------------------------------------------------------
// Upgrade handler
// ---------------------------------------------------------------------------

/// Validates the JWT from the query string, then upgrades the connection to a
/// WebSocket.
async fn ws_upgrade(
    State(state): State<AppState>,
    Path(session_id): Path<Uuid>,
    Query(params): Query<WsConnectParams>,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    // Validate JWT from query param — browsers cannot send headers on WS
    // upgrade requests, so we accept the token as `?token=xxx`.
    let key = DecodingKey::from_secret(state.config.jwt_secret.as_bytes());
    let validation = Validation::default();
    let token_data = decode::<Claims>(&params.token, &key, &validation)?;
    let claims = token_data.claims;

    tracing::info!(
        user_id = %claims.sub,
        org_id = %claims.org_id,
        %session_id,
        "WebSocket upgrade requested"
    );

    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, session_id, claims)))
}

// ---------------------------------------------------------------------------
// Socket loop
// ---------------------------------------------------------------------------

async fn handle_socket(
    mut socket: WebSocket,
    state: AppState,
    session_id: Uuid,
    claims: Claims,
) {
    tracing::info!(
        %session_id,
        user_id = %claims.sub,
        "WebSocket connected"
    );

    // Fetch the initial snapshot from the engine bridge and send it to the
    // client so they have DOM state immediately upon connection.
    match state.engine_bridge.snapshot(session_id).await {
        Ok(snap) => {
            if let Err(e) = send_msg(
                &mut socket,
                &ServerMessage::Snapshot {
                    dom_snapshot: serde_json::json!({
                        "session_id": session_id,
                        "status": "connected",
                        "url": snap.url,
                        "title": snap.title,
                        "html": snap.html,
                        "text": snap.text,
                        "screenshot_url": snap.screenshot_url,
                    }),
                },
            )
            .await
            {
                tracing::warn!("Failed to send initial snapshot: {e}");
                return;
            }
        }
        Err(e) => {
            tracing::warn!("Failed to get initial snapshot: {e}");
            let _ = send_msg(
                &mut socket,
                &ServerMessage::Error {
                    code: "snapshot_failed".into(),
                    message: format!("Failed to get initial snapshot: {e}"),
                },
            )
            .await;
            return;
        }
    }

    // Main receive loop.
    while let Some(frame) = socket.recv().await {
        let msg = match frame {
            Ok(Message::Text(txt)) => txt,
            Ok(Message::Close(_)) => {
                tracing::info!(%session_id, "Client sent close frame");
                break;
            }
            // Silently ignore binary / ping / pong frames.
            Ok(_) => continue,
            Err(e) => {
                tracing::warn!(%session_id, "Read error: {e}");
                break;
            }
        };

        let client_msg: ClientMessage = match serde_json::from_str(&msg) {
            Ok(m) => m,
            Err(e) => {
                let _ = send_msg(
                    &mut socket,
                    &ServerMessage::Error {
                        code: "invalid_message".into(),
                        message: format!("Failed to parse message: {e}"),
                    },
                )
                .await;
                continue;
            }
        };

        let started = Instant::now();

        match client_msg {
            ClientMessage::Navigate { url } => {
                match state.engine_bridge.navigate(session_id, &url).await {
                    Ok(snap) => {
                        let duration_ms = started.elapsed().as_millis() as u64;
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::Navigation {
                                url: snap.url.clone().unwrap_or_else(|| url.clone()),
                                title: snap.title.clone().unwrap_or_default(),
                            },
                        )
                        .await;
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::Snapshot {
                                dom_snapshot: serde_json::json!({
                                    "url": snap.url,
                                    "title": snap.title,
                                    "html": snap.html,
                                    "text": snap.text,
                                    "screenshot_url": snap.screenshot_url,
                                    "duration_ms": duration_ms,
                                }),
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::Error {
                                code: "navigate_failed".into(),
                                message: format!("{e}"),
                            },
                        )
                        .await;
                    }
                }
            }
            ClientMessage::Click { ref_id } => {
                let ref_id_str = ref_id.to_string();
                match state.engine_bridge.click(session_id, &ref_id_str).await {
                    Ok(action_result) => {
                        let duration_ms = started.elapsed().as_millis() as u64;
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::ActionResult {
                                action: "click".into(),
                                result: serde_json::json!({
                                    "ref_id": ref_id,
                                    "success": action_result.success,
                                    "message": action_result.message,
                                }),
                                duration_ms,
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::Error {
                                code: "click_failed".into(),
                                message: format!("{e}"),
                            },
                        )
                        .await;
                    }
                }
            }
            ClientMessage::Fill { ref_id, text } => {
                let ref_id_str = ref_id.to_string();
                match state.engine_bridge.fill(session_id, &ref_id_str, &text).await {
                    Ok(action_result) => {
                        let duration_ms = started.elapsed().as_millis() as u64;
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::ActionResult {
                                action: "fill".into(),
                                result: serde_json::json!({
                                    "ref_id": ref_id,
                                    "text": text,
                                    "success": action_result.success,
                                    "message": action_result.message,
                                }),
                                duration_ms,
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::Error {
                                code: "fill_failed".into(),
                                message: format!("{e}"),
                            },
                        )
                        .await;
                    }
                }
            }
            ClientMessage::EvalJs { code } => {
                match state.engine_bridge.eval_js(session_id, &code).await {
                    Ok(output) => {
                        let duration_ms = started.elapsed().as_millis() as u64;
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::ActionResult {
                                action: "eval_js".into(),
                                result: serde_json::json!({
                                    "code": code,
                                    "output": output,
                                }),
                                duration_ms,
                            },
                        )
                        .await;
                    }
                    Err(e) => {
                        let _ = send_msg(
                            &mut socket,
                            &ServerMessage::Error {
                                code: "eval_js_failed".into(),
                                message: format!("{e}"),
                            },
                        )
                        .await;
                    }
                }
            }
            ClientMessage::Ping => {
                let _ = socket
                    .send(Message::Text(r#"{"type":"pong"}"#.into()))
                    .await;
            }
        }
    }

    // Clean up the engine session on disconnect (best-effort).
    if let Err(e) = state.engine_bridge.destroy_session(session_id).await {
        tracing::warn!(%session_id, "Failed to destroy session on disconnect: {e}");
    }

    // Notify client that the session has ended (best-effort).
    let _ = send_msg(&mut socket, &ServerMessage::SessionEnded).await;

    tracing::info!(%session_id, "WebSocket disconnected");
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Serialize a [`ServerMessage`] and send it over the socket.
async fn send_msg(
    socket: &mut WebSocket,
    msg: &ServerMessage,
) -> Result<(), axum::Error> {
    let text = serde_json::to_string(msg).expect("ServerMessage is always serializable");
    socket.send(Message::Text(text.into())).await
}
