use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(serde::Deserialize)]
struct QueryRequest {
    #[serde(rename = "connectionId")]
    connection_id: String,
    sql: String,
}

pub async fn handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(mut socket: WebSocket, state: Arc<AppState>) {
    // Expect first message to be the query request JSON
    let request: QueryRequest = match socket.recv().await {
        Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                let _ = socket
                    .send(Message::Text(
                        serde_json::json!({
                            "event": "error",
                            "data": { "message": format!("Invalid query request: {}", e) }
                        })
                        .to_string(),
                    ))
                    .await;
                return;
            }
        },
        _ => return,
    };

    let (tx, mut rx) = mpsc::channel::<sqlator_core::models::QueryEvent>(256);

    // Bridge: core mpsc → WebSocket text frames
    let bridge_handle = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let json = match serde_json::to_string(&event) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if socket.send(Message::Text(json)).await.is_err() {
                break;
            }
        }
        // Cleanly close
        let _ = socket.send(Message::Close(None)).await;
    });

    let _ = state
        .db
        .execute_query(&request.connection_id, &request.sql, tx)
        .await;

    let _ = bridge_handle.await;
}
