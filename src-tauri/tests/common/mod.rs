//! Tiny HTTPS server for tests (self-signed cert, like Riot's local APIs).
#![allow(dead_code)]
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub type Handler = Arc<dyn Fn(&str, Option<&str>) -> (u16, String) + Send + Sync>;

/// Serves `handler(path, authorization_header)` on 127.0.0.1; returns the port.
pub async fn serve(handler: Handler) -> u16 {
    let cert = include_bytes!("../../../test/fixtures/mock-cert.pem");
    let key = include_bytes!("../../../test/fixtures/mock-key.pem");
    let identity = native_tls::Identity::from_pkcs8(cert, key).expect("identity");
    let acceptor = tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            let (acceptor, handler) = (acceptor.clone(), handler.clone());
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let auth = req
                    .lines()
                    .find_map(|l| l.strip_prefix("authorization: ").or_else(|| l.strip_prefix("Authorization: ")))
                    .map(str::trim)
                    .map(str::to_string);
                let (code, body) = handler(&path, auth.as_deref());
                let reason = match code { 200 => "OK", 204 => "No Content", _ => "Error" };
                let resp = format!(
                    "HTTP/1.1 {code} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(resp.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    port
}
