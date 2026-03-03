use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Metrics {
    events_received_total: AtomicU64,
    webhook_sent_total: AtomicU64,
    webhook_failed_total: AtomicU64,
    webhook_retry_total: AtomicU64,
    webhook_latency_ms_sum: AtomicU64,
    webhook_latency_ms_count: AtomicU64,
}

impl Metrics {
    pub fn inc_events_received(&self) {
        self.events_received_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_webhook_sent(&self, latency_ms: u64) {
        self.webhook_sent_total.fetch_add(1, Ordering::Relaxed);
        self.webhook_latency_ms_sum
            .fetch_add(latency_ms, Ordering::Relaxed);
        self.webhook_latency_ms_count
            .fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_webhook_failed(&self) {
        self.webhook_failed_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn inc_webhook_retry(&self) {
        self.webhook_retry_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn render_prometheus(&self) -> String {
        let events_received_total = self.events_received_total.load(Ordering::Relaxed);
        let webhook_sent_total = self.webhook_sent_total.load(Ordering::Relaxed);
        let webhook_failed_total = self.webhook_failed_total.load(Ordering::Relaxed);
        let webhook_retry_total = self.webhook_retry_total.load(Ordering::Relaxed);
        let webhook_latency_ms_sum = self.webhook_latency_ms_sum.load(Ordering::Relaxed);
        let webhook_latency_ms_count = self.webhook_latency_ms_count.load(Ordering::Relaxed);

        format!(
            concat!(
                "# TYPE events_received_total counter\n",
                "events_received_total {}\n",
                "# TYPE webhook_sent_total counter\n",
                "webhook_sent_total {}\n",
                "# TYPE webhook_failed_total counter\n",
                "webhook_failed_total {}\n",
                "# TYPE webhook_retry_total counter\n",
                "webhook_retry_total {}\n",
                "# TYPE webhook_latency_ms_sum counter\n",
                "webhook_latency_ms_sum {}\n",
                "# TYPE webhook_latency_ms_count counter\n",
                "webhook_latency_ms_count {}\n",
            ),
            events_received_total,
            webhook_sent_total,
            webhook_failed_total,
            webhook_retry_total,
            webhook_latency_ms_sum,
            webhook_latency_ms_count,
        )
    }
}
