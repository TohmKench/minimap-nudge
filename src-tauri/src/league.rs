//! Riot's Live Client Data API — served by the League game client on
//! https://127.0.0.1:2999 while a match is running. Official, local, read-only
//! (no memory reading or injection), so it's fine with Vanguard.
//!
//! The certificate is self-signed by Riot, so certificate checks are disabled
//! on this client, which only ever talks to the loopback address.

use serde::Deserialize;
use std::time::Duration;

pub const DEFAULT_BASE: &str = "https://127.0.0.1:2999";

#[derive(Debug, Clone, PartialEq)]
pub struct GameStats {
    pub game_time: f64,
    pub game_mode: String,
    /// 11 = Summoner's Rift, 12 = Howling Abyss (ARAM), 30 = Arena.
    pub map_number: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStats {
    game_time: Option<f64>,
    game_mode: Option<String>,
    map_number: Option<i64>,
}

#[derive(Deserialize)]
struct RawEvents {
    #[serde(rename = "Events", default)]
    events: Vec<RawEvent>,
}

#[derive(Deserialize)]
struct RawEvent {
    #[serde(rename = "EventName", default)]
    name: String,
}

#[derive(Clone)]
pub struct LeagueClient {
    http: reqwest::Client,
    base: String,
}

impl LeagueClient {
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true)
            .timeout(Duration::from_millis(1500))
            .connect_timeout(Duration::from_millis(800))
            .no_proxy() // never route the loopback API through a system proxy
            .pool_max_idle_per_host(2)
            .build()
            .expect("http client");
        Self { http, base: base.into() }
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Option<T> {
        let res = self.http.get(format!("{}{}", self.base, path)).send().await.ok()?;
        if !res.status().is_success() {
            return None;
        }
        res.json::<T>().await.ok()
    }

    /// `None` when no match is running.
    pub async fn game_stats(&self) -> Option<GameStats> {
        let raw: RawStats = self.get("/liveclientdata/gamestats").await?;
        Some(GameStats {
            game_time: raw.game_time.filter(|t| t.is_finite())?,
            game_mode: raw.game_mode.unwrap_or_else(|| "UNKNOWN".into()),
            map_number: raw.map_number,
        })
    }

    /// True once the nexus has fallen (the client keeps serving data on the end screen).
    pub async fn game_ended(&self) -> bool {
        match self.get::<RawEvents>("/liveclientdata/eventdata").await {
            Some(ev) => ev.events.iter().any(|e| e.name == "GameEnd"),
            None => false,
        }
    }
}

impl Default for LeagueClient {
    fn default() -> Self {
        Self::new(DEFAULT_BASE)
    }
}
