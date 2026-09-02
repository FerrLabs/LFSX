// The audit trail is a log stream, not a store: one target, one event per
// privileged mutation, each naming who acted. The dedicated target is the
// point: `RUST_LOG=lfsx::audit=info` routes it to a file or a collector
// without turning anything else up, and it costs nothing unrouted. The
// server has no database on purpose, so durability belongs to whatever the
// operator ships logs to.
macro_rules! audit_log {
    ($($arg:tt)*) => {
        tracing::info!(target: "lfsx::audit", $($arg)*)
    };
}

pub(crate) use audit_log;
