use crate::{
    error::{AppError, Result},
    state::AppState,
};
use axum::{
    Router,
    extract::Request,
    http::{HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use std::time::Duration;
use tower_http::{compression::CompressionLayer, services::ServeDir, trace::TraceLayer};

/// Far above any real path segment, well under the system's file name limit: scanners send
/// longer ones, which the file service would answer with a 500.
const MAX_PATH_SEGMENT_BYTES: usize = 200;
const HTTP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);
/// The site's own files carry a version in their address, so a few minutes of caching is safe.
const ASSET_CACHE: &str = "public, max-age=300";
/// Pictures come from tmpfile; everything else is the site's own.
const CSP: &str = "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: https:; font-src 'self'; connect-src 'self'; media-src 'none'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'";

pub fn router(state: AppState) -> Router {
    let assets = state.config.pages_dir.join("assets");
    Router::new()
        .merge(crate::pages::router())
        .nest_service(
            "/assets",
            ServeDir::new(assets)
                .precompressed_gzip()
                .append_index_html_on_directories(false),
        )
        .fallback(|| async { AppError::NotFound("没有这个页面".into()) })
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn(reject_oversized_path))
        .layer(middleware::from_fn(security_headers))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn reject_oversized_path(request: Request, next: Next) -> Response {
    if request
        .uri()
        .path()
        .split('/')
        .any(|segment| segment.len() > MAX_PATH_SEGMENT_BYTES)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    next.run(request).await
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    for (name, value) in [
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        (header::REFERRER_POLICY, "strict-origin-when-cross-origin"),
        (header::CONTENT_SECURITY_POLICY, CSP),
        (header::X_FRAME_OPTIONS, "DENY"),
        (header::STRICT_TRANSPORT_SECURITY, "max-age=31536000"),
    ] {
        headers.insert(name, HeaderValue::from_static(value));
    }
    headers.insert(
        header::HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    headers.insert(
        header::HeaderName::from_static("cross-origin-opener-policy"),
        HeaderValue::from_static("same-origin"),
    );
    if !headers
        .get_all(header::VARY)
        .iter()
        .any(|value| value.as_bytes().eq_ignore_ascii_case(b"accept-encoding"))
    {
        headers.append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    }
    if !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(ASSET_CACHE));
    }
    response
}

pub async fn serve(state: AppState) -> Result<()> {
    let bind_addr = state.config.bind_addr;
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|error| AppError::Config(format!("监听地址不可用: {error}").into()))?;
    tracing::info!(%bind_addr, "learn listening");
    let signal = state.shutdown.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        signal.cancel();
    });
    let stopping = state.shutdown.clone();
    let deadline = state.shutdown.clone();
    let server = std::future::IntoFuture::into_future(
        axum::serve(listener, router(state))
            .with_graceful_shutdown(async move { stopping.cancelled().await }),
    );
    tokio::pin!(server);
    // Once shutdown begins, open requests get a while to finish.
    let result = tokio::select! {
        result = &mut server => result,
        () = deadline.cancelled() => match tokio::time::timeout(HTTP_SHUTDOWN_TIMEOUT, &mut server).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!("HTTP graceful shutdown timed out; closing connections");
                Ok(())
            }
        },
    };
    result.map_err(|error| AppError::Internal(format!("HTTP 服务失败: {error}").into()))
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {}
        () = terminate => {}
    }
}
