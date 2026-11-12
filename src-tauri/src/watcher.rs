//! Tracks the in-game clock and decides when to ping. Port of `watcher.js`.
//!
//! The state machine (`Watcher::apply`) is pure so it can be unit-tested; the
//! network part (`poll_once`) fetches from the game and feeds it in.

use crate::lcu::{ClientInfo, Lcu, Phase as ClientPhase};
use crate::league::{GameStats, LeagueClient};
use crate::schedule::{format_clock, next_alert_after};
use crate::settings::Settings;
use serde::Serialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const POLL_INTERVAL: Duration = Duration::from_secs(1);
pub const END_CHECK_INTERVAL: Duration = Duration::from_secs(5);
/// If we wake up later than this past a ping (sleep, hitch), skip it silently.
pub const LATE_TOLERANCE_SEC: f64 = 10.0;
pub const SUMMONERS_RIFT: i64 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Waiting,
    /// League client open, not in a game (lobby, queue, champ select…). Details in `client`.
    Client,
    Active,
    Paused,
    Ineligible,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: State,
    pub in_game: bool,
    pub game_time: Option<f64>,
    pub game_mode: Option<String>,
    pub map_number: Option<i64>,
    pub next_alert: Option<f64>,
    pub line: String,
    pub client: Option<ClientInfo>,
    /// Whether pings will play for the upcoming/current game (false = mode isn't Summoner's Rift and SR-only is on).
    pub will_ping: bool,
}

/// What happened during one poll, for logging.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    GameDetected { game_time: f64, mode: String },
    Ping { game_time: f64, due: f64 },
    SkippedLate { due: f64, game_time: f64 },
    GameEnded,
    GameClosed,
    ClientPhase { phase: Option<ClientPhase>, queue: Option<String> },
}

#[derive(Debug)]
pub struct Watcher {
    in_game: bool,
    ended: bool,
    game_time: Option<f64>,
    game_mode: Option<String>,
    map_number: Option<i64>,
    next_alert: Option<f64>,
    last_end_check: Option<Instant>,
    pub end_check_interval: Duration,
    client: Option<ClientInfo>,
}

impl Default for Watcher {
    fn default() -> Self {
        Self {
            in_game: false,
            ended: false,
            game_time: None,
            game_mode: None,
            map_number: None,
            next_alert: None,
            last_end_check: None,
            end_check_interval: END_CHECK_INTERVAL,
            client: None,
        }
    }
}

impl Watcher {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset(&mut self) {
        *self = Self { end_check_interval: self.end_check_interval, client: self.client.take(), ..Self::default() };
    }

    /// Latest League client state (`None` = client not running). Returns an event when the phase changes.
    pub fn set_client(&mut self, info: Option<ClientInfo>) -> Option<Event> {
        let old = self.client.as_ref().map(|c| c.phase.clone());
        let new = info.as_ref().map(|c| c.phase.clone());
        let queue = info.as_ref().and_then(|c| c.queue.clone());
        self.client = info;
        (old != new).then_some(Event::ClientPhase { phase: new, queue })
    }

    pub fn in_game(&self) -> bool {
        self.in_game
    }

    fn will_ping(&self, s: &Settings) -> bool {
        if !s.summoners_rift_only {
            return true;
        }
        let map = self.map_number.or(self.client.as_ref().and_then(|c| c.map_id));
        map.map_or(true, |m| m == SUMMONERS_RIFT)
    }

    fn eligible(&self, s: &Settings) -> bool {
        !s.summoners_rift_only || self.map_number.map_or(true, |m| m == SUMMONERS_RIFT)
    }

    /// Whether the caller should ask the game for GameEnd on this poll.
    pub fn end_check_due(&self, now: Instant) -> bool {
        self.in_game
            && !self.ended
            && self.last_end_check.map_or(true, |t| now.duration_since(t) >= self.end_check_interval)
    }

    /// Recompute the next ping after a schedule change.
    pub fn recompute(&mut self, s: &Settings) {
        if let (true, Some(t)) = (self.in_game, self.game_time) {
            self.next_alert = next_alert_after(&s.phases, t);
        }
    }

    /// Feed one poll result in. `ended` is `Some` only when an end check ran.
    pub fn apply(&mut self, stats: Option<GameStats>, ended: Option<bool>, s: &Settings, now: Instant) -> Vec<Event> {
        let mut events = Vec::new();
        let Some(stats) = stats else {
            if self.in_game {
                self.reset();
                events.push(Event::GameClosed);
            }
            return events;
        };

        let new_game = !self.in_game || self.game_time.is_some_and(|prev| stats.game_time + 5.0 < prev);
        if new_game {
            self.reset();
            self.in_game = true;
            self.next_alert = next_alert_after(&s.phases, stats.game_time);
            events.push(Event::GameDetected { game_time: stats.game_time, mode: stats.game_mode.clone() });
        }

        self.game_time = Some(stats.game_time);
        self.game_mode = Some(stats.game_mode);
        self.map_number = stats.map_number;

        if let Some(e) = ended {
            self.last_end_check = Some(now);
            if e && !self.ended {
                self.ended = true;
                events.push(Event::GameEnded);
            }
        }

        if let Some(due) = self.next_alert {
            let t = stats.game_time;
            if t >= due {
                let late = t - due;
                if s.enabled && self.eligible(s) && !self.ended {
                    if late <= LATE_TOLERANCE_SEC {
                        events.push(Event::Ping { game_time: t, due });
                    } else {
                        events.push(Event::SkippedLate { due, game_time: t });
                    }
                }
                self.next_alert = next_alert_after(&s.phases, t);
            }
        }
        events
    }

    pub fn status(&self, s: &Settings) -> Status {
        let state = if self.in_game {
            if self.ended {
                State::Ended
            } else if !self.eligible(s) {
                State::Ineligible
            } else if !s.enabled {
                State::Paused
            } else {
                State::Active
            }
        } else if !s.enabled {
            State::Paused
        } else if self.client.is_some() {
            State::Client
        } else {
            State::Waiting
        };
        let line = match state {
            State::Active => format!(
                "In game {} · next ping {}",
                format_clock(self.game_time),
                format_clock(self.next_alert)
            ),
            State::Paused if self.in_game => format!("In game {} · alerts paused", format_clock(self.game_time)),
            State::Paused => match &self.client {
                Some(c) => format!("Alerts paused · {}", client_line(c)),
                None => "Alerts paused".into(),
            },
            State::Client => client_line(self.client.as_ref().expect("client state")),
            State::Ineligible => format!(
                "In game ({}) · not Summoner's Rift, staying quiet",
                self.game_mode.as_deref().unwrap_or("?")
            ),
            State::Ended => "Game over · waiting for next match".into(),
            State::Waiting => "Waiting for League".into(),
        };
        Status {
            state,
            in_game: self.in_game,
            game_time: self.game_time,
            game_mode: self.game_mode.clone(),
            map_number: self.map_number,
            next_alert: self.next_alert,
            line,
            client: if self.in_game { None } else { self.client.clone() },
            will_ping: self.will_ping(s),
        }
    }
}

/// One-line description of what the client is doing (tray tooltip, log).
pub fn client_line(c: &ClientInfo) -> String {
    let q = c.queue.as_deref();
    let with_q = |base: &str| match q {
        Some(q) => format!("{base} · {q}"),
        None => base.to_string(),
    };
    match c.phase {
        ClientPhase::None | ClientPhase::Other => "League client open".into(),
        ClientPhase::Lobby => {
            let mut l = with_q("In lobby");
            if let Some(n) = c.party_size.filter(|n| *n > 1) {
                l.push_str(&format!(" · party of {n}"));
            }
            l
        }
        ClientPhase::Matchmaking => {
            let mut l = with_q("In queue");
            if let Some(t) = c.time_in_queue {
                l.push_str(&format!(" · {}", format_clock(Some(t))));
            }
            l
        }
        ClientPhase::ReadyCheck => "Match found — accept it!".into(),
        ClientPhase::ChampSelect => with_q("Champion select"),
        ClientPhase::GameStart | ClientPhase::InProgress => "Loading into game…".into(),
        ClientPhase::Reconnect => "Reconnect to your game".into(),
        ClientPhase::PostGame => "Post-game".into(),
    }
}

/// One poll: ask the game (and, when not in one, the client), update the watcher.
/// Never holds the lock across the network.
pub async fn poll_once(
    client: &LeagueClient,
    lcu: &Lcu,
    watcher: &Mutex<Watcher>,
    settings: &Settings,
) -> (Vec<Event>, Status) {
    let stats = client.game_stats().await;
    let now = Instant::now();
    let check_end = stats.is_some() && watcher.lock().unwrap().end_check_due(now);
    let ended = if check_end { Some(client.game_ended().await) } else { None };
    // The client's state only matters outside a game; skip those calls while playing.
    let client_info = if stats.is_none() { Some(lcu.snapshot().await) } else { None };
    let mut w = watcher.lock().unwrap();
    let mut events = w.apply(stats, ended, settings, now);
    if let Some(info) = client_info {
        events.extend(w.set_client(info));
    }
    (events, w.status(settings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schedule::Phase;

    fn settings() -> Settings {
        Settings {
            phases: vec![
                Phase { start_min: 0.0, interval_sec: Some(180.0) },
                Phase { start_min: 10.0, interval_sec: Some(60.0) },
            ],
            ..Settings::default()
        }
    }
    fn sr(t: f64) -> Option<GameStats> {
        Some(GameStats { game_time: t, game_mode: "CLASSIC".into(), map_number: Some(11) })
    }
    fn pings(ev: &[Event]) -> usize {
        ev.iter().filter(|e| matches!(e, Event::Ping { .. })).count()
    }

    #[test]
    fn steps_through_a_game() {
        let s = settings();
        let mut w = Watcher::new();
        let now = Instant::now();
        assert_eq!(w.status(&s).state, State::Waiting);
        let ev = w.apply(sr(170.0), Some(false), &s, now);
        assert!(matches!(ev[0], Event::GameDetected { .. }));
        assert_eq!(w.status(&s).next_alert, Some(180.0));
        assert_eq!(pings(&w.apply(sr(179.0), None, &s, now)), 0);
        assert_eq!(pings(&w.apply(sr(180.4), None, &s, now)), 1);
        assert_eq!(w.status(&s).next_alert, Some(360.0));
        assert_eq!(w.apply(None, None, &s, now), vec![Event::GameClosed]);
        assert_eq!(w.status(&s).state, State::Waiting);
    }

    #[test]
    fn late_ping_skipped_new_game_on_clock_jump() {
        let s = settings();
        let mut w = Watcher::new();
        let now = Instant::now();
        w.apply(sr(100.0), None, &s, now);
        let ev = w.apply(sr(195.0), None, &s, now); // 15 s late
        assert!(matches!(ev[0], Event::SkippedLate { .. }));
        let ev = w.apply(sr(20.0), None, &s, now); // clock went backwards -> new game
        assert!(matches!(ev[0], Event::GameDetected { .. }));
        assert_eq!(w.status(&s).next_alert, Some(180.0));
    }

    #[test]
    fn ended_beats_paused_and_end_check_cadence() {
        let mut s = settings();
        s.enabled = false;
        let mut w = Watcher::new();
        let t0 = Instant::now();
        assert!(!w.end_check_due(t0), "no checks outside a game");
        w.apply(sr(100.0), None, &s, t0);
        assert!(w.end_check_due(t0));
        w.apply(sr(101.0), Some(false), &s, t0);
        assert!(!w.end_check_due(t0 + Duration::from_secs(1)));
        assert!(w.end_check_due(t0 + Duration::from_secs(5)));
        assert_eq!(w.status(&s).state, State::Paused);
        w.apply(sr(102.0), Some(true), &s, t0);
        assert_eq!(w.status(&s).state, State::Ended);
    }

    fn client(phase: ClientPhase, map: Option<i64>) -> Option<ClientInfo> {
        Some(ClientInfo {
            phase,
            queue_id: Some(420),
            queue: Some("Ranked Solo/Duo".into()),
            map_id: map,
            party_size: Some(2),
            time_in_queue: Some(83.0),
            est_queue_time: Some(120.0),
            champ_select_stage: None,
            phase_time_left: None,
        })
    }

    #[test]
    fn client_states_and_lines() {
        let mut s = settings();
        let mut w = Watcher::new();
        assert_eq!(w.status(&s).line, "Waiting for League");
        assert!(matches!(w.set_client(client(ClientPhase::Lobby, Some(11))), Some(Event::ClientPhase { .. })));
        assert_eq!(w.set_client(client(ClientPhase::Lobby, Some(11))), None, "same phase -> no event");
        let st = w.status(&s);
        assert_eq!(st.state, State::Client);
        assert_eq!(st.line, "In lobby · Ranked Solo/Duo · party of 2");
        assert!(st.will_ping);

        w.set_client(client(ClientPhase::Matchmaking, Some(11)));
        assert_eq!(w.status(&s).line, "In queue · Ranked Solo/Duo · 1:23");
        w.set_client(client(ClientPhase::ReadyCheck, Some(11)));
        assert_eq!(w.status(&s).line, "Match found — accept it!");
        w.set_client(client(ClientPhase::ChampSelect, Some(12)));
        assert!(!w.status(&s).will_ping, "ARAM map with SR-only");
        s.summoners_rift_only = false;
        assert!(w.status(&s).will_ping);

        // once the game is live, in-game status wins and client details are hidden
        w.apply(sr(30.0), None, &s, Instant::now());
        let st = w.status(&s);
        assert_eq!(st.state, State::Active);
        assert!(st.client.is_none());

        // paused shows what the client is doing too
        let mut w = Watcher::new();
        w.set_client(client(ClientPhase::ChampSelect, Some(11)));
        s.enabled = false;
        assert_eq!(w.status(&s).line, "Alerts paused · Champion select · Ranked Solo/Duo");
        assert!(matches!(w.set_client(None), Some(Event::ClientPhase { phase: None, .. })));
        assert_eq!(w.status(&s).line, "Alerts paused");
    }

    #[test]
    fn recompute_uses_current_clock() {
        let mut s = settings();
        let mut w = Watcher::new();
        w.apply(sr(200.0), None, &s, Instant::now());
        assert_eq!(w.status(&s).next_alert, Some(360.0));
        s.phases = vec![Phase { start_min: 0.0, interval_sec: Some(30.0) }];
        w.recompute(&s);
        assert_eq!(w.status(&s).next_alert, Some(210.0));
    }
}
