use std::time::Duration;

use axum::body::Bytes;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{reads, upload_client};

const READ_TIMEOUT: Duration = Duration::from_millis(200);
const CHUNKS: usize = 6;
const CHUNK: usize = 10;

async fn silent_store() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _held = listener.accept().await;
        std::future::pending::<()>().await;
    });
    url
}

async fn draining_store() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut received = Vec::new();
        let mut buffer = [0u8; 1024];
        let head_end = loop {
            let read = socket.read(&mut buffer).await.unwrap();
            received.extend_from_slice(&buffer[..read]);
            if let Some(at) = received.windows(4).position(|w| w == b"\r\n\r\n") {
                break at + 4;
            }
        };
        while received.len() < head_end + CHUNKS * CHUNK {
            let read = socket.read(&mut buffer).await.unwrap();
            received.extend_from_slice(&buffer[..read]);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    url
}

fn slow_body() -> reqwest::Body {
    let stream = futures_util::stream::unfold(0, |sent| async move {
        if sent == CHUNKS {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        Some((
            Ok::<_, std::io::Error>(Bytes::from(vec![0u8; CHUNK])),
            sent + 1,
        ))
    });
    reqwest::Body::wrap_stream(stream)
}

#[tokio::test]
async fn a_store_that_accepts_and_never_answers_times_out() {
    let url = silent_store().await;

    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        reads(READ_TIMEOUT).unwrap().get(url).send(),
    )
    .await
    .expect("the client gave up on its own");

    assert!(outcome.unwrap_err().is_timeout());
}

#[tokio::test]
async fn a_slow_but_steady_upload_outlives_the_read_timeout_on_the_upload_client() {
    let url = draining_store().await;

    let response = upload_client()
        .unwrap()
        .put(url)
        .header(reqwest::header::CONTENT_LENGTH, CHUNKS * CHUNK)
        .body(slow_body())
        .send()
        .await
        .unwrap();

    assert!(response.status().is_success());
}

#[tokio::test]
async fn the_read_client_cuts_a_streamed_upload_that_outlives_its_timeout() {
    let url = draining_store().await;

    let outcome = reads(READ_TIMEOUT)
        .unwrap()
        .put(url)
        .header(reqwest::header::CONTENT_LENGTH, CHUNKS * CHUNK)
        .body(slow_body())
        .send()
        .await;

    assert!(outcome.unwrap_err().is_timeout());
}
