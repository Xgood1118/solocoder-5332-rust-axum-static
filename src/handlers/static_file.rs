use std::fs::Metadata;
use std::io::SeekFrom;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_compression::tokio::write::{BrotliEncoder, DeflateEncoder, GzipEncoder};
use axum::body::Body;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;
use tracing::debug;

use crate::cache::CachePolicy;
use crate::config::{CompressionConfig, ServerConfig};
use crate::handlers::directory::DirectoryLister;
use crate::hotlink::{HotlinkProtector, HotlinkResult};
use crate::metrics::ServerMetrics;
use crate::mime_util::{guess_mime, is_compressible};
use crate::security::{PathValidator, SecurityError};

pub struct StaticFileHandler {
    pub path_validator: PathValidator,
    pub cache_policy: CachePolicy,
    pub compression_config: Arc<CompressionConfig>,
    pub hotlink_protector: HotlinkProtector,
    pub metrics: ServerMetrics,
    pub directory_lister: DirectoryLister,
    pub index_file: String,
    pub spa_fallback: bool,
    pub directory_listing: bool,
}

impl StaticFileHandler {
    pub fn new(
        path_validator: PathValidator,
        cache_policy: CachePolicy,
        compression_config: Arc<CompressionConfig>,
        hotlink_protector: HotlinkProtector,
        metrics: ServerMetrics,
        server_config: Arc<ServerConfig>,
    ) -> Self {
        let directory_lister = DirectoryLister::new(
            server_config.clone(),
            path_validator.clone(),
        );
        let index_file = server_config.index_file.clone();
        let spa_fallback = server_config.spa_fallback;
        let directory_listing = server_config.directory_listing;

        Self {
            path_validator,
            cache_policy,
            compression_config,
            hotlink_protector,
            metrics,
            directory_lister,
            index_file,
            spa_fallback,
            directory_listing,
        }
    }

    pub async fn serve(&self, request_path: &str, headers: &HeaderMap, method: &Method) -> Response {
        self.serve_with_query(request_path, headers, method, None, None, None, None)
            .await
    }

    pub async fn serve_with_query(
        &self,
        request_path: &str,
        headers: &HeaderMap,
        method: &Method,
        page: Option<usize>,
        sort: Option<&str>,
        order: Option<&str>,
        search: Option<&str>,
    ) -> Response {
        self.metrics.increment_requests();

        if self.hotlink_protector.is_enabled() {
            let referer = headers.get(header::REFERER).and_then(|v| v.to_str().ok());
            match self.hotlink_protector.check(referer) {
                HotlinkResult::Denied => {
                    if let Some(fallback) = self.hotlink_protector.fallback_url() {
                        return Response::builder()
                            .status(StatusCode::FOUND)
                            .header(header::LOCATION, fallback)
                            .body(Body::empty())
                            .unwrap();
                    } else {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                }
                HotlinkResult::Allowed => {}
            }
        }

        let file_path = match self.resolve_path(request_path) {
            Ok(p) => p,
            Err(e) => {
                return self
                    .handle_path_error_async(e, request_path, headers, method)
                    .await;
            }
        };

        if file_path.is_dir() {
            return self
                .handle_directory(request_path, &file_path, headers, method, page, sort, order, search)
                .await;
        }

        self.serve_file(&file_path, headers, method).await
    }

    fn resolve_path(&self, request_path: &str) -> Result<PathBuf, SecurityError> {
        let mut path = request_path.to_string();
        if path.is_empty() || path == "/" {
            path = format!("/{}", self.index_file);
        }

        self.path_validator.validate_and_resolve(&path)
    }

    fn handle_path_error(&self, error: SecurityError, _request_path: &str, _method: &Method) -> Response {
        match error {
            SecurityError::NotFound => StatusCode::NOT_FOUND.into_response(),
            SecurityError::PathTraversal | SecurityError::PathTooDeep => {
                StatusCode::FORBIDDEN.into_response()
            }
            SecurityError::HiddenFile | SecurityError::SymlinkDenied => {
                StatusCode::NOT_FOUND.into_response()
            }
            SecurityError::InvalidPath => StatusCode::BAD_REQUEST.into_response(),
        }
    }

    async fn handle_path_error_async(
        &self,
        error: SecurityError,
        request_path: &str,
        headers: &HeaderMap,
        method: &Method,
    ) -> Response {
        match error {
            SecurityError::NotFound => {
                if self.spa_fallback && !has_extension(request_path) {
                    let index_path = self.path_validator.root_dir().join(&self.index_file);
                    if index_path.exists() && index_path.is_file() {
                        return self.serve_file(&index_path, headers, method).await;
                    }
                }
                StatusCode::NOT_FOUND.into_response()
            }
            _ => self.handle_path_error(error, request_path, method),
        }
    }

    async fn handle_directory(
        &self,
        request_path: &str,
        dir_path: &Path,
        headers: &HeaderMap,
        method: &Method,
        page: Option<usize>,
        sort: Option<&str>,
        order: Option<&str>,
        search: Option<&str>,
    ) -> Response {
        let index_path = dir_path.join(&self.index_file);
        if index_path.exists() && index_path.is_file() {
            return self.serve_file(&index_path, headers, method).await;
        }

        if self.directory_listing {
            return self
                .list_directory(request_path, dir_path, headers, page, sort, order, search)
                .await;
        }

        if self.spa_fallback && !has_extension(request_path) {
            let index_path = format!("/{}", self.index_file);
            if let Ok(path) = self.path_validator.validate_and_resolve(&index_path) {
                return self.serve_file(&path, headers, method).await;
            }
        }

        StatusCode::NOT_FOUND.into_response()
    }

    async fn list_directory(
        &self,
        request_path: &str,
        dir_path: &Path,
        _headers: &HeaderMap,
        page: Option<usize>,
        sort: Option<&str>,
        order: Option<&str>,
        search: Option<&str>,
    ) -> Response {
        let page = page.unwrap_or(1).max(1);
        let sort_by = match sort {
            Some(s) if matches!(s, "name" | "size" | "time") => s,
            _ => "name",
        };
        let sort_order = match order {
            Some(o) if matches!(o, "asc" | "desc") => o,
            _ => "asc",
        };
        self.directory_lister.list_directory_html(
            request_path,
            dir_path,
            page,
            self.directory_lister.config.directory_page_size,
            sort_by,
            sort_order,
            search,
        )
    }

    pub async fn serve_file(&self, path: &Path, headers: &HeaderMap, method: &Method) -> Response {
        debug!("Serving file: {:?}", path);

        let metadata = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(_) => return StatusCode::NOT_FOUND.into_response(),
        };

        if !metadata.is_file() {
            return StatusCode::NOT_FOUND.into_response();
        }

        let mime = guess_mime(path);
        let etag = generate_etag(&metadata);
        let last_modified = http_date(metadata.modified().unwrap_or_else(|_| SystemTime::now()));

        if self.check_not_modified(headers, &etag, &metadata) {
            self.metrics.increment_cache_hits();
            let mut response = Response::new(Body::empty());
            *response.status_mut() = StatusCode::NOT_MODIFIED;
            let resp_headers = response.headers_mut();
            resp_headers.insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
            resp_headers.insert(
                header::LAST_MODIFIED,
                HeaderValue::from_str(&last_modified).unwrap(),
            );
            return response;
        }

        self.metrics.increment_cache_misses();

        let range_header = headers.get(header::RANGE).and_then(|v| v.to_str().ok());

        if method == Method::HEAD {
            return self.build_head_response(path, &metadata, &mime, &etag, &last_modified);
        }

        if let Some(range_str) = range_header {
            self.metrics.increment_range_requests();
            return self
                .serve_range(path, &metadata, &mime, &etag, &last_modified, range_str)
                .await;
        }

        let accept_encoding = headers
            .get(header::ACCEPT_ENCODING)
            .and_then(|v| v.to_str().ok());

        if self.compression_config.enabled
            && is_compressible(&mime)
            && metadata.len() >= self.compression_config.min_size as u64
        {
            if let Some(encoding) = self.negotiate_encoding(path, accept_encoding) {
                self.metrics.increment_compressed_responses();
                if encoding.ends_with("-pre") {
                    let enc = encoding.trim_end_matches("-pre");
                    return self
                        .serve_precompressed(path, &mime, &etag, &last_modified, enc)
                        .await;
                } else {
                    return self
                        .serve_realtime_compressed(path, &metadata, &mime, &etag, &last_modified, encoding)
                        .await;
                }
            }
        }

        self.serve_full_file(path, &metadata, &mime, &etag, &last_modified)
            .await
    }

    fn check_not_modified(
        &self,
        headers: &HeaderMap,
        etag: &str,
        metadata: &Metadata,
    ) -> bool {
        if let Some(if_none_match) = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
        {
            let etags: Vec<&str> = if_none_match.split(',').map(|s| s.trim()).collect();
            for e in etags {
                if e == "*" || e == etag || e == format!("W/{}", etag).as_str() {
                    return true;
                }
            }
            return false;
        }

        if let Some(if_modified_since) = headers
            .get(header::IF_MODIFIED_SINCE)
            .and_then(|v| v.to_str().ok())
        {
            if let Ok(since) = parse_http_date(if_modified_since) {
                if let Ok(modified) = metadata.modified() {
                    let modified_secs = modified
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let since_secs = since
                        .duration_since(SystemTime::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    if modified_secs <= since_secs {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn negotiate_encoding(&self, path: &Path, accept_encoding: Option<&str>) -> Option<&'static str> {
        let accept = accept_encoding.unwrap_or("");
        let accept_lower = accept.to_lowercase();

        if self.compression_config.precompressed {
            if accept_lower.contains("br") {
                let br_path = PathBuf::from(format!("{}.br", path.display()));
                if br_path.exists() {
                    return Some("br-pre");
                }
            }
            if accept_lower.contains("gzip") {
                let gz_path = PathBuf::from(format!("{}.gz", path.display()));
                if gz_path.exists() {
                    return Some("gzip-pre");
                }
            }
        }

        if accept_lower.contains("br") {
            return Some("br");
        }
        if accept_lower.contains("gzip") {
            return Some("gzip");
        }
        if accept_lower.contains("deflate") {
            return Some("deflate");
        }

        None
    }

    fn build_head_response(
        &self,
        path: &Path,
        metadata: &Metadata,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
    ) -> Response {
        let cache_directives = self.cache_policy.get_cache_control(path);
        let content_length = metadata.len();

        let mut response = Response::new(Body::empty());
        let headers = response.headers_mut();

        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(mime.as_ref()).unwrap(),
        );
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from(content_length),
        );
        headers.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
        headers.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_str(last_modified).unwrap(),
        );
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_str(&cache_directives.to_header_value()).unwrap(),
        );
        headers.insert(
            header::ACCEPT_RANGES,
            HeaderValue::from_static("bytes"),
        );

        response
    }

    async fn serve_full_file(
        &self,
        path: &Path,
        metadata: &Metadata,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
    ) -> Response {
        let cache_directives = self.cache_policy.get_cache_control(path);
        let content_length = metadata.len();

        match File::open(path).await {
            Ok(file) => {
                let stream = ReaderStream::new(file);
                let body = Body::from_stream(stream);

                let mut response = Response::new(body);
                let headers = response.headers_mut();

                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref()).unwrap(),
                );
                headers.insert(
                    header::CONTENT_LENGTH,
                    HeaderValue::from(content_length),
                );
                headers.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
                headers.insert(
                    header::LAST_MODIFIED,
                    HeaderValue::from_str(last_modified).unwrap(),
                );
                headers.insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_str(&cache_directives.to_header_value()).unwrap(),
                );
                headers.insert(
                    header::ACCEPT_RANGES,
                    HeaderValue::from_static("bytes"),
                );

                self.metrics.add_bytes_sent(content_length);

                response
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }

    async fn serve_precompressed(
        &self,
        original_path: &Path,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
        encoding: &'static str,
    ) -> Response {
        let compressed_path = match encoding {
            "br" => PathBuf::from(format!("{}.br", original_path.display())),
            "gzip" => PathBuf::from(format!("{}.gz", original_path.display())),
            _ => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };

        let compressed_meta = match std::fs::metadata(&compressed_path) {
            Ok(m) => m,
            Err(_) => {
                return self
                    .serve_full_file(
                        original_path,
                        &std::fs::metadata(original_path).unwrap(),
                        mime,
                        etag,
                        last_modified,
                    )
                    .await
            }
        };

        let cache_directives = self.cache_policy.get_cache_control(original_path);
        let content_length = compressed_meta.len();

        match File::open(&compressed_path).await {
            Ok(file) => {
                let stream = ReaderStream::new(file);
                let body = Body::from_stream(stream);

                let mut response = Response::new(body);
                let headers = response.headers_mut();

                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref()).unwrap(),
                );
                headers.insert(
                    header::CONTENT_LENGTH,
                    HeaderValue::from(content_length),
                );
                headers.insert(
                    header::CONTENT_ENCODING,
                    HeaderValue::from_static(encoding),
                );
                headers.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
                headers.insert(
                    header::LAST_MODIFIED,
                    HeaderValue::from_str(last_modified).unwrap(),
                );
                headers.insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_str(&cache_directives.to_header_value()).unwrap(),
                );
                headers.insert(
                    header::ACCEPT_RANGES,
                    HeaderValue::from_static("none"),
                );
                headers.insert(
                    header::VARY,
                    HeaderValue::from_static("Accept-Encoding"),
                );

                self.metrics.add_bytes_sent(content_length);

                response
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }

    async fn serve_realtime_compressed(
        &self,
        path: &Path,
        metadata: &Metadata,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
        encoding: &'static str,
    ) -> Response {
        let mut file = match File::open(path).await {
            Ok(f) => f,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };

        let mut content = Vec::with_capacity(metadata.len() as usize);
        if let Err(_) = file.read_to_end(&mut content).await {
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }

        let compressed = match self.compress_bytes(&content, encoding).await {
            Ok(c) => c,
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };

        let cache_directives = self.cache_policy.get_cache_control(path);
        let content_length = compressed.len() as u64;

        let mut response = Response::new(Body::from(compressed));
        let headers = response.headers_mut();

        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(mime.as_ref()).unwrap(),
        );
        headers.insert(
            header::CONTENT_LENGTH,
            HeaderValue::from(content_length),
        );
        headers.insert(
            header::CONTENT_ENCODING,
            HeaderValue::from_static(encoding),
        );
        headers.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
        headers.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_str(last_modified).unwrap(),
        );
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_str(&cache_directives.to_header_value()).unwrap(),
        );
        headers.insert(
            header::ACCEPT_RANGES,
            HeaderValue::from_static("none"),
        );
        headers.insert(
            header::VARY,
            HeaderValue::from_static("Accept-Encoding"),
        );

        self.metrics.add_bytes_sent(content_length);

        response
    }

    async fn compress_bytes(&self, data: &[u8], encoding: &str) -> Result<Vec<u8>, std::io::Error> {
        match encoding {
            "gzip" => {
                let mut encoder = GzipEncoder::with_quality(
                    Vec::new(),
                    async_compression::Level::Precise(self.compression_config.gzip_level as i32),
                );
                encoder.write_all(data).await?;
                encoder.shutdown().await?;
                Ok(encoder.into_inner())
            }
            "br" => {
                let mut encoder = BrotliEncoder::with_quality(
                    Vec::new(),
                    async_compression::Level::Precise(self.compression_config.brotli_level as i32),
                );
                encoder.write_all(data).await?;
                encoder.shutdown().await?;
                Ok(encoder.into_inner())
            }
            "deflate" => {
                let mut encoder = DeflateEncoder::with_quality(
                    Vec::new(),
                    async_compression::Level::Precise(self.compression_config.deflate_level as i32),
                );
                encoder.write_all(data).await?;
                encoder.shutdown().await?;
                Ok(encoder.into_inner())
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "unsupported encoding",
            )),
        }
    }

    async fn serve_range(
        &self,
        path: &Path,
        metadata: &Metadata,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
        range_str: &str,
    ) -> Response {
        let file_size = metadata.len();

        let ranges = match parse_range_header(range_str, file_size) {
            Ok(ranges) => ranges,
            Err(RangeError::Invalid) => return StatusCode::BAD_REQUEST.into_response(),
            Err(RangeError::Unsatisfiable) => {
                let mut resp = Response::new(Body::empty());
                *resp.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
                resp.headers_mut().insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes */{}", file_size)).unwrap(),
                );
                return resp;
            }
        };

        if ranges.len() == 1 {
            return self
                .serve_single_range(path, &ranges[0], file_size, mime, etag, last_modified)
                .await;
        }

        self.serve_multiple_ranges(path, &ranges, file_size, mime, etag, last_modified)
            .await
    }

    async fn serve_single_range(
        &self,
        path: &Path,
        range: &ByteRange,
        file_size: u64,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
    ) -> Response {
        let cache_directives = self.cache_policy.get_cache_control(path);
        let length = range.length();

        match File::open(path).await {
            Ok(mut file) => {
                if let Err(_) = file.seek(SeekFrom::Start(range.start)).await {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }

                let limited = file.take(length);
                let stream = ReaderStream::new(limited);
                let body = Body::from_stream(stream);

                let mut response = Response::new(body);
                *response.status_mut() = StatusCode::PARTIAL_CONTENT;
                let headers = response.headers_mut();

                headers.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(mime.as_ref()).unwrap(),
                );
                headers.insert(
                    header::CONTENT_LENGTH,
                    HeaderValue::from(length),
                );
                headers.insert(
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!(
                        "bytes {}-{}/{}",
                        range.start,
                        range.end,
                        file_size
                    ))
                    .unwrap(),
                );
                headers.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
                headers.insert(
                    header::LAST_MODIFIED,
                    HeaderValue::from_str(last_modified).unwrap(),
                );
                headers.insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_str(&cache_directives.to_header_value()).unwrap(),
                );
                headers.insert(
                    header::ACCEPT_RANGES,
                    HeaderValue::from_static("bytes"),
                );

                self.metrics.add_bytes_sent(length);

                response
            }
            Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }

    async fn serve_multiple_ranges(
        &self,
        path: &Path,
        ranges: &[ByteRange],
        file_size: u64,
        mime: &mime::Mime,
        etag: &str,
        last_modified: &str,
    ) -> Response {
        let boundary = generate_boundary();
        let content_type = format!("multipart/byteranges; boundary={}", boundary);
        let ranges_vec = ranges.to_vec();
        let path_buf = path.to_path_buf();
        let mime_str = mime.to_string();

        let stream = build_multipart_stream(path_buf, ranges_vec, boundary, mime_str, file_size);

        let cache_directives = self.cache_policy.get_cache_control(path);
        let body = Body::from_stream(stream);

        let mut response = Response::new(body);
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        let headers = response.headers_mut();

        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type).unwrap(),
        );
        headers.insert(header::ETAG, HeaderValue::from_str(etag).unwrap());
        headers.insert(
            header::LAST_MODIFIED,
            HeaderValue::from_str(last_modified).unwrap(),
        );
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_str(&cache_directives.to_header_value()).unwrap(),
        );
        headers.insert(
            header::ACCEPT_RANGES,
            HeaderValue::from_static("bytes"),
        );

        response
    }
}

fn has_extension(path: &str) -> bool {
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    last_segment.contains('.')
}

fn generate_etag(metadata: &Metadata) -> String {
    let size = metadata.len();
    let mtime = metadata
        .modified()
        .unwrap_or_else(|_| SystemTime::UNIX_EPOCH);
    let mtime_secs = mtime
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("\"{:x}-{:x}\"", mtime_secs, size)
}

fn http_date(time: SystemTime) -> String {
    let dur = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;

    let days = secs / 86400;
    let weekday = ((days + 4) % 7 + 7) % 7;
    let weekday_names = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let weekday_name = weekday_names[weekday as usize];

    let mut day_of_year = days;
    let mut year = 1970;
    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if day_of_year < days_in_year {
            break;
        }
        day_of_year -= days_in_year;
        year += 1;
    }

    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut month = 0;
    let mut day = day_of_year;
    while month < 12 && day >= month_days[month] as i64 {
        day -= month_days[month] as i64;
        month += 1;
    }

    let month_names = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month_name = month_names[month];

    let secs_of_day = (secs % 86400) as u32;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        weekday_name,
        day + 1,
        month_name,
        year,
        hour,
        minute,
        second
    )
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn parse_http_date(s: &str) -> Result<SystemTime, ()> {
    let s = s.trim();

    if let Some(dt) = parse_rfc1123(s) {
        return Ok(dt);
    }
    if let Some(dt) = parse_rfc850(s) {
        return Ok(dt);
    }
    if let Some(dt) = parse_asctime(s) {
        return Ok(dt);
    }

    Err(())
}

fn parse_rfc1123(s: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() < 5 {
        return None;
    }

    let day: u32 = parts[1].parse().ok()?;
    let month = month_to_num(parts[2])?;
    let year: i32 = parts[3].parse().ok()?;

    let time_parts: Vec<&str> = parts[4].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u32 = time_parts[0].parse().ok()?;
    let minute: u32 = time_parts[1].parse().ok()?;
    let second: u32 = time_parts[2].parse().ok()?;

    Some(datetime_to_system_time(year, month, day, hour, minute, second))
}

fn parse_rfc850(s: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 4 {
        return None;
    }

    let date_parts: Vec<&str> = parts[1].split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }

    let day: u32 = date_parts[0].parse().ok()?;
    let month = month_to_num(date_parts[1])?;
    let year_str = date_parts[2];
    let year: i32 = if year_str.len() == 2 {
        let y: i32 = year_str.parse().ok()?;
        if y >= 70 { 1900 + y } else { 2000 + y }
    } else {
        year_str.parse().ok()?
    };

    let time_parts: Vec<&str> = parts[2].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u32 = time_parts[0].parse().ok()?;
    let minute: u32 = time_parts[1].parse().ok()?;
    let second: u32 = time_parts[2].parse().ok()?;

    Some(datetime_to_system_time(year, month, day, hour, minute, second))
}

fn parse_asctime(s: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }

    let month = month_to_num(parts[1])?;
    let day: u32 = parts[2].parse().ok()?;

    let time_parts: Vec<&str> = parts[3].split(':').collect();
    if time_parts.len() != 3 {
        return None;
    }
    let hour: u32 = time_parts[0].parse().ok()?;
    let minute: u32 = time_parts[1].parse().ok()?;
    let second: u32 = time_parts[2].parse().ok()?;

    let year: i32 = parts[4].parse().ok()?;

    Some(datetime_to_system_time(year, month, day, hour, minute, second))
}

fn month_to_num(month: &str) -> Option<u32> {
    match month.to_ascii_lowercase().as_str() {
        "jan" => Some(1),
        "feb" => Some(2),
        "mar" => Some(3),
        "apr" => Some(4),
        "may" => Some(5),
        "jun" => Some(6),
        "jul" => Some(7),
        "aug" => Some(8),
        "sep" => Some(9),
        "oct" => Some(10),
        "nov" => Some(11),
        "dec" => Some(12),
        _ => None,
    }
}

fn datetime_to_system_time(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> SystemTime {
    let mut days = 0;

    for y in 1970..year {
        days += if is_leap_year(y) { 366 } else { 365 };
    }

    let month_days = if is_leap_year(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    for m in 0..month.saturating_sub(1) as usize {
        days += month_days[m] as i64;
    }

    days += day.saturating_sub(1) as i64;

    let secs = days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;

    SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(secs.max(0) as u64)
}

#[derive(Debug, Clone, Copy)]
struct ByteRange {
    start: u64,
    end: u64,
}

impl ByteRange {
    fn length(&self) -> u64 {
        self.end - self.start + 1
    }
}

#[derive(Debug)]
enum RangeError {
    Invalid,
    Unsatisfiable,
}

fn parse_range_header(header: &str, file_size: u64) -> Result<Vec<ByteRange>, RangeError> {
    let header = header.trim();

    let suffix = header.strip_prefix("bytes=").ok_or(RangeError::Invalid)?;

    if suffix.is_empty() {
        return Err(RangeError::Invalid);
    }

    let mut ranges = Vec::new();

    for part in suffix.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(RangeError::Invalid);
        }

        if let Some(suffix_len) = part.strip_prefix('-') {
            let suffix_len: u64 = suffix_len.parse().map_err(|_| RangeError::Invalid)?;
            if suffix_len == 0 || suffix_len > file_size {
                return Err(RangeError::Unsatisfiable);
            }
            let start = file_size - suffix_len;
            ranges.push(ByteRange {
                start,
                end: file_size - 1,
            });
        } else if let Some(start_str) = part.strip_suffix('-') {
            let start: u64 = start_str.parse().map_err(|_| RangeError::Invalid)?;
            if start >= file_size {
                return Err(RangeError::Unsatisfiable);
            }
            ranges.push(ByteRange {
                start,
                end: file_size - 1,
            });
        } else {
            let parts: Vec<&str> = part.splitn(2, '-').collect();
            if parts.len() != 2 {
                return Err(RangeError::Invalid);
            }
            let start: u64 = parts[0].parse().map_err(|_| RangeError::Invalid)?;
            let end: u64 = parts[1].parse().map_err(|_| RangeError::Invalid)?;

            if start > end || start >= file_size {
                return Err(RangeError::Unsatisfiable);
            }

            let end = std::cmp::min(end, file_size - 1);

            ranges.push(ByteRange { start, end });
        }
    }

    if ranges.is_empty() {
        return Err(RangeError::Invalid);
    }

    Ok(ranges)
}

fn build_multipart_stream(
    path: PathBuf,
    ranges: Vec<ByteRange>,
    boundary: String,
    mime: String,
    file_size: u64,
) -> impl futures::Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::try_stream! {
        for range in ranges {
            let boundary_line = format!("--{}\r\n", boundary);
            yield Bytes::from(boundary_line);

            let content_type_line = format!("Content-Type: {}\r\n", mime);
            yield Bytes::from(content_type_line);

            let content_range_line = format!(
                "Content-Range: bytes {}-{}/{}\r\n\r\n",
                range.start, range.end, file_size
            );
            yield Bytes::from(content_range_line);

            let mut file = File::open(&path).await?;

            file.seek(SeekFrom::Start(range.start)).await?;

            let mut remaining = range.length();
            let mut buf = vec![0u8; 64 * 1024];

            while remaining > 0 {
                let to_read = std::cmp::min(remaining as usize, buf.len());
                let n = file.read(&mut buf[..to_read]).await?;
                if n == 0 {
                    break;
                }
                remaining -= n as u64;
                yield Bytes::copy_from_slice(&buf[..n]);
            }

            yield Bytes::from("\r\n");
        }

        let end_boundary = format!("--{}--\r\n", boundary);
        yield Bytes::from(end_boundary);
    }
}

fn generate_boundary() -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(32);
    for _ in 0..16 {
        let b = rand_byte();
        let _ = write!(s, "{:02x}", b);
    }
    s
}

fn rand_byte() -> u8 {
    use std::time::{SystemTime, UNIX_EPOCH};
    static mut COUNTER: u64 = 0;
    unsafe {
        COUNTER = COUNTER.wrapping_add(1);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        (now.wrapping_mul(COUNTER)).to_le_bytes()[0]
    }
}
