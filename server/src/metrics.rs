use std::time::Instant;

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::state::Shared;
use prometheus::{
    Encoder, Histogram, HistogramVec, IntCounter, IntCounterVec, IntGauge, Registry, TextEncoder,
    histogram_opts, opts, register_histogram_vec_with_registry, register_histogram_with_registry,
    register_int_counter_vec_with_registry, register_int_counter_with_registry,
    register_int_gauge_with_registry,
};

pub struct Metrics {
    registry: Registry,
    pub requests: IntCounterVec,
    pub duration: HistogramVec,
    pub rejections: IntCounterVec,
    pub uploaded_bytes: IntCounter,
    pub downloaded_bytes: IntCounter,
    pub object_size: Histogram,
    pub objects_stored: IntGauge,
    pub store_bytes: IntGauge,
}

const SIZE_BUCKETS: &[f64] = &[
    1_024.0,
    65_536.0,
    1_048_576.0,
    16_777_216.0,
    134_217_728.0,
    1_073_741_824.0,
    8_589_934_592.0,
];

const DURATION_BUCKETS: &[f64] = &[0.005, 0.05, 0.5, 2.0, 10.0, 60.0, 300.0];

impl Metrics {
    pub fn new() -> Self {
        let registry = Registry::new();

        Self {
            registry: registry.clone(),
            requests: register_int_counter_vec_with_registry!(
                opts!(
                    "lfsx_requests_total",
                    "Requests served, by route and status"
                ),
                &["route", "status"],
                registry
            )
            .expect("metric"),
            duration: register_histogram_vec_with_registry!(
                histogram_opts!(
                    "lfsx_request_duration_seconds",
                    "Time to serve a request",
                    DURATION_BUCKETS.to_vec()
                ),
                &["route"],
                registry
            )
            .expect("metric"),
            rejections: register_int_counter_vec_with_registry!(
                opts!("lfsx_rejections_total", "Requests refused, by cause"),
                &["cause"],
                registry
            )
            .expect("metric"),
            uploaded_bytes: register_int_counter_with_registry!(
                opts!("lfsx_uploaded_bytes_total", "Object bytes accepted"),
                registry
            )
            .expect("metric"),
            downloaded_bytes: register_int_counter_with_registry!(
                opts!("lfsx_downloaded_bytes_total", "Object bytes served"),
                registry
            )
            .expect("metric"),
            object_size: register_histogram_with_registry!(
                histogram_opts!(
                    "lfsx_object_size_bytes",
                    "Size of objects accepted",
                    SIZE_BUCKETS.to_vec()
                ),
                registry
            )
            .expect("metric"),
            objects_stored: register_int_gauge_with_registry!(
                opts!("lfsx_objects_stored", "Objects on disk at the last scrape"),
                registry
            )
            .expect("metric"),
            store_bytes: register_int_gauge_with_registry!(
                opts!("lfsx_store_bytes", "Bytes on disk at the last scrape"),
                registry
            )
            .expect("metric"),
        }
    }

    pub fn render(&self) -> String {
        let mut buffer = Vec::new();
        let encoder = TextEncoder::new();

        match encoder.encode(&self.registry.gather(), &mut buffer) {
            Ok(()) => String::from_utf8(buffer).unwrap_or_default(),
            Err(error) => {
                tracing::error!(%error, "could not encode metrics");
                String::new()
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Cause(pub &'static str);

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

pub async fn record(State(state): State<Shared>, request: Request, next: Next) -> Response {
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| "<unmatched>".to_owned());

    let started = Instant::now();
    let response = next.run(request).await;

    let metrics = &state.metrics;
    metrics
        .duration
        .with_label_values(&[route.as_str()])
        .observe(started.elapsed().as_secs_f64());
    metrics
        .requests
        .with_label_values(&[route.as_str(), response.status().as_str()])
        .inc();

    if let Some(Cause(cause)) = response.extensions().get::<Cause>() {
        metrics.rejections.with_label_values(&[cause]).inc();
    }

    response
}
