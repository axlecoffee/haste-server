// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
mod config;
mod limit;
mod logs;
mod store;

use axum::{
    Json, Router,
    body::to_bytes,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use mongodb::bson::{DateTime, Document, doc};
use serde_json::json;
use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tower_http::{
    compression::CompressionLayer,
    services::{ServeDir, ServeFile},
};

const MAX_BYTES: usize = 4_000_000;
const CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'";
const README: &str = include_str!("../../README.md");
const LICENSE: &str = concat!(
    include_str!("../../LICENSE.md"),
    "\n\n",
    include_str!("../../THIRD_PARTY_NOTICES")
);

struct App {
    store: store::Store,
    limiter: Mutex<limit::Limiter>,
    proxies: Vec<IpAddr>,
    logs: tokio::sync::mpsc::Sender<Document>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("haste_server=info")
        .init();
    let config = config::Config::load()?;
    let store = store::Store::new(&config);

    if std::env::args().any(|arg| arg == "--provision") {
        store.ready(true).await?;
        store.probe().await?;
        tracing::info!("provisioning complete");
        return Ok(());
    }
    store.ready(false).await?;

    let (logs, writer) = logs::start(config.mongo);
    let state = Arc::new(App {
        store,
        limiter: Mutex::default(),
        proxies: config.proxies,
        logs,
    });

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!("listening on {}", config.port);
    let app = router(state, &config.assets);
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown())
    .await?;

    if tokio::time::timeout(std::time::Duration::from_secs(15), writer)
        .await
        .is_err()
    {
        tracing::error!("log drain timed out");
    }
    Ok(())
}

async fn shutdown() {
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("signal handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}

fn router(state: Arc<App>, assets: &str) -> Router {
    let index = format!("{assets}/index.html");
    Router::new()
        .route("/documents", post(save))
        .route("/documents/{id}", get(document))
        .route("/raw/{id}", get(raw))
        .route("/raw", get(not_found))
        .route("/source.tar.gz", get_service_source())
        .route("/theme.css", get(theme_css))
        .route("/syntax-notices.txt", get(syntax_notices))
        .route_service("/", ServeFile::new(&index))
        .route_service("/{id}", ServeFile::new(index))
        .nest_service("/assets", ServeDir::new(assets).precompressed_gzip())
        .fallback(not_found)
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(state.clone(), observe))
        .with_state(state)
}

fn get_service_source() -> axum::routing::MethodRouter<Arc<App>> {
    axum::routing::get_service(ServeFile::new(
        std::env::var("SOURCE_ARCHIVE").unwrap_or_else(|_| "source.tar.gz".into()),
    ))
}

async fn theme_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!(concat!(env!("OUT_DIR"), "/theme.css")),
    )
        .into_response()
}

async fn syntax_notices() -> &'static str {
    include_str!(concat!(env!("OUT_DIR"), "/syntax-notices.txt"))
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Document not found.")
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"message": message}))).into_response()
}

fn reserved(key: &str) -> Option<&'static str> {
    match key {
        "about" | "readme" => Some(README),
        "license" => Some(LICENSE),
        _ => None,
    }
}

fn parse_key(id: &str) -> Option<&str> {
    // anything after the first dot is presentation only
    let key = id.split('.').next()?;
    if key.is_empty() || key.len() > 64 {
        return None;
    }
    if !key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return None;
    }
    Some(key)
}

async fn save(State(state): State<Arc<App>>, request: Request) -> Response {
    let content = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("text/plain");
    let media = content
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        media.as_str(),
        "text/plain"
            | "application/json"
            | "application/x-www-form-urlencoded"
            | "application/octet-stream"
    ) {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "Use plain text or a JSON string.",
        );
    }

    let bytes = match to_bytes(request.into_body(), MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Maximum body size is 400000 bytes.",
            );
        }
    };

    let text = if media == "application/json" {
        match serde_json::from_slice::<String>(&bytes) {
            Ok(text) => text,
            Err(_) => return error(StatusCode::BAD_REQUEST, "Body must be a JSON string."),
        }
    } else {
        match String::from_utf8(bytes.to_vec()) {
            Ok(text) => text,
            Err(_) => return error(StatusCode::BAD_REQUEST, "Body must be UTF-8."),
        }
    };

    match state.store.save(&text).await {
        Ok(key) => (StatusCode::CREATED, Json(json!({ "key": key }))).into_response(),
        Err(e) => {
            tracing::error!("save failed: {e:#}");
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Storage unavailable; document was not saved.",
            )
        }
    }
}

async fn document(State(state): State<Arc<App>>, Path(id): Path<String>) -> Response {
    read(&state, &id, false).await
}
async fn raw(State(state): State<Arc<App>>, Path(id): Path<String>) -> Response {
    read(&state, &id, true).await
}

async fn read(state: &App, id: &str, raw: bool) -> Response {
    let Some(key) = parse_key(id) else {
        return not_found().await;
    };

    // builtins come from the binary, pastes from storage
    let builtin = reserved(key);
    let data = if let Some(text) = builtin {
        text.to_owned()
    } else {
        match state.store.get(key).await {
            Ok(Some(text)) => text,
            Ok(None) => return not_found().await,
            Err(e) => {
                tracing::error!("read failed: {e:#}");
                return error(StatusCode::SERVICE_UNAVAILABLE, "Storage unavailable.");
            }
        }
    };

    let mut response = if raw {
        ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], data).into_response()
    } else {
        Json(json!({ "data": data, "key": key })).into_response()
    };
    let cache = if builtin.is_some() {
        "no-cache"
    } else {
        "public, max-age=300, s-maxage=86400"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

// forwarding headers only count when the socket peer is a configured proxy
fn client(headers: &HeaderMap, peer: IpAddr, proxies: &[IpAddr]) -> (IpAddr, bool) {
    if proxies.contains(&peer)
        && let Some(ip) = header_string(headers, "cf-connecting-ip").and_then(|ip| ip.parse().ok())
    {
        return (ip, true);
    }
    (peer, false)
}

// log values are capped so one giant header cannot bloat the collection
fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    Some(value.chars().take(1024).collect())
}

async fn observe(State(state): State<Arc<App>>, request: Request, next: Next) -> Response {
    let start = Instant::now();
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .expect("socket connection info")
        .0
        .ip();
    let (ip, forwarded) = client(request.headers(), peer, &state.proxies);
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    // built before the request moves into the handler
    let mut entry = doc! {
        "timestamp": DateTime::now(),
        "method": method.as_str(),
        "path": path.chars().take(2048).collect::<String>(),
        "ip": ip.to_string(),
        "peer_ip": peer.to_string(),
    };
    if let Some(agent) = header_string(request.headers(), "user-agent") {
        entry.insert("user_agent", agent);
    }
    if forwarded {
        for (field, name) in [
            ("country", "cf-ipcountry"),
            ("region", "cf-region"),
            ("city", "cf-ipcity"),
        ] {
            if let Some(value) = header_string(request.headers(), name) {
                entry.insert(field, value);
            }
        }
    }

    let limited = method == Method::POST && path == "/documents";
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let quota = if limited {
        state.limiter.lock().unwrap().check(ip, now)
    } else {
        None
    };
    let mut response = match quota.as_ref() {
        Some(quota) if !quota.allowed => {
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many documents; try again shortly.",
            );
            let wait = quota.reset - now;
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, wait.to_string().parse().unwrap());
            response
        }
        None if limited => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Upload limiter is at capacity.",
        ),
        _ => next.run(request).await,
    };

    let headers = response.headers_mut();
    if let Some(quota) = quota {
        headers.insert("x-ratelimit-limit", HeaderValue::from_static("120"));
        headers.insert(
            "x-ratelimit-remaining",
            quota.remaining.to_string().parse().unwrap(),
        );
        headers.insert(
            "x-ratelimit-reset",
            quota.reset.to_string().parse().unwrap(),
        );
    }
    headers.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static(CSP));
    if !headers.contains_key(header::CACHE_CONTROL) {
        let value = if path.starts_with("/assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    }
    if !response.status().is_success() || method == Method::POST {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }

    entry.insert("status", response.status().as_u16() as i32);
    entry.insert("duration_ms", start.elapsed().as_millis() as i64);
    if state.logs.try_send(entry).is_err() {
        tracing::error!("request log dropped");
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn forwarding_headers_need_a_trusted_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "203.0.113.5".parse().unwrap());
        let peer = "192.168.1.10".parse().unwrap();

        // not a listed proxy, the header is ignored
        assert_eq!(client(&headers, peer, &[]), (peer, false));
        // exact peer match, the header is the client
        assert_eq!(
            client(&headers, peer, &[peer]),
            ("203.0.113.5".parse().unwrap(), true)
        );
        // garbage in the header falls back to the socket address
        headers.insert("cf-connecting-ip", "bad".parse().unwrap());
        assert_eq!(client(&headers, peer, &[peer]), (peer, false));
    }

    fn app() -> Router {
        let config = config::Config {
            endpoint: "http://127.0.0.1:1".into(),
            region: "ca-1".into(),
            bucket: "test".into(),
            access: "test".into(),
            secret: "test".into(),
            mongo: String::new(),
            proxies: vec![],
            assets: String::new(),
            port: 8292,
        };
        let (logs, _rx) = tokio::sync::mpsc::channel(1024);
        let state = App {
            store: store::Store::new(&config),
            limiter: Mutex::default(),
            proxies: vec![],
            logs,
        };
        router(Arc::new(state), "/nonexistent")
    }

    fn request(method: Method, path: &str) -> Request {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .body(axum::body::Body::empty())
            .unwrap();
        request
            .extensions_mut()
            .insert(ConnectInfo("127.0.0.1:4567".parse::<SocketAddr>().unwrap()));
        request
    }

    #[tokio::test]
    async fn posts_stop_at_120_per_minute() {
        let app = app();
        // an invalid content type keeps requests away from storage while still counting
        for _ in 0..120 {
            let mut request = request(Method::POST, "/documents");
            request
                .headers_mut()
                .insert(header::CONTENT_TYPE, "application/xml".parse().unwrap());
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        let mut request = request(Method::POST, "/documents");
        request
            .headers_mut()
            .insert(header::CONTENT_TYPE, "application/xml".parse().unwrap());
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[tokio::test]
    async fn reads_are_never_limited() {
        let app = app();
        for _ in 0..125 {
            let response = app
                .clone()
                .oneshot(request(Method::GET, "/documents/about.md"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(!response.headers().contains_key("x-ratelimit-limit"));
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        }
    }
}
