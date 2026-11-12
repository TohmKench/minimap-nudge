//! The real LCU client code against a fake League client: lockfile discovery,
//! basic auth, and every phase the app shows.
mod common;

use minimap_nudge_lib::lcu::{Lcu, Phase};
use std::sync::{Arc, Mutex};

const PASSWORD: &str = "s3cr3t-Token_x";

#[derive(Clone)]
struct Fake {
    phase: &'static str,
    queue_id: i64,
    map_id: i64,
    members: usize,
}

fn expected_auth() -> String {
    // base64("riot:s3cr3t-Token_x")
    format!("Basic {}", b64(&format!("riot:{PASSWORD}")))
}

fn b64(s: &str) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

async fn fake_client(state: Arc<Mutex<Fake>>) -> u16 {
    common::serve(Arc::new(move |path, auth| {
        if auth != Some(expected_auth().as_str()) {
            return (401, r#"{"message":"unauthorized"}"#.into());
        }
        let f = state.lock().unwrap().clone();
        match path {
            "/lol-gameflow/v1/gameflow-phase" => (200, format!("\"{}\"", f.phase)),
            "/lol-gameflow/v1/session" => (
                200,
                format!(
                    r#"{{"phase":"{}","gameData":{{"isCustomGame":false,"queue":{{"id":{},"mapId":{},"description":"5v5 Ranked Solo games"}}}},"map":{{"id":{}}}}}"#,
                    f.phase, f.queue_id, f.map_id, f.map_id
                ),
            ),
            "/lol-lobby/v2/lobby" => {
                let members = vec!["{}"; f.members].join(",");
                (200, format!(r#"{{"members":[{members}],"gameConfig":{{"queueId":{},"mapId":{}}}}}"#, f.queue_id, f.map_id))
            }
            "/lol-matchmaking/v1/search" => {
                (200, r#"{"searchState":"Searching","timeInQueue":83.4,"estimatedQueueTime":125.0}"#.into())
            }
            "/lol-champ-select/v1/session" => (
                200,
                r#"{"timer":{"adjustedTimeLeftInPhase":24500,"phase":"BAN_PICK","isInfinite":false},"localPlayerCellId":3,"myTeam":[{"cellId":1,"assignedPosition":"top"},{"cellId":3,"assignedPosition":"JUNGLE"},{"cellId":4,"assignedPosition":"utility"}]}"#.into(),
            ),
            _ => (404, "{}".into()),
        }
    }))
    .await
}

#[tokio::test]
async fn walks_through_client_phases() {
    let state = Arc::new(Mutex::new(Fake { phase: "None", queue_id: 420, map_id: 11, members: 2 }));
    let port = fake_client(state.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let lcu = Lcu::new(Some(dir.path().to_path_buf()));

    // No lockfile yet -> client not running
    assert_eq!(lcu.snapshot().await, None);

    std::fs::write(dir.path().join("lockfile"), format!("LeagueClient:4242:{port}:{PASSWORD}:https")).unwrap();
    let c = lcu.snapshot().await.expect("client found via lockfile");
    assert_eq!(c.phase, Phase::None);

    state.lock().unwrap().phase = "Lobby";
    let c = lcu.snapshot().await.unwrap();
    assert_eq!((c.phase, c.queue.as_deref(), c.party_size, c.map_id), (Phase::Lobby, Some("Ranked Solo/Duo"), Some(2), Some(11)));

    state.lock().unwrap().phase = "Matchmaking";
    let c = lcu.snapshot().await.unwrap();
    assert_eq!((c.phase, c.time_in_queue, c.est_queue_time), (Phase::Matchmaking, Some(83.4), Some(125.0)));

    state.lock().unwrap().phase = "ReadyCheck";
    assert_eq!(lcu.snapshot().await.unwrap().phase, Phase::ReadyCheck);

    state.lock().unwrap().phase = "ChampSelect";
    let c = lcu.snapshot().await.unwrap();
    assert_eq!((c.phase, c.champ_select_stage.as_deref(), c.phase_time_left), (Phase::ChampSelect, Some("Picks & bans"), Some(24.5)));
    assert_eq!(c.assigned_position.as_deref(), Some("jungle"), "my cell's position, lower-cased");

    {
        let mut s = state.lock().unwrap();
        s.phase = "ChampSelect";
        s.queue_id = 450;
        s.map_id = 12;
    }
    let c = lcu.snapshot().await.unwrap();
    assert_eq!((c.queue.as_deref(), c.map_id), (Some("ARAM"), Some(12)));

    for (raw, want) in [("GameStart", Phase::GameStart), ("InProgress", Phase::InProgress), ("EndOfGame", Phase::PostGame)] {
        state.lock().unwrap().phase = raw;
        assert_eq!(lcu.snapshot().await.unwrap().phase, want);
    }

    // Client closes: lockfile removed
    std::fs::remove_file(dir.path().join("lockfile")).unwrap();
    assert_eq!(lcu.snapshot().await, None);
}

#[tokio::test]
async fn wrong_password_means_unknown_not_crash() {
    let state = Arc::new(Mutex::new(Fake { phase: "Lobby", queue_id: 420, map_id: 11, members: 1 }));
    let port = fake_client(state).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lockfile"), format!("LeagueClient:1:{port}:wrong:https")).unwrap();
    assert_eq!(Lcu::new(Some(dir.path().to_path_buf())).snapshot().await, None);
}
