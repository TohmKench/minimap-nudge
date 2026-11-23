//! Auto-hover your last ban in champ select. Pure decision logic; lib.rs does
//! the LCU calls.
//!
//! - Remembers the champion you last *completed* a ban on. Ban actions belong
//!   to seats (cells), and a pick-order swap moves you to another seat (your
//!   `localPlayerCellId` changes), even during the ranked ban phase where the
//!   whole team bans at once. So a ban is yours if it was in progress on the
//!   seat you were in at the last poll before it locked: swap away and your
//!   old seat's ban is your teammate's; swap onto a seat and its ban is yours.
//! - The role used for the per-role memory is the one seen when champ select
//!   started, so a swap (or a moment where the client reports the role of the
//!   seat you just left or joined) can't file the ban under a teammate's role.
//! - When your ban turn starts, hovers it (never locks: you still click Ban).
//! - Hovers once per ban turn and never replaces something you've hovered,
//!   and skips a champion that's already been banned.
//! - Only hovers in the BAN_PICK stage. In draft, the client already marks the
//!   ban action "in progress" during PLANNING (declare your pick) and accepts a
//!   hover then, but clears it when bans actually open. So we wait for
//!   BAN_PICK, and check the hover is still there afterwards; if the client
//!   dropped it (back to nothing), we try again.

use serde::Deserialize;
use std::collections::HashMap;

const MAX_ATTEMPTS: u8 = 3;
/// Polls (about 1 s each) to wait for a sent hover to show up before resending.
const CONFIRM_POLLS: u8 = 3;

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

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CsTimer {
    /// "PLANNING" | "BAN_PICK" | "FINALIZATION" | "GAME_STARTING" | ...
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CsMember {
    pub cell_id: Option<i64>,
    pub assigned_position: Option<String>,
}

/// The parts of `/lol-champ-select/v1/session` this needs.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct CsSession {
    pub local_player_cell_id: Option<i64>,
    pub actions: Vec<Vec<CsAction>>,
    pub timer: CsTimer,
    pub my_team: Vec<CsMember>,
}

impl CsSession {
    fn all(&self) -> impl Iterator<Item = &CsAction> {
        self.actions.iter().flatten()
    }

    fn my_bans(&self) -> impl Iterator<Item = &CsAction> {
        let me = self.local_player_cell_id;
        self.all().filter(move |a| a.kind == "ban" && Some(a.actor_cell_id) == me)
    }


    /// Your assigned role in this champ select ("middle", ...), if the queue assigns one.
    pub fn my_position(&self) -> Option<String> {
        self.my_team
            .iter()
            .find(|m| m.cell_id.is_some() && m.cell_id == self.local_player_cell_id)
            .and_then(|m| m.assigned_position.as_deref())
            .map(|p| p.trim().to_ascii_lowercase())
            .filter(|p| crate::settings::role_name(p).is_some())
    }

    /// Bans are open. Blind pick / unknown stage (no timer phase) counts as open.
    fn bans_open(&self) -> bool {
        self.timer.phase.as_deref().map_or(true, |p| p == "BAN_PICK")
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
    /// Ban actions seen in progress this champ select: was it on your seat at the last poll?
    on_my_seat: HashMap<i64, bool>,
    /// Your completed bans already reported by `my_completed_ban`.
    reported: Vec<i64>,
    /// Your role, the first time champ select showed one.
    role: Option<String>,
    attempts: u8,
    /// A hover was sent and the client said OK; waiting to see it in the session.
    sent: Option<i64>,
    /// Polls since `sent` without the hover showing.
    unconfirmed_polls: u8,
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

    /// Your role for this champ select ("middle", ...), read once when it starts and
    /// kept: pick-order swaps don't change your role.
    pub fn role(&mut self, s: &CsSession) -> Option<String> {
        if self.role.is_none() {
            self.role = s.my_position();
        }
        self.role.clone()
    }

    /// Champion you just banned, reported once when the ban locks in.
    /// Call every poll, so your ban turn is seen while it's in progress.
    pub fn my_completed_ban(&mut self, s: &CsSession) -> Option<i64> {
        let me = s.local_player_cell_id;
        let mut newly = None;
        for a in s.all().filter(|a| a.kind == "ban") {
            if a.is_in_progress && !a.completed {
                self.on_my_seat.insert(a.id, Some(a.actor_cell_id) == me);
            } else if a.completed
                && a.champion_id > 0
                && self.on_my_seat.get(&a.id) == Some(&true)
                && !self.reported.contains(&a.id)
            {
                self.reported.push(a.id);
                newly = Some(a.champion_id);
            }
        }
        newly
    }

    pub fn step(&mut self, s: &CsSession, enabled: bool, last_ban: Option<i64>) -> Decision {
        let (Some(champ), true) = (last_ban.filter(|c| *c > 0), enabled) else {
            return Decision::Nothing;
        };
        if !s.bans_open() {
            return Decision::Nothing;
        }
        let Some(turn) = s.my_bans().find(|a| a.is_in_progress && !a.completed) else {
            return Decision::Nothing;
        };
        if self.action != Some(turn.id) {
            // a new ban turn (e.g. a second ban phase)
            self.action = Some(turn.id);
            self.attempts = 0;
            self.sent = None;
            self.unconfirmed_polls = 0;
            self.done = false;
        }
        if let Some(sent) = self.sent {
            if turn.champion_id == sent {
                // the client shows our hover: done for this turn
                self.sent = None;
                self.done = true;
                return Decision::Nothing;
            }
            if turn.champion_id != 0 {
                // you picked something else: leave it
                self.sent = None;
                self.done = true;
                return Decision::Nothing;
            }
            self.unconfirmed_polls += 1;
            if self.unconfirmed_polls < CONFIRM_POLLS {
                return Decision::Nothing;
            }
            // the client took the request but the hover never showed (or was cleared): resend
            self.sent = None;
            self.hovered = None;
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
            self.sent = Some(champion_id);
            self.unconfirmed_polls = 0;
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
        staged("BAN_PICK", actions)
    }
    fn staged(phase: &str, actions: Vec<CsAction>) -> CsSession {
        CsSession { local_player_cell_id: Some(2), actions: vec![actions], timer: CsTimer { phase: Some(phase.into()) }, my_team: vec![] }
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
        // the client may still report 0 for a moment: don't resend straight away
        assert_eq!(b.step(&s, true, Some(RENGAR)), Decision::Nothing);
        // it shows up: done, never sent again
        let shown = session(vec![ban(10, 2, RENGAR, true, false)]);
        for _ in 0..10 {
            assert_eq!(b.step(&shown, true, Some(RENGAR)), Decision::Nothing);
        }
        // even if you then clear it
        for _ in 0..10 {
            assert_eq!(b.step(&s, true, Some(RENGAR)), Decision::Nothing);
        }
    }

    #[test]
    fn waits_for_bans_to_open_in_draft() {
        // Draft: during PLANNING the client already marks the ban action in progress
        // and accepts a hover, then clears it when bans open. Don't hover then.
        let mut b = BanAssist::new();
        let planning = staged("PLANNING", vec![ban(10, 2, 0, true, false)]);
        for _ in 0..5 {
            assert_eq!(b.step(&planning, true, Some(RENGAR)), Decision::Nothing);
        }
        let open = session(vec![ban(10, 2, 0, true, false)]);
        assert_eq!(b.step(&open, true, Some(RENGAR)), Decision::Hover { action_id: 10, champion_id: RENGAR });
        // no stage at all (older client / blind): treated as open
        let mut b = BanAssist::new();
        let mut s = session(vec![ban(10, 2, 0, true, false)]);
        s.timer.phase = None;
        assert!(matches!(b.step(&s, true, Some(RENGAR)), Decision::Hover { .. }));
    }

    #[test]
    fn resends_when_the_client_drops_the_hover() {
        let mut b = BanAssist::new();
        let empty = session(vec![ban(10, 2, 0, true, false)]);
        assert!(matches!(b.step(&empty, true, Some(RENGAR)), Decision::Hover { .. }));
        b.hovered(RENGAR, true); // client said 204...
        // ...but the action stays empty
        let mut sends = 1;
        for _ in 0..20 {
            if let Decision::Hover { .. } = b.step(&empty, true, Some(RENGAR)) {
                sends += 1;
                b.hovered(RENGAR, true);
            }
        }
        assert_eq!(sends, MAX_ATTEMPTS as i32, "bounded retries");
    }

    #[test]
    fn stops_if_you_hover_something_else_after_us() {
        let mut b = BanAssist::new();
        let empty = session(vec![ban(10, 2, 0, true, false)]);
        assert!(matches!(b.step(&empty, true, Some(RENGAR)), Decision::Hover { .. }));
        b.hovered(RENGAR, true);
        let lee = session(vec![ban(10, 2, 64, true, false)]);
        assert_eq!(b.step(&lee, true, Some(RENGAR)), Decision::Nothing);
        for _ in 0..10 {
            assert_eq!(b.step(&empty, true, Some(RENGAR)), Decision::Nothing);
        }
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
        let mut b = BanAssist::new();
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 2, RENGAR, true, false)])), None, "hovered isn't banned");
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 2, RENGAR, false, true)])), Some(RENGAR), "seen in progress, then locked");
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 2, RENGAR, false, true)])), None, "reported once");
        let mut b = BanAssist::new();
        let s = session(vec![ban(10, 1, 64, true, false)]);
        b.my_completed_ban(&s);
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 1, 64, false, true)])), None, "someone else's ban");
        let mut b = BanAssist::new();
        let json = r#"{"localPlayerCellId":2,"actions":[[{"id":10,"actorCellId":2,"championId":107,"type":"ban","isInProgress":true,"completed":false,"pickTurn":1}],[{"id":20,"actorCellId":2,"championId":0,"type":"pick","isInProgress":false,"completed":false}]],"timer":{}}"#;
        let s: CsSession = serde_json::from_str(json).unwrap();
        assert_eq!(b.my_completed_ban(&s), None);
        let s: CsSession = serde_json::from_str(&json.replace(r#""isInProgress":true,"completed":false"#, r#""isInProgress":false,"completed":true"#)).unwrap();
        assert_eq!(b.my_completed_ban(&s), Some(RENGAR));
    }

    #[test]
    fn pick_order_swap_keeps_your_own_ban() {
        const ZED: i64 = 238;
        let mut b = BanAssist::new();
        // you're cell 2 and ban Rengar; your teammate in cell 4 bans Zed
        let during = session(vec![ban(10, 2, RENGAR, true, false), ban(11, 4, ZED, true, false)]);
        assert_eq!(b.my_completed_ban(&during), None);
        let banned = vec![ban(10, 2, RENGAR, false, true), ban(11, 4, ZED, false, true)];
        assert_eq!(b.my_completed_ban(&session(banned.clone())), Some(RENGAR));
        // you swap pick order with cell 4: the client now says you're cell 4
        let mut swapped = session(banned);
        swapped.local_player_cell_id = Some(4);
        for _ in 0..3 {
            assert_eq!(b.my_completed_ban(&swapped), None, "Zed is your teammate's ban");
        }
    }

    #[test]
    fn swap_during_ranked_bans() {
        // Ranked: the whole team bans at once. You (cell 2) hover Locke, then swap
        // pick order with cell 4 while their Draven ban is still in progress.
        const LOCKE: i64 = 9001;
        const DRAVEN: i64 = 119;
        let mut b = BanAssist::new();
        let mut s = session(vec![ban(10, 2, LOCKE, true, false), ban(11, 4, DRAVEN, true, false)]);
        assert_eq!(b.my_completed_ban(&s), None);
        s.local_player_cell_id = Some(4);
        assert_eq!(b.my_completed_ban(&s), None);
        // your teammate (now in cell 2) locks Locke: not your ban
        let s2 = |locke_done: bool, draven_done: bool| {
            let mut s = session(vec![ban(10, 2, LOCKE, !locke_done, locke_done), ban(11, 4, DRAVEN, !draven_done, draven_done)]);
            s.local_player_cell_id = Some(4);
            s
        };
        assert_eq!(b.my_completed_ban(&s2(true, false)), None);
        // you lock Draven on your new seat: that's what you banned
        assert_eq!(b.my_completed_ban(&s2(true, true)), Some(DRAVEN));
        assert_eq!(b.my_completed_ban(&s2(true, true)), None);

        // Same swap, but you'd locked Locke before swapping: Locke is yours, and
        // your teammate's later Draven isn't.
        let mut b = BanAssist::new();
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 2, LOCKE, true, false), ban(11, 4, DRAVEN, true, false)])), None);
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 2, LOCKE, false, true), ban(11, 4, DRAVEN, true, false)])), Some(LOCKE));
        let mut after = session(vec![ban(10, 2, LOCKE, false, true), ban(11, 4, DRAVEN, true, false)]);
        after.local_player_cell_id = Some(4);
        assert_eq!(b.my_completed_ban(&after), None);
        // Draven's seat is yours now and it was still in progress, so if it
        // locks now, you locked it
        let mut locked = session(vec![ban(10, 2, LOCKE, false, true), ban(11, 4, DRAVEN, false, true)]);
        locked.local_player_cell_id = Some(4);
        assert_eq!(b.my_completed_ban(&locked), Some(DRAVEN));
    }

    #[test]
    fn role_is_kept_through_a_swap() {
        let member = |cell, pos: &str| CsMember { cell_id: Some(cell), assigned_position: Some(pos.into()) };
        let mut b = BanAssist::new();
        let mut s = session(vec![]);
        s.my_team = vec![];
        assert_eq!(b.role(&s), None, "team not loaded yet");
        s.my_team = vec![member(2, "bottom"), member(4, "jungle")];
        assert_eq!(b.role(&s).as_deref(), Some("bottom"));
        // after a swap the client says you're cell 4 (and, for a moment or for good,
        // that cell's role): you're still bottom
        s.local_player_cell_id = Some(4);
        assert_eq!(b.role(&s).as_deref(), Some("bottom"));
        b.reset();
        assert_eq!(b.role(&s).as_deref(), Some("jungle"), "next champ select reads it again");
    }

    #[test]
    fn joining_after_bans_remembers_nothing() {
        // app started mid champ select: we never saw the ban in progress, so don't guess
        let mut b = BanAssist::new();
        assert_eq!(b.my_completed_ban(&session(vec![ban(10, 2, RENGAR, false, true)])), None);
    }

    #[test]
    fn reads_my_assigned_role() {
        let json = r#"{"localPlayerCellId":2,"myTeam":[{"cellId":0,"assignedPosition":"top"},{"cellId":2,"assignedPosition":"MIDDLE"}]}"#;
        let s: CsSession = serde_json::from_str(json).unwrap();
        assert_eq!(s.my_position().as_deref(), Some("middle"));
        let blind = r#"{"localPlayerCellId":2,"myTeam":[{"cellId":2,"assignedPosition":""}]}"#;
        assert_eq!(serde_json::from_str::<CsSession>(blind).unwrap().my_position(), None);
    }
}
