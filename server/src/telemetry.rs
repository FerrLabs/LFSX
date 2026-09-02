use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

// Traces are opt-in: `LFSX_OTLP_ENDPOINT` names the collector's HTTP traces
// URL (`http://collector:4318/v1/traces`), and while it is unset the layer is
// not even installed, so the operator who does not care pays nothing. Metrics
// stay Prometheus either way; the trace answers the one question a counter
// cannot, which is where inside a slow request the time went.
//
// The returned provider is the flush handle: whoever installed it shuts it
// down on exit so the last batch of spans is not dropped on the floor.
pub fn init() -> Option<SdkTracerProvider> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    let plain = tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer());

    let Some(endpoint) = std::env::var("LFSX_OTLP_ENDPOINT")
        .ok()
        .filter(|endpoint| !endpoint.is_empty())
    else {
        plain.init();
        return None;
    };

    let exporter = match opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(&endpoint)
        .build()
    {
        Ok(exporter) => exporter,
        Err(error) => {
            plain.init();
            tracing::error!(
                %error,
                endpoint,
                "LFSX_OTLP_ENDPOINT is set but no exporter could be built for it, so traces are off"
            );
            return None;
        }
    };

    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name("lfsx")
                .build(),
        )
        .build();

    plain
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("lfsx")))
        .init();
    tracing::info!(endpoint, "traces are exported over OTLP");

    Some(provider)
}

// W3C trace context onto an outbound request, so a forge or a proxy that
// participates lands in the same trace as the request that asked. Without the
// propagator installed (traces off) this injects nothing and costs a lookup.
pub(crate) fn propagated(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    let context = tracing::Span::current().context();
    let mut carrier = std::collections::HashMap::new();
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&context, &mut carrier);
    });

    carrier.into_iter().fold(request, |request, (name, value)| {
        request.header(name, value)
    })
}
