// crates/node/src/explorer.rs
use axum::{
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use mime_guess::from_path;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../explorer/dist"]
pub struct WebAssets;

/// A fallback handler for the Axum router that intercepts missing API routes
/// and maps them to the embedded Vite SPA.
pub async fn static_handler(uri: Uri) -> impl IntoResponse {
    let mut path = uri.path().trim_start_matches('/').to_string();

    // Map the root /ui endpoint to the built index.html
    if path.starts_with("ui/") {
        path = path.replacen("ui/", "", 1);
    } else if path == "ui" {
        path = "index.html".to_string();
    }
    
    // For general root requests bridging to the UI
    if path.is_empty() {
        path = "index.html".to_string();
    }

    // Try to get the embedded asset
    match WebAssets::get(&path) {
        Some(content) => {
            let mime = from_path(&path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref())],
                content.data,
            ).into_response()
        }
        None => {
            // Fallback for React Router / SPA semantics: if an asset is not found,
            // we default back to the index.html so the client router takes over.
            if let Some(index) = WebAssets::get("index.html") {
                let mime = from_path("index.html").first_or_octet_stream();
                (
                    [(header::CONTENT_TYPE, mime.as_ref())],
                    index.data,
                ).into_response()
            } else {
                (StatusCode::NOT_FOUND, "404 Not Found. Make sure to build the explorer.").into_response()
            }
        }
    }
}
