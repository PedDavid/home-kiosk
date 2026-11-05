//! Upstream's shared-key access: `?key=` on a page is swapped for a long-lived HttpOnly cookie
//! (`hq`), and API calls carry that cookie or an `x-hq-key` header. Keys are compared in constant
//! time. With no key configured, everything is open (local development and the demo).

use axum::{
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use subtle::ConstantTimeEq;

use crate::AppState;

const COOKIE: &str = "hq";
const TEN_YEARS_S: u64 = 315_360_000;

pub fn same_key(given: &str, expected: &str) -> bool {
    !given.is_empty() && given.as_bytes().ct_eq(expected.as_bytes()).into()
}

fn cookie_key(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| percent_decode(v))
}

fn query_param(query: &str, name: &str) -> Option<String> {
    query
        .split('&')
        .filter_map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (k == name).then(|| percent_decode(&v.replace('+', " ")))
        })
        .next()
}

fn without_param(query: &str, name: &str) -> String {
    query
        .split('&')
        .filter(|kv| !kv.is_empty() && kv.split_once('=').map_or(*kv, |(k, _)| k) != name)
        .collect::<Vec<_>>()
        .join("&")
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(hex) = std::str::from_utf8(&b[i + 1..i + 3])
            && let Ok(n) = u8::from_str_radix(hex, 16)
        {
            out.push(n);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|c| match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (c as char).to_string(),
            _ => format!("%{c:02X}"),
        })
        .collect()
}

fn locked() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(serde_json::json!({ "error": "locked" })),
    )
        .into_response()
}

pub async fn middleware(State(app): State<AppState>, req: Request, next: Next) -> Response {
    let Some(expected) = app.key.as_deref() else {
        return next.run(req).await;
    };
    let path = req.uri().path().to_string();
    let query = req.uri().query().unwrap_or("").to_string();
    let is_api = path.starts_with("/api/");

    // ?key=… on any page: remember it in a cookie and drop it from the address bar.
    if !is_api && let Some(given) = query_param(&query, "key") {
        if !same_key(&given, expected) {
            return (StatusCode::FORBIDDEN, "Wrong key").into_response();
        }
        let rest = without_param(&query, "key");
        let location = if rest.is_empty() {
            path
        } else {
            format!("{path}?{rest}")
        };
        let cookie = format!(
            "{COOKIE}={}; Path=/; Max-Age={TEN_YEARS_S}; HttpOnly; Secure; SameSite=Lax",
            percent_encode(&given)
        );
        let mut res = StatusCode::FOUND.into_response();
        let h = res.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&location) {
            h.insert(header::LOCATION, v);
        }
        if let Ok(v) = HeaderValue::from_str(&cookie) {
            h.insert(header::SET_COOKIE, v);
        }
        return res;
    }

    // Pages and assets are public (they hold no data); the API needs the key.
    if !is_api {
        return next.run(req).await;
    }
    let given = cookie_key(req.headers()).or_else(|| {
        req.headers()
            .get("x-hq-key")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    });
    match given {
        Some(g) if same_key(&g, expected) => next.run(req).await,
        _ => locked(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_keys() {
        assert!(same_key("abc", "abc"));
        assert!(!same_key("abd", "abc"));
        assert!(!same_key("ab", "abc"));
        assert!(!same_key("", ""));
    }

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(percent_decode("a%20b%2Fc"), "a b/c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode(&percent_encode("k/+ =é")), "k/+ =é");
    }

    #[test]
    fn strips_only_the_key_param() {
        assert_eq!(without_param("frame=kitchen&key=x&a=1", "key"), "frame=kitchen&a=1");
        assert_eq!(without_param("key=x", "key"), "");
        assert_eq!(query_param("a=1&key=s%2Fx", "key").as_deref(), Some("s/x"));
    }
}
