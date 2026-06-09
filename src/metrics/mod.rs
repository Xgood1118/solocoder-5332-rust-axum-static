use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;

#[derive(Debug, Clone)]
pub struct ServerMetrics {
    inner: Arc<MetricsInner>,
}

#[derive(Debug)]
struct MetricsInner {
    start_time: Instant,
    total_requests: AtomicU64,
    active_connections: AtomicU64,
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    status_counts: DashMap<u16, u64>,
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
    range_requests: AtomicU64,
    compressed_responses: AtomicU64,
}

impl ServerMetrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(MetricsInner {
                start_time: Instant::now(),
                total_requests: AtomicU64::new(0),
                active_connections: AtomicU64::new(0),
                bytes_sent: AtomicU64::new(0),
                bytes_received: AtomicU64::new(0),
                status_counts: DashMap::new(),
                cache_hits: AtomicU64::new(0),
                cache_misses: AtomicU64::new(0),
                range_requests: AtomicU64::new(0),
                compressed_responses: AtomicU64::new(0),
            }),
        }
    }

    pub fn start_time(&self) -> Instant {
        self.inner.start_time
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.inner.start_time.elapsed().as_secs()
    }

    pub fn total_requests(&self) -> u64 {
        self.inner.total_requests.load(Ordering::Relaxed)
    }

    pub fn increment_requests(&self) {
        self.inner.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn active_connections(&self) -> u64 {
        self.inner.active_connections.load(Ordering::Relaxed)
    }

    pub fn increment_connections(&self) {
        self.inner.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    pub fn decrement_connections(&self) {
        self.inner.active_connections.fetch_sub(1, Ordering::Relaxed);
    }

    pub fn bytes_sent(&self) -> u64 {
        self.inner.bytes_sent.load(Ordering::Relaxed)
    }

    pub fn add_bytes_sent(&self, bytes: u64) {
        self.inner.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn record_status(&self, status: u16) {
        let mut entry = self.inner.status_counts.entry(status).or_insert(0);
        *entry += 1;
    }

    pub fn cache_hits(&self) -> u64 {
        self.inner.cache_hits.load(Ordering::Relaxed)
    }

    pub fn increment_cache_hits(&self) {
        self.inner.cache_hits.fetch_add(1, Ordering::Relaxed);
    }

    pub fn cache_misses(&self) -> u64 {
        self.inner.cache_misses.load(Ordering::Relaxed)
    }

    pub fn increment_cache_misses(&self) {
        self.inner.cache_misses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_range_requests(&self) {
        self.inner.range_requests.fetch_add(1, Ordering::Relaxed);
    }

    pub fn increment_compressed_responses(&self) {
        self.inner.compressed_responses.fetch_add(1, Ordering::Relaxed);
    }

    pub fn to_health_json(&self) -> serde_json::Value {
        let uptime = self.uptime_seconds();
        let hours = uptime / 3600;
        let minutes = (uptime % 3600) / 60;
        let seconds = uptime % 60;

        serde_json::json!({
            "status": "healthy",
            "uptime": {
                "seconds": uptime,
                "formatted": format!("{}h {}m {}s", hours, minutes, seconds)
            },
            "total_requests": self.total_requests(),
            "active_connections": self.active_connections(),
            "bytes_sent": self.bytes_sent(),
            "cache": {
                "hits": self.cache_hits(),
                "misses": self.cache_misses(),
                "hit_rate": if self.cache_hits() + self.cache_misses() > 0 {
                    (self.cache_hits() as f64 / (self.cache_hits() + self.cache_misses()) as f64 * 100.0) as u64
                } else {
                    0
                }
            }
        })
    }
}

impl Default for ServerMetrics {
    fn default() -> Self {
        Self::new()
    }
}
