use std::io;
use std::time::Duration;

use axum::body::{Body, Bytes};
use futures_util::{Stream, StreamExt};
use tokio::sync::OwnedSemaphorePermit;

pub const IDLE: Duration = Duration::from_secs(60);

fn stalled() -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, "the transfer made no progress")
}

pub fn upload_chunks<S, E>(
    chunks: S,
    idle: Duration,
) -> impl Stream<Item = Result<Bytes, io::Error>> + Unpin
where
    S: Stream<Item = Result<Bytes, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    Box::pin(futures_util::stream::unfold(
        Some(chunks),
        move |chunks| async move {
            let mut chunks = chunks?;
            match tokio::time::timeout(idle, chunks.next()).await {
                Ok(Some(Ok(chunk))) => Some((Ok(chunk), Some(chunks))),
                Ok(Some(Err(error))) => Some((Err(io::Error::other(error)), None)),
                Ok(None) => None,
                Err(_) => Some((Err(stalled()), None)),
            }
        },
    ))
}

pub fn download_body<S, E>(
    mut chunks: S,
    permit: Option<OwnedSemaphorePermit>,
    idle: Duration,
) -> Body
where
    S: Stream<Item = Result<Bytes, E>> + Send + Unpin + 'static,
    E: Into<axum::BoxError> + Send + 'static,
{
    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, axum::BoxError>>(1);

    tokio::spawn(async move {
        let _permit = permit;
        while let Ok(Some(chunk)) = tokio::time::timeout(idle, chunks.next()).await {
            let failed = chunk.is_err();
            let sent = tokio::time::timeout(idle, sender.send(chunk.map_err(Into::into))).await;
            if failed || !matches!(sent, Ok(Ok(()))) {
                break;
            }
        }
    });

    Body::from_stream(futures_util::stream::unfold(
        receiver,
        |mut receiver| async { receiver.recv().await.map(|chunk| (chunk, receiver)) },
    ))
}
