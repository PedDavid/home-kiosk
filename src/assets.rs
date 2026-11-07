//! The upstream frontend's Vite build, embedded in the binary.

use axum::{
    body::Body,
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "frontend/dist"]
struct Dist;

/// Page paths without the `.html`, as Cloudflare's asset server answered them upstream.
fn resolve(path: &str) -> String {
    let p = path.trim_start_matches('/');
    match p {
        "" => "index.html".into(),
        "immich" | "immich/" => "immich.html".into(),
        "calendars" | "calendars/" => "calendars.html".into(),
        "m" | "m/" => "m.html".into(),
        "upload" | "upload/" => "upload.html".into(),
        _ => p.to_string(),
    }
}

pub async fn serve(uri: Uri) -> Response {
    let name = resolve(uri.path());
    let Some(file) = Dist::get(&name) else {
        if name == "index.html" {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "Frontend not built: run `npm ci && npm run build` in frontend/, then rebuild.",
            )
                .into_response();
        }
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    };
    let mime = mime_guess::from_path(&name).first_or_octet_stream();
    // Hashed assets never change; everything else (pages, build.json) must be re-checked so frames
    // pick up a new deploy.
    let cache = if name.starts_with("assets/") {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(file.data.into_owned()))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::resolve;

    #[test]
    fn maps_pages() {
        assert_eq!(resolve("/"), "index.html");
        assert_eq!(resolve("/m"), "m.html");
        assert_eq!(resolve("/assets/main.js"), "assets/main.js");
    }
}
