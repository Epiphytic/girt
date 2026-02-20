use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Trait for metrics backends. Implementations can forward to Prometheus,
/// StatsD, or simply log metrics.
pub trait MetricsBackend: Send + Sync {
    fn record_counter(&self, name: &str, value: u64);
    fn record_gauge(&self, name: &str, value: f64);
    fn record_histogram(&self, name: &str, value: f64);
}

/// In-memory metrics collector with atomic counters.
/// Thread-safe for concurrent pipeline operations.
pub struct PipelineMetrics {
    pub builds_started: AtomicU64,
    pub builds_completed: AtomicU64,
    pub builds_failed: AtomicU64,
    pub circuit_breaker_triggers: AtomicU64,
    pub cache_hits: AtomicU64,
    pub cache_misses: AtomicU64,
    pub total_build_iterations: AtomicU64,
    pub recommend_extend_count: AtomicU64,
    backend: Option<Arc<dyn MetricsBackend>>,
}

impl std::fmt::Debug for PipelineMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineMetrics")
            .field("builds_started", &self.builds_started)
            .field("builds_completed", &self.builds_completed)
            .field("builds_failed", &self.builds_failed)
            .field("circuit_breaker_triggers", &self.circuit_breaker_triggers)
            .field("cache_hits", &self.cache_hits)
            .field("cache_misses", &self.cache_misses)
            .field("total_build_iterations", &self.total_build_iterations)
            .field("recommend_extend_count", &self.recommend_extend_count)
            .finish()
    }
}

impl Default for PipelineMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl PipelineMetrics {
    pub fn new() -> Self {
        Self {
            builds_started: AtomicU64::new(0),
            builds_completed: AtomicU64::new(0),
            builds_failed: AtomicU64::new(0),
            circuit_breaker_triggers: AtomicU64::new(0),
            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
            total_build_iterations: AtomicU64::new(0),
            recommend_extend_count: AtomicU64::new(0),
            backend: None,
        }
    }

    pub fn with_backend(backend: Arc<dyn MetricsBackend>) -> Self {
        Self {
            backend: Some(backend),
            ..Self::new()
        }
    }

    pub fn record_build_started(&self) {
        let val = self.builds_started.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.builds_started", val);
        }
    }

    pub fn record_build_completed(&self, iterations: u32) {
        let val = self.builds_completed.fetch_add(1, Ordering::Relaxed) + 1;
        self.total_build_iterations
            .fetch_add(u64::from(iterations), Ordering::Relaxed);
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.builds_completed", val);
            backend.record_histogram("girt.pipeline.build_iterations", f64::from(iterations));
        }
    }

    pub fn record_build_failed(&self) {
        let val = self.builds_failed.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.builds_failed", val);
        }
    }

    pub fn record_circuit_breaker(&self) {
        let val = self
            .circuit_breaker_triggers
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.circuit_breaker_triggers", val);
        }
    }

    pub fn record_cache_hit(&self) {
        let val = self.cache_hits.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.cache_hits", val);
        }
    }

    pub fn record_cache_miss(&self) {
        let val = self.cache_misses.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.cache_misses", val);
        }
    }

    pub fn record_recommend_extend(&self) {
        let val = self.recommend_extend_count.fetch_add(1, Ordering::Relaxed) + 1;
        if let Some(backend) = &self.backend {
            backend.record_counter("girt.pipeline.recommend_extend", val);
        }
    }

    /// Get a snapshot of all metrics.
    pub fn snapshot(&self) -> MetricsSnapshot {
        MetricsSnapshot {
            builds_started: self.builds_started.load(Ordering::Relaxed),
            builds_completed: self.builds_completed.load(Ordering::Relaxed),
            builds_failed: self.builds_failed.load(Ordering::Relaxed),
            circuit_breaker_triggers: self.circuit_breaker_triggers.load(Ordering::Relaxed),
            cache_hits: self.cache_hits.load(Ordering::Relaxed),
            cache_misses: self.cache_misses.load(Ordering::Relaxed),
            total_build_iterations: self.total_build_iterations.load(Ordering::Relaxed),
            recommend_extend_count: self.recommend_extend_count.load(Ordering::Relaxed),
        }
    }
}

/// A point-in-time snapshot of pipeline metrics.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MetricsSnapshot {
    pub builds_started: u64,
    pub builds_completed: u64,
    pub builds_failed: u64,
    pub circuit_breaker_triggers: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub total_build_iterations: u64,
    pub recommend_extend_count: u64,
}

/// Logging-based metrics backend. Emits metrics as structured log events.
pub struct LoggingMetricsBackend;

impl MetricsBackend for LoggingMetricsBackend {
    fn record_counter(&self, name: &str, value: u64) {
        tracing::info!(metric = name, value = value, kind = "counter", "metric");
    }

    fn record_gauge(&self, name: &str, value: f64) {
        tracing::info!(metric = name, value = value, kind = "gauge", "metric");
    }

    fn record_histogram(&self, name: &str, value: f64) {
        tracing::info!(metric = name, value = value, kind = "histogram", "metric");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_metrics_start_at_zero() {
        let metrics = PipelineMetrics::new();
        let snap = metrics.snapshot();
        assert_eq!(snap.builds_started, 0);
        assert_eq!(snap.builds_completed, 0);
        assert_eq!(snap.builds_failed, 0);
    }

    #[test]
    fn counters_increment_correctly() {
        let metrics = PipelineMetrics::new();
        metrics.record_build_started();
        metrics.record_build_started();
        metrics.record_build_completed(3);
        metrics.record_build_failed();
        metrics.record_circuit_breaker();

        let snap = metrics.snapshot();
        assert_eq!(snap.builds_started, 2);
        assert_eq!(snap.builds_completed, 1);
        assert_eq!(snap.builds_failed, 1);
        assert_eq!(snap.circuit_breaker_triggers, 1);
        assert_eq!(snap.total_build_iterations, 3);
    }

    #[test]
    fn cache_metrics_track_hits_and_misses() {
        let metrics = PipelineMetrics::new();
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();

        let snap = metrics.snapshot();
        assert_eq!(snap.cache_hits, 2);
        assert_eq!(snap.cache_misses, 1);
    }

    #[test]
    fn with_logging_backend() {
        let backend = Arc::new(LoggingMetricsBackend);
        let metrics = PipelineMetrics::with_backend(backend);
        metrics.record_build_started();
        assert_eq!(metrics.snapshot().builds_started, 1);
    }

    #[test]
    fn recommend_extend_counter() {
        let metrics = PipelineMetrics::new();
        metrics.record_recommend_extend();
        metrics.record_recommend_extend();
        assert_eq!(metrics.snapshot().recommend_extend_count, 2);
    }

    #[test]
    fn concurrent_increments() {
        let metrics = Arc::new(PipelineMetrics::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let m = Arc::clone(&metrics);
            handles.push(std::thread::spawn(move || {
                for _ in 0..100 {
                    m.record_build_started();
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(metrics.snapshot().builds_started, 1000);
    }
}
