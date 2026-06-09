use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use axum::body::Body;
use axum::http::header;
use axum::response::Response;
use serde::Serialize;

use crate::config::ServerConfig;
use crate::mime_util::guess_mime;
use crate::security::PathValidator;

#[derive(Debug, Clone)]
pub struct DirectoryLister {
    pub config: Arc<ServerConfig>,
    path_validator: PathValidator,
}

#[derive(Debug, Serialize)]
struct DirectoryEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: i64,
    modified_rfc3339: String,
    mime_type: String,
}

#[derive(Debug, Serialize)]
struct DirectoryListing {
    path: String,
    entries: Vec<DirectoryEntry>,
    total: usize,
    page: usize,
    page_size: usize,
    has_more: bool,
    sort_by: String,
    sort_order: String,
}

impl DirectoryLister {
    pub fn new(config: Arc<ServerConfig>, path_validator: PathValidator) -> Self {
        Self {
            config,
            path_validator,
        }
    }

    pub fn list_directory_html(
        &self,
        request_path: &str,
        dir_path: &Path,
        page: usize,
        page_size: usize,
        sort_by: &str,
        sort_order: &str,
        search: Option<&str>,
    ) -> Response {
        let listing = match self.list_directory(
            request_path,
            dir_path,
            page,
            page_size,
            sort_by,
            sort_order,
            search,
        ) {
            Ok(l) => l,
            Err(_) => return Response::builder()
                .status(500)
                .body(Body::from("Failed to list directory"))
                .unwrap(),
        };

        let html = render_directory_html(&listing);

        Response::builder()
            .status(200)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .body(Body::from(html))
            .unwrap()
    }

    fn list_directory(
        &self,
        request_path: &str,
        dir_path: &Path,
        page: usize,
        page_size: usize,
        sort_by: &str,
        sort_order: &str,
        search: Option<&str>,
    ) -> std::io::Result<DirectoryListing> {
        let mut entries: Vec<DirectoryEntry> = Vec::new();

        for entry in std::fs::read_dir(dir_path)? {
            let entry = entry?;
            let file_name = entry.file_name();
            let name = file_name.to_string_lossy().to_string();

            if !self.config.serve_hidden_files && name.starts_with('.') {
                continue;
            }

            if let Some(search_term) = search {
                if !name.to_lowercase().contains(&search_term.to_lowercase()) {
                    continue;
                }
            }

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

            let is_dir = metadata.file_type().is_dir();

            if !self.config.follow_symlinks && metadata.file_type().is_symlink() {
                continue;
            }

            let size = if is_dir { 0 } else { metadata.len() };

            let modified = metadata
                .modified()
                .unwrap_or_else(|_| SystemTime::UNIX_EPOCH);

            let modified_secs = modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            let modified_rfc3339 = format_datetime_rfc3339(modified);

            let mime_type = if is_dir {
                "directory".to_string()
            } else {
                guess_mime(&entry.path()).to_string()
            };

            let entry_path = if request_path.ends_with('/') {
                format!("{}{}", request_path, name)
            } else {
                format!("{}/{}", request_path, name)
            };

            entries.push(DirectoryEntry {
                name,
                path: entry_path,
                is_dir,
                size,
                modified: modified_secs,
                modified_rfc3339,
                mime_type,
            });
        }

        sort_entries(&mut entries, sort_by, sort_order);

        let total = entries.len();
        let start = page.saturating_sub(1) * page_size;
        let has_more = start + page_size < total;
        let entries: Vec<_> = entries.into_iter().skip(start).take(page_size).collect();

        Ok(DirectoryListing {
            path: request_path.to_string(),
            entries,
            total,
            page,
            page_size,
            has_more,
            sort_by: sort_by.to_string(),
            sort_order: sort_order.to_string(),
        })
    }
}

fn sort_entries(entries: &mut [DirectoryEntry], sort_by: &str, sort_order: &str) {
    let descending = sort_order == "desc";

    match sort_by {
        "name" => entries.sort_by(|a, b| {
            let cmp = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            if descending { cmp.reverse() } else { cmp }
        }),
        "size" => entries.sort_by(|a, b| {
            let cmp = a.size.cmp(&b.size);
            if descending { cmp.reverse() } else { cmp }
        }),
        "time" => entries.sort_by(|a, b| {
            let cmp = a.modified.cmp(&b.modified);
            if descending { cmp.reverse() } else { cmp }
        }),
        _ => entries.sort_by(|a, b| {
            let dir_cmp = b.is_dir.cmp(&a.is_dir);
            if dir_cmp != std::cmp::Ordering::Equal {
                return dir_cmp;
            }
            let cmp = a.name.to_lowercase().cmp(&b.name.to_lowercase());
            if descending { cmp.reverse() } else { cmp }
        }),
    }
}

fn format_datetime_rfc3339(time: SystemTime) -> String {
    let dur = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs() as i64;

    let days = secs / 86400;
    let mut year = 1970;
    let mut day_of_year = days;

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

    let secs_of_day = (secs % 86400) as u32;
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day % 3600) / 60;
    let second = secs_of_day % 60;

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year,
        month + 1,
        day + 1,
        hour,
        minute,
        second
    )
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn render_directory_html(listing: &DirectoryListing) -> String {
    let mut html = String::new();

    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    html.push_str("<meta charset=\"UTF-8\">\n");
    html.push_str(&format!(
        "<title>Directory listing for {}</title>\n",
        html_escape(&listing.path)
    ));
    html.push_str("<style>\n");
    html.push_str("body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 20px; }\n");
    html.push_str("h1 { font-size: 1.5em; }\n");
    html.push_str("table { width: 100%; border-collapse: collapse; }\n");
    html.push_str("th, td { text-align: left; padding: 8px 12px; border-bottom: 1px solid #eee; }\n");
    html.push_str("th { background: #f5f5f5; font-weight: 600; }\n");
    html.push_str("a { color: #0366d6; text-decoration: none; }\n");
    html.push_str("a:hover { text-decoration: underline; }\n");
    html.push_str(".size { text-align: right; font-variant-numeric: tabular-nums; }\n");
    html.push_str(".pagination { margin-top: 20px; }\n");
    html.push_str(".search { margin-bottom: 16px; }\n");
    html.push_str("</style>\n");
    html.push_str("</head>\n<body>\n");

    html.push_str(&format!("<h1>Directory listing for {}</h1>\n", html_escape(&listing.path)));

    html.push_str("<div class=\"search\">\n");
    html.push_str(&format!(
        "<form method=\"get\"><input type=\"text\" name=\"q\" placeholder=\"Search...\" value=\"\"> \
         <select name=\"sort\">\
         <option value=\"name\" {}>Name</option>\
         <option value=\"size\" {}>Size</option>\
         <option value=\"time\" {}>Modified</option>\
         </select>\
         <select name=\"order\">\
         <option value=\"asc\" {}>Ascending</option>\
         <option value=\"desc\" {}>Descending</option>\
         </select>\
         <button type=\"submit\">Go</button></form>\n",
        if listing.sort_by == "name" { "selected" } else { "" },
        if listing.sort_by == "size" { "selected" } else { "" },
        if listing.sort_by == "time" { "selected" } else { "" },
        if listing.sort_order == "asc" { "selected" } else { "" },
        if listing.sort_order == "desc" { "selected" } else { "" },
    ));
    html.push_str("</div>\n");

    if listing.path != "/" {
        let parent = get_parent_path(&listing.path);
        html.push_str(&format!(
            "<p><a href=\"{}?sort={}&order={}\">&larr; Parent directory</a></p>\n",
            parent, listing.sort_by, listing.sort_order
        ));
    }

    html.push_str("<table>\n");
    html.push_str("<thead><tr>\n");
    html.push_str("<th>Name</th>\n");
    html.push_str("<th class=\"size\">Size</th>\n");
    html.push_str("<th>Modified</th>\n");
    html.push_str("<th>Type</th>\n");
    html.push_str("</tr></thead>\n");
    html.push_str("<tbody>\n");

    for entry in &listing.entries {
        let size_str = if entry.is_dir {
            "-".to_string()
        } else {
            format_size(entry.size)
        };

        let display_name = if entry.is_dir {
            format!("{}/", entry.name)
        } else {
            entry.name.clone()
        };

        html.push_str("<tr>\n");
        html.push_str(&format!(
            "<td><a href=\"{}\">{}</a></td>\n",
            html_escape(&entry.path),
            html_escape(&display_name)
        ));
        html.push_str(&format!("<td class=\"size\">{}</td>\n", size_str));
        html.push_str(&format!("<td>{}</td>\n", &entry.modified_rfc3339));
        html.push_str(&format!("<td>{}</td>\n", html_escape(&entry.mime_type)));
        html.push_str("</tr>\n");
    }

    html.push_str("</tbody>\n</table>\n");

    html.push_str("<div class=\"pagination\">\n");
    html.push_str(&format!(
        "<p>Showing {} of {} items (page {} of ~{})</p>\n",
        listing.entries.len(),
        listing.total,
        listing.page,
        (listing.total + listing.page_size - 1) / listing.page_size.max(1)
    ));

    if listing.page > 1 {
        html.push_str(&format!(
            "<a href=\"{}?page={}&sort={}&order={}\">&larr; Previous</a> \n",
            html_escape(&listing.path),
            listing.page - 1,
            listing.sort_by,
            listing.sort_order
        ));
    }

    if listing.has_more {
        html.push_str(&format!(
            "<a href=\"{}?page={}&sort={}&order={}\">Next &rarr;</a>\n",
            html_escape(&listing.path),
            listing.page + 1,
            listing.sort_by,
            listing.sort_order
        ));
    }

    html.push_str("</div>\n");
    html.push_str("</body>\n</html>\n");

    html
}

fn format_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut s = size as f64;
    let mut i = 0;
    while s >= 1024.0 && i < UNITS.len() - 1 {
        s /= 1024.0;
        i += 1;
    }
    format!("{:.1} {}", s, UNITS[i])
}

fn html_escape(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '&' => escaped.push_str("&amp;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

fn get_parent_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        return "/".to_string();
    }
    match trimmed.rfind('/') {
        Some(idx) => {
            let parent = &trimmed[..=idx];
            if parent.is_empty() { "/".to_string() } else { parent.to_string() }
        }
        None => "/".to_string(),
    }
}
