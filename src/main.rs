mod config;
mod security;
mod mime_util;
mod cache;
mod hotlink;
mod metrics;
mod handlers;
mod middleware;

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method};
use axum::middleware::from_fn;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use clap::Parser;
use serde::Deserialize;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;
use crate::handlers::static_file::StaticFileHandler;
use crate::middleware::security_headers::security_headers_middleware;
use crate::middleware::server_timing::server_timing_middleware;
use crate::metrics::ServerMetrics;
use crate::security::PathValidator;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "127.0.0.1:8080")]
    addr: String,

    #[arg(short, long, default_value = "./public")]
    root: String,

    #[arg(long)]
    spa: bool,

    #[arg(long)]
    dir_listing: bool,
}

#[derive(Clone)]
struct AppState {
    file_handler: Arc<StaticFileHandler>,
    metrics: ServerMetrics,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let config = build_config(&cli);

    let metrics = ServerMetrics::new();

    let path_validator = PathValidator::new(Arc::new(config.server.clone()));
    let cache_policy = crate::cache::CachePolicy::new(Arc::new(config.cache.clone()));
    let hotlink_protector =
        crate::hotlink::HotlinkProtector::new(Arc::new(config.hotlink.clone()));

    let file_handler = StaticFileHandler::new(
        path_validator,
        cache_policy,
        Arc::new(config.compression.clone()),
        hotlink_protector,
        metrics.clone(),
        Arc::new(config.server.clone()),
    );

    let state = AppState {
        file_handler: Arc::new(file_handler),
        metrics: metrics.clone(),
    };

    let app = Router::new()
        .route("/health", get(health_handler).head(health_handler))
        .route("/", get(root_handler).head(root_handler))
        .route("/*path", get(static_handler).head(static_handler))
        .with_state(state)
        .layer(from_fn(security_headers_middleware))
        .layer(from_fn(server_timing_middleware));

    let listener = tokio::net::TcpListener::bind(&config.server.listen_addr).await?;
    info!("Static file server listening on {}", config.server.listen_addr);
    info!("Serving files from: {:?}", config.server.root_dir);
    info!("SPA fallback: {}", config.server.spa_fallback);
    info!("Directory listing: {}", config.server.directory_listing);

    axum::serve(listener, app).await?;

    Ok(())
}

fn build_config(cli: &Cli) -> AppConfig {
    let mut config = AppConfig::new();
    config.server.listen_addr = cli.addr.clone();
    config.server.root_dir = std::path::PathBuf::from(&cli.root);
    config.server.spa_fallback = cli.spa;
    config.server.directory_listing = cli.dir_listing;
    config
}

#[derive(Debug, Deserialize)]
struct DirectoryQuery {
    page: Option<usize>,
    sort: Option<String>,
    order: Option<String>,
    q: Option<String>,
}

async fn health_handler(State(state): State<AppState>) -> Response {
    axum::Json(state.metrics.to_health_json()).into_response()
}

async fn root_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    Query(query): Query<DirectoryQuery>,
) -> Response {
    state
        .file_handler
        .serve_with_query(
            "/",
            &headers,
            &method,
            query.page,
            query.sort.as_deref(),
            query.order.as_deref(),
            query.q.as_deref(),
        )
        .await
}

async fn static_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    method: Method,
    Path(path): Path<String>,
    Query(query): Query<DirectoryQuery>,
) -> Response {
    state
        .file_handler
        .serve_with_query(
            &format!("/{}", path),
            &headers,
            &method,
            query.page,
            query.sort.as_deref(),
            query.order.as_deref(),
            query.q.as_deref(),
        )
        .await
}
