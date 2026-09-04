use ferrum_core::acme::{Directory, Issuer, not_after_of, renew_due};
use ferrum_core::state::State;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const VALIDATION_PORT: u16 = 5002;
const HOST: &str = "localtest.me";

fn pebble() -> Directory {
    let ca = std::env::var("FERRUM_PEBBLE_CA").expect(
        "set FERRUM_PEBBLE_CA to the PEM that signed Pebble's API certificate before running this",
    );
    Directory::Custom {
        url: std::env::var("FERRUM_PEBBLE_DIR")
            .unwrap_or_else(|_| "https://localhost:14000/dir".to_string()),
        root_pem: Some(PathBuf::from(ca)),
    }
}

async fn serve_challenges(webroot: PathBuf) -> tokio::task::JoinHandle<()> {
    let listener = TcpListener::bind(("::", VALIDATION_PORT))
        .await
        .expect("bind the HTTP-01 validation port");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let webroot = webroot.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let Ok(n) = socket.read(&mut buf).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buf[..n]);
                let token = request
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .and_then(|p| p.rsplit('/').next())
                    .unwrap_or_default()
                    .to_string();

                let response = match std::fs::read_to_string(webroot.join(&token)) {
                    Ok(body) => format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\n\r\n{body}",
                        body.len()
                    ),
                    Err(_) => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string(),
                };
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    })
}

#[tokio::test]
#[ignore]
async fn issues_against_pebble() {
    let dir = tempfile::tempdir().unwrap();
    let webroot = dir.path().join("webroot");
    let certs = dir.path().join("certs");
    std::fs::create_dir_all(&webroot).unwrap();

    let server = serve_challenges(webroot.clone()).await;

    let state = State::open(&dir.path().join("state")).await.unwrap();
    let issuer = Issuer::new(&state, pebble(), "ferrum@example.com")
        .await
        .expect("register an account with pebble")
        .with_webroot(webroot.clone());

    let cert = issuer
        .issue(HOST, &certs)
        .await
        .expect("issue a certificate");

    server.abort();

    assert!(cert.fullchain.exists());
    assert!(cert.key.exists());
    assert!(!renew_due(cert.not_after, OffsetDateTime::now_utc()));

    let mode = |p: &Path| {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).unwrap().permissions().mode() & 0o777
    };
    assert_eq!(mode(&cert.key), 0o600);
    assert_eq!(mode(&cert.fullchain), 0o644);

    let chain = std::fs::read_to_string(&cert.fullchain).unwrap();
    assert_eq!(not_after_of(&chain).unwrap(), cert.not_after);

    assert_eq!(std::fs::read_dir(&webroot).unwrap().count(), 0);
}
