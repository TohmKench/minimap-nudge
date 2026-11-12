//! Reminder schedule — a faithful port of `src/schedule.js` (the settings page
//! uses the JS copy for its timeline preview; `tests/schedule-vectors.json`
//! keeps the two in lock-step).
//!
//! A schedule is a list of phases, each starting at a game minute with an
//! interval in seconds, or `None` for a silent stretch.
//!
//! Rules:
//!  - No ping at 0:00.
//!  - A new phase begins with a ping at its start time.
//!  - Pings closer than `MIN_GAP_SEC` to the previous one are dropped.

use serde::{Deserialize, Deserializer, Serialize};

pub const MIN_INTERVAL_SEC: f64 = 15.0;
/// Never above MIN_INTERVAL_SEC, or short intervals get thinned out.
pub const MIN_GAP_SEC: f64 = 15.0;
pub const MAX_GAME_SEC: f64 = 4.0 * 60.0 * 60.0;
const DEFAULT_INTERVAL_SEC: f64 = 180.0;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Phase {
    pub start_min: f64,
    /// `None` = silent.
    pub interval_sec: Option<f64>,
}

/// Phase as it arrives from JSON, before validation. Distinguishes a missing
/// `intervalSec` (→ default) from an explicit `null` (→ silent), like the JS.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPhase {
    #[serde(default)]
    pub start_min: Option<serde_json::Value>,
    #[serde(default, deserialize_with = "present")]
    pub interval_sec: Option<Option<serde_json::Value>>,
    #[serde(default)]
    pub silent: Option<bool>,
}

fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<serde_json::Value>>, D::Error> {
    Ok(Some(Option::deserialize(d)?))
}

impl<'de> Deserialize<'de> for Phase {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = RawPhase::deserialize(d)?;
        Ok(normalize_one(&raw))
    }
}

fn as_number(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|n| n.is_finite())
}

fn normalize_one(raw: &RawPhase) -> Phase {
    let start_min = raw.start_min.as_ref().and_then(as_number).unwrap_or(0.0).max(0.0);
    let silent = raw.silent == Some(true)
        || matches!(raw.interval_sec, Some(None))
        || matches!(raw.interval_sec.as_ref().and_then(|o| o.as_ref()).and_then(as_number), Some(n) if n == 0.0);
    let interval_sec = if silent {
        None
    } else {
        let n = raw
            .interval_sec
            .as_ref()
            .and_then(|o| o.as_ref())
            .and_then(as_number)
            .filter(|n| *n != 0.0)
            .unwrap_or(DEFAULT_INTERVAL_SEC);
        Some(n.round().max(MIN_INTERVAL_SEC))
    };
    Phase { start_min, interval_sec }
}

/// Sort, collapse duplicate start times (last wins), guarantee a phase at 0.
pub fn normalize(phases: &[Phase]) -> Vec<Phase> {
    let mut list: Vec<Phase> = phases
        .iter()
        .map(|p| Phase {
            start_min: if p.start_min.is_finite() { p.start_min.max(0.0) } else { 0.0 },
            interval_sec: p.interval_sec.filter(|n| n.is_finite() && *n != 0.0).map(|n| n.round().max(MIN_INTERVAL_SEC)),
        })
        .collect();
    list.sort_by(|a, b| a.start_min.total_cmp(&b.start_min)); // stable
    let mut out: Vec<Phase> = Vec::with_capacity(list.len());
    for p in list {
        match out.last_mut() {
            Some(last) if last.start_min == p.start_min => *last = p,
            _ => out.push(p),
        }
    }
    if out.first().map_or(true, |p| p.start_min != 0.0) {
        let interval = out.first().map_or(Some(DEFAULT_INTERVAL_SEC), |p| p.interval_sec);
        out.insert(0, Phase { start_min: 0.0, interval_sec: interval });
    }
    out
}

fn phase_index_at(phases: &[Phase], t: f64) -> usize {
    let mut idx = 0;
    for (i, p) in phases.iter().enumerate() {
        if p.start_min * 60.0 <= t {
            idx = i;
        } else {
            break;
        }
    }
    idx
}

/// All ping times (seconds of game time) up to and including `until_sec`.
pub fn alert_times(raw: &[Phase], until_sec: f64) -> Vec<f64> {
    let phases = normalize(raw);
    let mut out = Vec::new();
    let mut cursor = 0.0_f64;
    let mut last_emitted = f64::NEG_INFINITY;

    while cursor < until_sec {
        let idx = phase_index_at(&phases, cursor);
        let next_start = phases.get(idx + 1).map_or(f64::INFINITY, |p| p.start_min * 60.0);
        let mut candidate = phases[idx].interval_sec.map_or(f64::INFINITY, |i| cursor + i);
        if next_start <= candidate {
            candidate = next_start;
        }
        if !candidate.is_finite() || candidate > until_sec {
            break;
        }
        let silent_here = phases[phase_index_at(&phases, candidate)].interval_sec.is_none();
        if !silent_here && candidate > 0.0 && candidate - last_emitted >= MIN_GAP_SEC {
            last_emitted = candidate;
            out.push(candidate);
        }
        cursor = candidate;
    }
    out
}

/// First ping strictly after game time `t`, or `None`.
pub fn next_alert_after(phases: &[Phase], t: f64) -> Option<f64> {
    alert_times(phases, MAX_GAME_SEC).into_iter().find(|a| *a > t + 1e-6)
}

pub fn format_clock(sec: Option<f64>) -> String {
    match sec {
        Some(s) if s.is_finite() => {
            let s = s.max(0.0).floor() as u64;
            format!("{}:{:02}", s / 60, s % 60)
        }
        _ => "--:--".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ph(list: &[(f64, Option<f64>)]) -> Vec<Phase> {
        list.iter().map(|&(s, i)| Phase { start_min: s, interval_sec: i }).collect()
    }

    #[test]
    fn single_phase_every_3_min() {
        assert_eq!(alert_times(&ph(&[(0.0, Some(180.0))]), 900.0), vec![180.0, 360.0, 540.0, 720.0, 900.0]);
    }

    #[test]
    fn switch_to_1_min_at_10() {
        assert_eq!(
            alert_times(&ph(&[(0.0, Some(180.0)), (10.0, Some(60.0))]), 780.0),
            vec![180.0, 360.0, 540.0, 600.0, 660.0, 720.0, 780.0]
        );
    }

    #[test]
    fn slow_down_later() {
        assert_eq!(
            alert_times(&ph(&[(0.0, Some(60.0)), (3.0, Some(300.0))]), 1000.0),
            vec![60.0, 120.0, 180.0, 480.0, 780.0]
        );
    }

    #[test]
    fn no_double_beep() {
        assert_eq!(
            alert_times(&ph(&[(0.0, Some(180.0)), (190.0 / 60.0, Some(60.0))]), 320.0),
            vec![180.0, 250.0, 310.0]
        );
    }

    #[test]
    fn silent_and_resume() {
        assert_eq!(
            alert_times(&ph(&[(0.0, Some(300.0)), (12.0, None), (20.0, Some(120.0))]), 1500.0),
            vec![300.0, 600.0, 1200.0, 1320.0, 1440.0]
        );
    }

    #[test]
    fn trailing_silent_ends() {
        assert_eq!(alert_times(&ph(&[(0.0, Some(300.0)), (11.0, None)]), 5000.0), vec![300.0, 600.0]);
    }

    #[test]
    fn next_after_reconnect() {
        let p = ph(&[(0.0, Some(180.0)), (10.0, Some(60.0))]);
        assert_eq!(next_alert_after(&p, 0.0), Some(180.0));
        assert_eq!(next_alert_after(&p, 180.0), Some(360.0));
        assert_eq!(next_alert_after(&p, 545.3), Some(600.0));
        assert_eq!(next_alert_after(&p, 1234.0), Some(1260.0));
    }

    #[test]
    fn normalize_adds_zero_clamps_sorts() {
        assert_eq!(normalize(&ph(&[(5.0, Some(3.0))])), ph(&[(0.0, Some(15.0)), (5.0, Some(15.0))]));
        assert_eq!(normalize(&[]), ph(&[(0.0, Some(180.0))]));
    }

    #[test]
    fn json_missing_vs_null_interval() {
        let p: Vec<Phase> = serde_json::from_str(r#"[{"startMin":0},{"startMin":5,"intervalSec":null},{"startMin":"7","intervalSec":"90"}]"#).unwrap();
        assert_eq!(p, ph(&[(0.0, Some(180.0)), (5.0, None), (7.0, Some(90.0))]));
    }

    #[test]
    fn clock() {
        assert_eq!(format_clock(Some(605.9)), "10:05");
        assert_eq!(format_clock(None), "--:--");
    }
}
