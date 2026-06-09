use std::time::Instant;

use axum::body::Body;
use axum::http::{HeaderValue, Request, Response};
use axum::middleware::Next;

pub async fn server_timing_middleware(
    request: Request<Body>,
    next: Next,
) -> Result<Response<Body>, std::convert::Infallible> {
    let start = Instant::now();
    let mut response = next.run(request).await;
    let duration = start.elapsed();

    let dur_ms = duration.as_secs_f64() * 1000.0;
    let timing_value = format!("total;dur={:.3}", dur_ms);

    if let Ok(value) = HeaderValue::from_str(&timing_value) {
        response.headers_mut().insert("server-timing", value);
    }

    Ok(response)
}
