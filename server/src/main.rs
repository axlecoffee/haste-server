// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
mod config;
mod limit;
mod logs;
mod store;

use axum::{
    Json, Router, body::to_bytes,
    extract::{ConnectInfo, Path, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response}, routing::{get, post},
};
use mongodb::bson::{DateTime, Document, doc};
use serde_json::json;
use std::{net::{IpAddr, SocketAddr}, sync::{Arc, Mutex}, time::{Instant, SystemTime, UNIX_EPOCH}};
use tower_http::{compression::CompressionLayer, services::{ServeDir, ServeFile}};

const MAX_BYTES: usize = 400_000;
const README: &str = include_str!("../../README.md");
const LICENSE: &str = concat!(include_str!("../../LICENSE.md"), "\n\n", include_str!("../../THIRD_PARTY_NOTICES"));

struct App {
    store: store::Store,
    limiter: Mutex<limit::Limiter>,
    proxies: Vec<IpAddr>,
    logs: tokio::sync::mpsc::Sender<Document>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("haste_server=info").init();
    let config = config::Config::load()?;
    let store = store::Store::new(&config);
    let provision = std::env::args().any(|arg| arg == "--provision");
    store.ready(provision).await?;
    if provision {
        store.probe().await?;
        tracing::info!("Private bucket ready; conditional PUT and GET verified; probe removed");
        return Ok(());
    }
    let (logs, writer) = logs::start(config.mongo);
    let state = Arc::new(App { store, limiter: Mutex::default(), proxies: config.proxies, logs });
    let app = router(state, &config.assets);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", config.port)).await?;
    tracing::info!(port = config.port, "Listening");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("signal handler");
            tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = term.recv() => {} }
        }).await?;
    if tokio::time::timeout(std::time::Duration::from_secs(15), writer).await.is_err() {
        tracing::error!("Log writer did not drain before shutdown deadline");
    }
    Ok(())
}

fn router(state: Arc<App>, assets: &str) -> Router {
    let index = format!("{assets}/index.html");
    Router::new()
        .route("/documents", post(save))
        .route("/documents/{id}", get(document))
        .route("/raw/{id}", get(raw))
        .route("/raw", get(not_found))
        .route("/source.tar.gz", get_service_source())
        .route("/theme.css", get(|| async { ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], include_str!(concat!(env!("OUT_DIR"), "/theme.css"))) }))
        .route("/syntax-notices.txt", get(|| async { include_str!(concat!(env!("OUT_DIR"), "/syntax-notices.txt")) }))
        .route_service("/", ServeFile::new(&index))
        .route_service("/{id}", ServeFile::new(index))
        .nest_service("/assets", ServeDir::new(assets).precompressed_gzip())
        .fallback(not_found)
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn_with_state(state.clone(), observe))
        .with_state(state)
}

fn get_service_source() -> axum::routing::MethodRouter<Arc<App>> {
    axum::routing::get_service(ServeFile::new(std::env::var("SOURCE_ARCHIVE").unwrap_or_else(|_| "source.tar.gz".into())))
}

async fn not_found() -> Response { error(StatusCode::NOT_FOUND, "Document not found.") }

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"message": message}))).into_response()
}

fn reserved(key: &str) -> Option<&'static str> {
    match key { "about" | "readme" => Some(README), "license" => Some(LICENSE), _ => None }
}

fn parse_key(id: &str) -> Option<&str> {
    let key = id.split('.').next()?;
    (!key.is_empty() && key.len() <= 64 && key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')).then_some(key)
}

async fn save(State(state): State<Arc<App>>, request: Request) -> Response {
    let media = request.headers().get(header::CONTENT_TYPE).and_then(|h| h.to_str().ok())
        .unwrap_or("text/plain").split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    if !matches!(media.as_str(), "text/plain" | "application/json" | "application/x-www-form-urlencoded" | "application/octet-stream") {
        return error(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Use plain text or a JSON string.");
    }
    let bytes = match to_bytes(request.into_body(), MAX_BYTES).await {
        Ok(bytes) => bytes,
        Err(_) => return error(StatusCode::PAYLOAD_TOO_LARGE, "Maximum body size is 400000 bytes."),
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
        Ok(key) => (StatusCode::CREATED, Json(json!({"key": key}))).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Paste save failed");
            error(StatusCode::SERVICE_UNAVAILABLE, "Storage unavailable; document was not saved.")
        }
    }
}

async fn document(State(state): State<Arc<App>>, Path(id): Path<String>) -> Response { read(&state, &id, false).await }
async fn raw(State(state): State<Arc<App>>, Path(id): Path<String>) -> Response { read(&state, &id, true).await }

async fn read(state: &App, id: &str, raw: bool) -> Response {
    let Some(key) = parse_key(id) else { return not_found().await; };
    let builtin = reserved(key);
    let data = if let Some(text) = builtin { text.into() } else {
        match state.store.get(key).await {
            Ok(Some(text)) => text,
            Ok(None) => return not_found().await,
            Err(e) => {
                tracing::error!(error = %e, "Paste read failed");
                return error(StatusCode::SERVICE_UNAVAILABLE, "Storage unavailable.");
            }
        }
    };
    let mut response = if raw {
        ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], data).into_response()
    } else { Json(json!({"data": data, "key": key})).into_response() };
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(if builtin.is_some() {
        "no-cache"
    } else { "public, max-age=300, s-maxage=86400" }));
    response
}

fn client(headers: &HeaderMap, peer: IpAddr, proxies: &[IpAddr]) -> (IpAddr, bool) {
    if proxies.contains(&peer) {
        if let Some(ip) = headers.get("cf-connecting-ip").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok()) {
            return (ip, true);
        }
    }
    (peer, false)
}

async fn observe(State(state): State<Arc<App>>, request: Request, next: Next) -> Response {
    let start = Instant::now();
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().expect("socket connection info").0.ip();
    let (ip, forwarded) = client(request.headers(), peer, &state.proxies);
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let mut log = doc! {"timestamp": DateTime::now(), "method": method.as_str(), "path": path.chars().take(2048).collect::<String>(), "ip": ip.to_string(), "peer_ip": peer.to_string()};
    for (field, name) in [("user_agent", "user-agent"), ("country", "cf-ipcountry"), ("region", "cf-region"), ("city", "cf-ipcity")] {
        if field == "user_agent" || forwarded {
            if let Some(value) = request.headers().get(name).and_then(|v| v.to_str().ok()) {
                log.insert(field, value.chars().take(1024).collect::<String>());
            }
        }
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let limited = method == Method::POST && path == "/documents";
    let quota = if limited { state.limiter.lock().unwrap().check(ip, now) } else { None };
    let mut response = match (limited, quota) {
        (true, Some((false, _, reset))) => {
            let mut response = error(StatusCode::TOO_MANY_REQUESTS, "Too many documents; try again shortly.");
            response.headers_mut().insert(header::RETRY_AFTER, (reset - now).to_string().parse().unwrap());
            response
        }
        (true, None) => error(StatusCode::SERVICE_UNAVAILABLE, "Upload limiter is at capacity."),
        _ => next.run(request).await,
    };
    let headers = response.headers_mut();
    if let Some((_, remaining, reset)) = quota {
        headers.insert("x-ratelimit-limit", HeaderValue::from_static("120"));
        headers.insert("x-ratelimit-remaining", remaining.to_string().parse().unwrap());
        headers.insert("x-ratelimit-reset", reset.to_string().parse().unwrap());
    }
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    headers.insert("content-security-policy", HeaderValue::from_static("default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; connect-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'"));
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(if path.starts_with("/assets/") { "public, max-age=31536000, immutable" } else { "no-cache" }));
    }
    if !response.status().is_success() || method == Method::POST {
        response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    log.insert("status", response.status().as_u16() as i32);
    log.insert("duration_ms", start.elapsed().as_millis() as i64);
    if state.logs.try_send(log).is_err() {
        tracing::error!("Mongo log queue full or closed; origin request log dropped");
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[test]
    fn trusted_headers_require_exact_peer() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", "203.0.113.5".parse().unwrap());
        let peer = "192.168.1.10".parse().unwrap();
        assert_eq!(client(&headers, peer, &[]), (peer, false));
        assert_eq!(client(&headers, peer, &[peer]), ("203.0.113.5".parse().unwrap(), true));
        headers.insert("cf-connecting-ip", "bad".parse().unwrap());
        assert_eq!(client(&headers, peer, &[peer]), (peer, false));
    }

    #[tokio::test]
    async fn reads_are_unlimited_and_creation_limit_is_route_specific() {
        let config = config::Config { endpoint: "http://127.0.0.1:1".into(), region: "ca-1".into(), bucket: "test".into(), access: "test".into(), secret: "test".into(), mongo: String::new(), proxies: vec![], assets: String::new(), port: 8292 };
        let (logs, _rx) = tokio::sync::mpsc::channel(1024);
        let app = router(Arc::new(App { store: store::Store::new(&config), limiter: Mutex::default(), proxies: vec![], logs }), "/nonexistent");
        for i in 0..121 {
            let mut request = Request::builder().method("POST").uri("/documents").header("content-type", "application/xml").body(axum::body::Body::empty()).unwrap();
            request.extensions_mut().insert(ConnectInfo("127.0.0.1:4567".parse::<SocketAddr>().unwrap()));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), if i == 120 { StatusCode::TOO_MANY_REQUESTS } else { StatusCode::UNSUPPORTED_MEDIA_TYPE });
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
        for _ in 0..125 {
            let mut request = Request::builder().uri("/documents/about.md").body(axum::body::Body::empty()).unwrap();
            request.extensions_mut().insert(ConnectInfo("127.0.0.1:4567".parse::<SocketAddr>().unwrap()));
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(!response.headers().contains_key("x-ratelimit-limit"));
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-cache");
        }
    }
}