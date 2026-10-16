use std::time::Duration;

use super::build;

#[tokio::test]
async fn a_store_that_accepts_and_never_answers_times_out() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _held = listener.accept().await;
        std::future::pending::<()>().await;
    });

    let client = build(Duration::from_secs(5), Duration::from_millis(200)).unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), client.get(url).send())
        .await
        .expect("the client gave up on its own");

    assert!(outcome.unwrap_err().is_timeout());
}
