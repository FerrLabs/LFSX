// rustls needs a process-wide crypto provider before the first TLS connection,
// and reqwest is built without one on purpose so the choice is this project's
// rather than whatever the default happens to be in a given release. ring is
// what 0.12 used; aws-lc, the new default, wants a C toolchain at build time on
// exactly the musl and cross-compiled aarch64 targets the releases ship.
//
// It goes where the clients are built rather than in `main`, because `app()` is
// a library entry point: anything that builds a client without going through a
// binary would otherwise panic on its first request.
pub fn install_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();

    ONCE.call_once(|| {
        // An error here means something already installed one, which is an
        // embedder's decision to make and not ours to override.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}
