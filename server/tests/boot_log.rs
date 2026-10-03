mod common;

use common::{config, forge};

#[derive(Clone, Default)]
struct Captured(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

#[tokio::test]
async fn a_boot_says_what_access_is_configured_once() {
    let root = tempfile::tempdir().unwrap();
    let (api_url, _forge) = forge().await;
    let config = config(&root, &api_url);

    let mut logged = String::new();
    for _ in 0..5 {
        let captured = Captured::default();
        {
            let _guard = tracing::subscriber::set_default(
                tracing_subscriber::fmt()
                    .with_writer(captured.clone())
                    .with_max_level(tracing::Level::INFO)
                    .with_ansi(false)
                    .finish(),
            );

            lfsx_server::reclaim(&config).await;
            lfsx_server::store(&config);
            let _app = lfsx_server::app(config.clone());
        }

        logged = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        if !logged.is_empty() {
            break;
        }
    }

    assert_eq!(
        logged.matches("LFSX_ALLOWED is unset").count(),
        1,
        "{logged}"
    );
}
