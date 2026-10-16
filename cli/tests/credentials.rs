use std::io::Read;
use std::net::TcpListener;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

fn request_seen_by_doctor(env: &[(&str, &str)]) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let reader = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(_) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
                Err(error) => panic!("lfsx never connected: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        let mut buffer = [0u8; 4096];
        let read = stream.read(&mut buffer).unwrap();
        String::from_utf8_lossy(&buffer[..read]).to_lowercase()
    });

    let mut command = Command::new(env!("CARGO_BIN_EXE_lfsx"));
    command
        .env_remove("LFSX_TOKEN")
        .env_remove("GITHUB_TOKEN")
        .envs(env.iter().copied())
        .args(["--url", &url, "doctor"]);
    let mut child = command.spawn().unwrap();
    let request = reader.join().unwrap();
    child.kill().ok();
    child.wait().ok();
    request
}

#[test]
fn github_token_alone_is_never_sent() {
    let request = request_seen_by_doctor(&[("GITHUB_TOKEN", "ghs_ambient")]);
    assert!(request.starts_with("get "));
    assert!(!request.contains("authorization"));
}

#[test]
fn lfsx_token_is_sent_to_loopback() {
    let request = request_seen_by_doctor(&[("LFSX_TOKEN", "explicit")]);
    assert!(request.contains("authorization: basic"));
}
