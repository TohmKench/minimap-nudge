//! Auto-accept for the "match found" ready check.
//!
//! Pure decision logic (unit-tested); lib.rs does the LCU calls. Rules:
//! - off unless the user turned it on;
//! - waits `delay` seconds after the pop, so there's time to decline by hand;
//! - accepts at most once per ready check, never overrides a manual
//!   accept/decline, and gives up after a few failed attempts.

use crate::lcu::ReadyCheck;
use serde::Serialize;
use std::time::Instant;

const MAX_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyCheckStatus {
    /// Auto-accept is on.
    pub auto: bool,
    /// Seconds until we accept (while waiting).
    pub accept_in: Option<f64>,
    /// "none" | "accepted" | "declined"
    pub response: String,
    /// We were the ones who accepted it.
    pub auto_accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Nothing,
    Accept,
}

#[derive(Debug, Default)]
pub struct AutoAccept {
    seen_at: Option<Instant>,
    attempts: u8,
    accepted_by_us: bool,
}

impl AutoAccept {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call every poll. `rc` is `Some` only while the client is in the ready-check phase.
    pub fn step(&mut self, rc: Option<&ReadyCheck>, enabled: bool, delay: f64, now: Instant) -> (Action, Option<ReadyCheckStatus>) {
        let Some(rc) = rc else {
            *self = Self::default(); // ready check over: reset for the next pop
            return (Action::Nothing, None);
        };
        let seen = *self.seen_at.get_or_insert(now);
        let response = rc.player_response.to_ascii_lowercase();
        let response = if response.is_empty() { "none".to_string() } else { response };
        let mut status = ReadyCheckStatus {
            auto: enabled,
            accept_in: None,
            response: response.clone(),
            auto_accepted: self.accepted_by_us,
        };
        let waiting_on_us = response == "none" && rc.state == "InProgress";
        if !enabled || !waiting_on_us || self.accepted_by_us || self.attempts >= MAX_ATTEMPTS {
            return (Action::Nothing, Some(status));
        }
        let left = delay - now.duration_since(seen).as_secs_f64();
        if left > 0.0 {
            status.accept_in = Some(left);
            return (Action::Nothing, Some(status));
        }
        self.attempts += 1;
        (Action::Accept, Some(status))
    }

    /// Report the result of an `Action::Accept`.
    pub fn accepted(&mut self, ok: bool) {
        if ok {
            self.accepted_by_us = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn rc(state: &str, resp: &str) -> ReadyCheck {
        ReadyCheck { state: state.into(), player_response: resp.into(), timer: 0.0 }
    }

    #[test]
    fn waits_then_accepts_once() {
        let mut a = AutoAccept::new();
        let t0 = Instant::now();
        let pop = rc("InProgress", "None");
        let (act, st) = a.step(Some(&pop), true, 2.0, t0);
        assert_eq!(act, Action::Nothing);
        assert!((st.unwrap().accept_in.unwrap() - 2.0).abs() < 1e-6);
        assert_eq!(a.step(Some(&pop), true, 2.0, t0 + Duration::from_millis(1500)).0, Action::Nothing);
        assert_eq!(a.step(Some(&pop), true, 2.0, t0 + Duration::from_secs(2)).0, Action::Accept);
        a.accepted(true);
        // client may still say "None" for a moment: don't accept twice
        assert_eq!(a.step(Some(&pop), true, 2.0, t0 + Duration::from_secs(3)).0, Action::Nothing);
        let (_, st) = a.step(Some(&rc("InProgress", "Accepted")), true, 2.0, t0 + Duration::from_secs(4));
        let st = st.unwrap();
        assert!(st.auto_accepted && st.response == "accepted");
        // next pop starts fresh
        a.step(None, true, 2.0, t0 + Duration::from_secs(5));
        assert_eq!(a.step(Some(&pop), true, 0.0, t0 + Duration::from_secs(60)).0, Action::Accept);
    }

    #[test]
    fn respects_off_manual_decline_and_retries_limit() {
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(10);
        let mut a = AutoAccept::new();
        assert_eq!(a.step(Some(&rc("InProgress", "None")), false, 0.0, later).0, Action::Nothing, "off");

        let mut a = AutoAccept::new();
        assert_eq!(a.step(Some(&rc("InProgress", "Declined")), true, 0.0, later).0, Action::Nothing, "declined by hand");

        let mut a = AutoAccept::new();
        assert_eq!(a.step(Some(&rc("EveryoneReady", "None")), true, 0.0, later).0, Action::Nothing, "not waiting on us");

        // turning it on mid-pop counts from when the pop was first seen
        let mut a = AutoAccept::new();
        a.step(Some(&rc("InProgress", "None")), false, 2.0, t0);
        assert_eq!(a.step(Some(&rc("InProgress", "None")), true, 2.0, t0 + Duration::from_secs(3)).0, Action::Accept);

        // failed accepts: give up after 3 tries
        let mut a = AutoAccept::new();
        let mut accepts = 0;
        for i in 0..10 {
            if a.step(Some(&rc("InProgress", "None")), true, 0.0, t0 + Duration::from_secs(i)).0 == Action::Accept {
                accepts += 1;
                a.accepted(false);
            }
        }
        assert_eq!(accepts, 3);
    }
}
