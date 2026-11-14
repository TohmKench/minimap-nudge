//! Auto-hover your last ban in champ select. Pure decision logic; lib.rs does
//! the LCU calls.
//!
//! - Remembers the champion you last *completed* a ban on.
//! - When your ban turn starts, hovers it (never locks: you still click Ban).
//! - Hovers once per ban turn and never replaces something you've hovered,
//!   and skips a champion that's already been banned.

use serde::Deserialize;

const MAX_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CsAction {
    pub id: i64,
    pub actor_cell_id: i64,
    pub champion_id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_in_progress: bool,
    pub completed: bool,
}

/// The parts of `/lol-champ-select/v1/session` this needs.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CsSession {
    pub local_player_cell_id: Option<i64>,
    pub actions: Vec<Vec<CsAction>>,
}

impl CsSession {
    fn all(&self) -> impl Iterator<Item = &CsAction> {
        self.actions.iter().flatten()
    }

    fn my_bans(&self) -> impl Iterator<Item = &CsAction> {
        let me = self.local_player_cell_id;
        self.all().filter(move |a| a.kind == "ban" && Some(a.actor_cell_id) == me)
    }

    /// Champion you banned in this champ select, once the ban is locked in.
    pub fn my_completed_ban(&self) -> Option<i64> {
        self.my_bans().filter(|a| a.completed && a.champion_id > 0).map(|a| a.champion_id).last()
    }

    fn already_banned(&self, champ: i64) -> bool {
        self.all().any(|a| a.kind == "ban" && a.completed && a.champion_id == champ)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Decision {
    Nothing,
    Hover { action_id: i64, champion_id: i64 },
}

#[derive(Debug, Default)]
pub struct BanAssist {
    action: Option<i64>,
    attempts: u8,
    done: bool,
    /// Champion we hovered this champ select (for the status line).
    pub hovered: Option<i64>,
}

impl BanAssist {
    pub fn new() -> Self {
        Self::default()
    }

    /// Call when champ select ends.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn step(&mut self, s: &CsSession, enabled: bool, last_ban: Option<i64>) -> Decision {
        let (Some(champ), true) = (last_ban.filter(|c| *c > 0), enabled) else {
            return Decision::Nothing;
        };
        let Some(turn) = s.my_bans().find(|a| a.is_in_progress && !a.completed) else {
            return Decision::Nothing;
        };
        if self.action != Some(turn.id) {
            // a new ban turn (e.g. a second ban phase)
            self.action = Some(turn.id);
            self.attempts = 0;
            self.done = false;
        }
        if self.done || self.attempts >= MAX_ATTEMPTS || turn.champion_id != 0 || s.already_banned(champ) {
            return Decision::Nothing;
        }
        self.attempts += 1;
        Decision::Hover { action_id: turn.id, champion_id: champ }
    }

    /// Report the result of a `Decision::Hover`.
    pub fn hovered(&mut self, champion_id: i64, ok: bool) {
        if ok {
            self.done = true;
            self.hovered = Some(champion_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENGAR: i64 = 107;

    fn ban(id: i64, cell: i64, champ: i64, in_progress: bool, completed: bool) -> CsAction {
        CsAction { id, actor_cell_id: cell, champion_id: champ, kind: "ban".into(), is_in_progress: in_progress, completed }
    }
    fn session(actions: Vec<CsAction>) -> CsSession {
        CsSession { local_player_cell_id: Some(2), actions: vec![actions] }
    }

    #[test]
    fn hovers_once_when_my_ban_turn_starts() {
        let mut b = BanAssist::new();
        // not my turn yet
        assert_eq!(b.step(&session(vec![ban(10, 2, 0, false, false)]), true, Some(RENGAR)), Decision::Nothing);
        // my turn, nothing hovered
        let s = session(vec![ban(10, 2, 0, true, false), ban(11, 7, 0, true, false)]);
        assert_eq!(b.step(&s, true, Some(RENGAR)), Decision::Hover { action_id: 10, champion_id: RENGAR });
        b.hovered(RENGAR, true);
        assert_eq!(b.hovered, Some(RENGAR));
        // the client may still report 0 for a moment: still only once
        assert_eq!(b.step(&s, true, Some(RENGAR)), Decision::Nothing);
    }

    #[test]
    fn leaves_your_choices_alone() {
        let mut b = BanAssist::new();
        assert_eq!(b.step(&session(vec![ban(10, 2, 0, true, false)]), false, Some(RENGAR)), Decision::Nothing, "off");
        assert_eq!(b.step(&session(vec![ban(10, 2, 0, true, false)]), true, None), Decision::Nothing, "nothing remembered");
        assert_eq!(b.step(&session(vec![ban(10, 2, 64, true, false)]), true, Some(RENGAR)), Decision::Nothing, "you hovered Lee Sin");
        // a teammate already banned Rengar
        let s = session(vec![ban(9, 1, RENGAR, false, true), ban(10, 2, 0, true, false)]);
        assert_eq!(b.step(&s, true, Some(RENGAR)), Decision::Nothing);
    }

    #[test]
    fn gives_up_after_failed_attempts_and_handles_second_ban_phase() {
        let mut b = BanAssist::new();
        let s = session(vec![ban(10, 2, 0, true, false)]);
        let mut tries = 0;
        for _ in 0..6 {
            if let Decision::Hover { .. } = b.step(&s, true, Some(RENGAR)) {
                tries += 1;
                b.hovered(RENGAR, false);
            }
        }
        assert_eq!(tries, 3);
        // second ban phase = new action id: tries again
        let s2 = session(vec![ban(10, 2, 33, false, true), ban(20, 2, 0, true, false)]);
        assert_eq!(b.step(&s2, true, Some(RENGAR)), Decision::Hover { action_id: 20, champion_id: RENGAR });
    }

    #[test]
    fn remembers_completed_ban() {
        assert_eq!(session(vec![ban(10, 2, RENGAR, true, false)]).my_completed_ban(), None, "hovered isn't banned");
        assert_eq!(session(vec![ban(10, 2, RENGAR, false, true)]).my_completed_ban(), Some(RENGAR));
        assert_eq!(session(vec![ban(10, 1, 64, false, true)]).my_completed_ban(), None, "someone else's ban");
        let json = r#"{"localPlayerCellId":2,"actions":[[{"id":10,"actorCellId":2,"championId":107,"type":"ban","isInProgress":false,"completed":true,"pickTurn":1}],[{"id":20,"actorCellId":2,"championId":0,"type":"pick","isInProgress":false,"completed":false}]],"timer":{}}"#;
        let s: CsSession = serde_json::from_str(json).unwrap();
        assert_eq!(s.my_completed_ban(), Some(RENGAR));
    }
}
