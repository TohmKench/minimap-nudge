//! Watcher + real HTTP client against an in-process fake League client
//! (HTTPS with a self-signed cert, like the real one). The test controls the
//! game clock step by step, so these are deterministic and fast.

use minimap_nudge_lib::league::LeagueClient;
use minimap_nudge_lib::schedule::Phase;
use minimap_nudge_lib::settings::Settings;
use minimap_nudge_lib::watcher::{poll_once, Event, State, Watcher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Clone)]
struct Game {
    up: bool,
    time: f64,
    mode: &'static str,
    map: i64,
    ended: bool,
}

impl Default for Game {
    fn default() -> Self {
        Game { up: true, time: 0.0, mode: "CLASSIC", map: 11, ended: false }
    }
}

async fn start_mock(game: Arc<Mutex<Game>>) -> String {
    let cert = include_bytes!("../../test/fixtures/mock-cert.pem");
    let key = include_bytes!("../../test/fixtures/mock-key.pem");
    let identity = native_tls::Identity::from_pkcs8(cert, key).expect("identity");
    let acceptor = tokio_native_tls::TlsAcceptor::from(native_tls::TlsAcceptor::new(identity).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { continue };
            let acceptor = acceptor.clone();
            let game = game.clone();
            tokio::spawn(async move {
                if !game.lock().unwrap().up {
                    return; // "game closed": drop the connection
                }
                let Ok(mut tls) = acceptor.accept(tcp).await else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&buf);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let g = game.lock().unwrap().clone();
                let (status, body) = match path.as_str() {
                    "/liveclientdata/gamestats" => (
                        "200 OK",
                        format!(r#"{{"gameMode":"{}","gameTime":{},"mapName":"Map11","mapNumber":{},"mapTerrain":"Default"}}"#, g.mode, g.time, g.map),
                    ),
                    "/liveclientdata/eventdata" => {
                        let end = if g.ended { r#",{"EventID":9,"EventName":"GameEnd","EventTime":1,"Result":"Win"}"# } else { "" };
                        ("200 OK", format!(r#"{{"Events":[{{"EventID":0,"EventName":"GameStart","EventTime":0.05}}{end}]}}"#))
                    }
                    _ => ("404 Not Found", "{}".to_string()),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = tls.write_all(resp.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    format!("https://127.0.0.1:{port}")
}

fn schedule() -> Settings {
    Settings {
        phases: vec![
            Phase { start_min: 0.0, interval_sec: Some(180.0) },
            Phase { start_min: 10.0, interval_sec: Some(60.0) },
        ],
        ..Settings::default()
    }
}

fn watcher() -> Mutex<Watcher> {
    let mut w = Watcher::new();
    w.end_check_interval = Duration::ZERO; // check GameEnd every poll in tests
    Mutex::new(w)
}

/// Steps the fake clock from `from` to `to` in 1 s increments, polling each step.
async fn run(base: &str, game: &Arc<Mutex<Game>>, w: &Mutex<Watcher>, s: &Settings, from: f64, to: f64) -> (Vec<f64>, Vec<Event>) {
    let client = LeagueClient::new(base);
    let mut pings = vec![];
    let mut all = vec![];
    let mut t = from;
    while t <= to {
        game.lock().unwrap().time = t;
        let (events, _) = poll_once(&client, w, s).await;
        for e in events {
            if let Event::Ping { due, .. } = e {
                pings.push(due);
            }
            all.push(e);
        }
        t += 1.0;
    }
    (pings, all)
}

#[tokio::test]
async fn no_game_running_means_waiting() {
    // Nothing listening on this port.
    let client = LeagueClient::new("https://127.0.0.1:9");
    let w = watcher();
    let (events, status) = poll_once(&client, &w, &schedule()).await;
    assert!(events.is_empty());
    assert_eq!(status.state, State::Waiting);
}

#[tokio::test]
async fn pings_follow_game_clock_through_phase_change() {
    let game = Arc::new(Mutex::new(Game::default()));
    let base = start_mock(game.clone()).await;
    let w = watcher();
    let (pings, events) = run(&base, &game, &w, &schedule(), 170.0, 785.0).await;
    assert!(matches!(events[0], Event::GameDetected { .. }));
    assert_eq!(pings, vec![180.0, 360.0, 540.0, 600.0, 660.0, 720.0, 780.0]);
    assert_eq!(w.lock().unwrap().status(&schedule()).next_alert, Some(840.0));
}

#[tokio::test]
async fn joining_mid_game_has_no_backlog_and_closing_resets() {
    let game = Arc::new(Mutex::new(Game { time: 700.0, ..Game::default() }));
    let base = start_mock(game.clone()).await;
    let w = watcher();
    let s = schedule();
    let (pings, _) = run(&base, &game, &w, &s, 700.0, 700.0).await;
    assert!(pings.is_empty());
    assert_eq!(w.lock().unwrap().status(&s).next_alert, Some(720.0));

    game.lock().unwrap().up = false;
    let client = LeagueClient::new(&base);
    let (events, status) = poll_once(&client, &w, &s).await;
    assert_eq!(events, vec![Event::GameClosed]);
    assert_eq!(status.state, State::Waiting);
}

#[tokio::test]
async fn aram_is_quiet_only_when_sr_only() {
    let game = Arc::new(Mutex::new(Game { mode: "ARAM", map: 12, ..Game::default() }));
    let base = start_mock(game.clone()).await;

    let quiet = schedule();
    let w = watcher();
    let (pings, _) = run(&base, &game, &w, &quiet, 175.0, 185.0).await;
    assert!(pings.is_empty());
    assert_eq!(w.lock().unwrap().status(&quiet).state, State::Ineligible);

    let loud = Settings { summoners_rift_only: false, ..schedule() };
    let w = watcher();
    let (pings, _) = run(&base, &game, &w, &loud, 175.0, 185.0).await;
    assert_eq!(pings, vec![180.0]);
}

#[tokio::test]
async fn game_end_and_disabled_stop_pings() {
    let game = Arc::new(Mutex::new(Game::default()));
    let base = start_mock(game.clone()).await;
    let s = schedule();

    let w = watcher();
    run(&base, &game, &w, &s, 170.0, 172.0).await;
    game.lock().unwrap().ended = true;
    let (pings, events) = run(&base, &game, &w, &s, 173.0, 200.0).await;
    assert!(pings.is_empty());
    assert!(events.contains(&Event::GameEnded));
    assert_eq!(w.lock().unwrap().status(&s).state, State::Ended);

    game.lock().unwrap().ended = false;
    let off = Settings { enabled: false, ..schedule() };
    let w = watcher();
    let (pings, _) = run(&base, &game, &w, &off, 170.0, 400.0).await;
    assert!(pings.is_empty());
    assert_eq!(w.lock().unwrap().status(&off).state, State::Paused);
}
