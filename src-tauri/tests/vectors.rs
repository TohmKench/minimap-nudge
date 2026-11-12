//! Shared schedule vectors (also run against src/schedule.js by `npm test`).
use minimap_nudge_lib::schedule::{alert_times, next_alert_after, Phase};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Vectors {
    alert_times: Vec<AlertCase>,
    next_alert_after: Vec<NextCase>,
}
#[derive(Deserialize)]
struct AlertCase {
    name: String,
    phases: Vec<Phase>,
    until: f64,
    expected: Vec<f64>,
}
#[derive(Deserialize)]
struct NextCase {
    phases: Vec<Phase>,
    t: f64,
    expected: Option<f64>,
}

#[test]
fn shared_schedule_vectors() {
    let raw = include_str!("../../tests/schedule-vectors.json");
    let v: Vectors = serde_json::from_str(raw).unwrap();
    for c in &v.alert_times {
        assert_eq!(alert_times(&c.phases, c.until), c.expected, "case: {}", c.name);
    }
    for c in &v.next_alert_after {
        assert_eq!(next_alert_after(&c.phases, c.t), c.expected, "next after {}", c.t);
    }
    assert!(v.alert_times.len() >= 12);
}
