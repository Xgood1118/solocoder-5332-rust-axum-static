use axum::body::Body;
use axum::http::{HeaderValue, Request, Response, header};
use axum::middleware::Next;

pub async fn security_headers_middleware(
    request: Request<Body>,
    next: Next,
) -> Result<Response<Body>, std::convert::Infallible> {
    let response = next.run(request).await;
    Ok(add_security_headers(response))
}

fn add_security_headers(mut response: Response<Body>) -> Response<Body> {
    let headers = response.headers_mut();

    if !headers.contains_key(header::X_CONTENT_TYPE_OPTIONS) {
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
    }

    if !headers.contains_key("X-Frame-Options") {
        headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    }

    if !headers.contains_key("X-XSS-Protection") {
        headers.insert(
            "X-XSS-Protection",
            HeaderValue::from_static("1; mode=block"),
        );
    }

    response
}
