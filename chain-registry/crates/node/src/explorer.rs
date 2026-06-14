// crates/node/src/explorer.rs
use axum::{
    http::{StatusCode, Uri},
    response::IntoResponse,
};

/// Serve the built Vite explorer SPA when compiled with `embedded-explorer`.
#[cfg(feature = "embedded-explorer")]
pub async fn static_handler(uri: Uri) -> impl IntoResponse {
    use axum::http::header;
    use mime_guess::from_path;
    use rust_embed::RustEmbed;

    #[derive(RustEmbed)]
    #[folder = "../../explorer/dist"]
    struct WebAssets;

    let mut path = uri.path().trim_start_matches('/').to_string();

    if path.starts_with("ui/") {
        path = path.replacen("ui/", "", 1);
    } else if path == "ui" {
        path = "index.html".to_string();
    }

    if path.is_empty() {
        path = "index.html".to_string();
    }

    match WebAssets::get(&path) {
        Some(content) => {
            let mime = from_path(&path).first_or_octet_stream();
            ([(header::CONTENT_TYPE, mime.as_ref())], content.data).into_response()
        }
        None => {
            if let Some(index) = WebAssets::get("index.html") {
                let mime = from_path("index.html").first_or_octet_stream();
                ([(header::CONTENT_TYPE, mime.as_ref())], index.data).into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    "404 Not Found. Make sure to build the explorer.",
                )
                    .into_response()
            }
        }
    }
}

/// Default CI/dev builds serve explorer static assets externally (nginx/CDN).
#[cfg(not(feature = "embedded-explorer"))]
pub async fn static_handler(_uri: Uri) -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        "Explorer UI is not embedded in this build. Serve static files externally or rebuild with --features embedded-explorer.",
    )
}
