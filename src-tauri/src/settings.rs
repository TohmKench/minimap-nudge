//! Settings persisted as JSON — same schema as the Electron version.

use crate::schedule::{normalize, Phase};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub struct BuiltinSound {
    pub id: &'static str,
    pub name: &'static str,
    pub bytes: &'static [u8],
}

pub const BUILTIN_SOUNDS: &[BuiltinSound] = &[
    BuiltinSound { id: "builtin:chime", name: "Chime", bytes: include_bytes!("../sounds/chime.wav") },
    BuiltinSound { id: "builtin:ping", name: "Sonar ping", bytes: include_bytes!("../sounds/ping.wav") },
    BuiltinSound { id: "builtin:double", name: "Double beep", bytes: include_bytes!("../sounds/double-beep.wav") },
    BuiltinSound { id: "builtin:knock", name: "Soft knock", bytes: include_bytes!("../sounds/knock.wav") },
    BuiltinSound { id: "builtin:rise", name: "Rising blip", bytes: include_bytes!("../sounds/rise.wav") },
];

pub const DEFAULT_SOUND: &str = "builtin:chime";

/// League's position ids (as the client reports them in champ select) and display names.
pub const ROLES: &[(&str, &str)] = &[
    ("top", "Top"),
    ("jungle", "Jungle"),
    ("middle", "Mid"),
    ("bottom", "Bot"),
    ("utility", "Support"),
];

pub fn role_name(id: &str) -> Option<&'static str> {
    ROLES.iter().find(|(k, _)| *k == id).map(|(_, n)| *n)
}

pub const DEFAULT_HOTKEY: &str = "Alt+A";

/// Feedback when the auto-accept shortcut is pressed.
pub const SOUND_TOGGLE_ON: &[u8] = include_bytes!("../sounds/toggle-on.wav");
pub const SOUND_TOGGLE_OFF: &[u8] = include_bytes!("../sounds/toggle-off.wav");

/// "auto" = use the role assigned in champ select; otherwise a fixed role id.
pub const ROLE_AUTO: &str = "auto";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomSound {
    pub id: String,
    pub name: String,
    pub file: PathBuf,
}

/// The champion you last banned in one game mode + role (remembered for auto-hover).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastBan {
    pub champion_id: i64,
    pub name: String,
    /// Game mode it was banned in, for display (e.g. "Ranked Solo/Duo").
    #[serde(default)]
    pub queue: String,
    /// Role id it was banned in ("" = no assigned role, e.g. blind pick).
    #[serde(default)]
    pub role: String,
}

/// Key into `last_bans`: queue id + assigned role, e.g. "420:middle", "400:any".
pub fn ban_key(queue_id: Option<i64>, role: Option<&str>) -> String {
    let q = queue_id.map_or_else(|| "any".to_string(), |q| q.to_string());
    let r = role.map(str::trim).filter(|r| role_name(r).is_some()).unwrap_or("any");
    format!("{q}:{r}")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub enabled: bool,
    pub phases: Vec<Phase>,
    pub sound_id: String,
    pub volume: f64,
    pub custom_sounds: Vec<CustomSound>,
    pub summoners_rift_only: bool,
    pub launch_at_startup: bool,
    /// Custom schedules per role; a role without an entry uses `phases`.
    pub role_schedules: BTreeMap<String, Vec<Phase>>,
    /// `"auto"` or a role id from `ROLES`.
    pub role_mode: String,
    /// Accept the "match found" popup automatically.
    pub auto_accept: bool,
    /// Seconds to wait before accepting (time to decline by hand).
    pub auto_accept_delay: f64,
    /// Global shortcut that toggles auto-accept, e.g. "Alt+A"; "" = none.
    pub auto_accept_hotkey: String,
    /// Hover your last ban for this game mode + role when your ban turn starts.
    pub auto_hover_ban: bool,
    /// Last ban per game mode + role, keyed by `ban_key`. (Replaces the old
    /// single `lastBan`, which is dropped.)
    pub last_bans: BTreeMap<String, LastBan>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            phases: vec![Phase { start_min: 0.0, interval_sec: Some(180.0) }],
            sound_id: DEFAULT_SOUND.into(),
            volume: 0.8,
            custom_sounds: vec![],
            summoners_rift_only: true,
            launch_at_startup: false,
            role_schedules: BTreeMap::new(),
            role_mode: ROLE_AUTO.into(),
            auto_accept: false,
            auto_accept_delay: 2.0,
            auto_accept_hotkey: DEFAULT_HOTKEY.into(),
            auto_hover_ban: false,
            last_bans: BTreeMap::new(),
        }
    }
}

impl Settings {
    pub fn sanitized(mut self) -> Self {
        self.phases = normalize(&self.phases);
        self.role_schedules = std::mem::take(&mut self.role_schedules)
            .into_iter()
            .filter(|(k, _)| role_name(k).is_some())
            .map(|(k, v)| (k, normalize(&v)))
            .collect();
        if self.role_mode != ROLE_AUTO && role_name(&self.role_mode).is_none() {
            self.role_mode = ROLE_AUTO.into();
        }
        self.auto_accept_delay =
            if self.auto_accept_delay.is_finite() { self.auto_accept_delay.clamp(0.0, 10.0) } else { 2.0 };
        self.auto_accept_hotkey = self.auto_accept_hotkey.trim().to_string();
        self.last_bans.retain(|_, b| b.champion_id > 0);
        self.custom_sounds.retain(|c| !c.id.is_empty() && c.file.is_file());
        let known = BUILTIN_SOUNDS.iter().any(|b| b.id == self.sound_id)
            || self.custom_sounds.iter().any(|c| c.id == self.sound_id);
        if !known {
            self.sound_id = DEFAULT_SOUND.into();
        }
        self.volume = if self.volume.is_finite() { self.volume.clamp(0.0, 1.0) } else { 0.8 };
        self
    }

    /// The schedule to use for `role` (its own if customised, else the default).
    pub fn phases_for(&self, role: Option<&str>) -> &[Phase] {
        role.and_then(|r| self.role_schedules.get(r)).map(Vec::as_slice).unwrap_or(&self.phases)
    }

    /// Shallow-merge a JSON patch (like `{...settings, ...patch}` in JS).
    pub fn patched(&self, patch: &serde_json::Value) -> Self {
        let mut base = serde_json::to_value(self).expect("settings serialize");
        if let (Some(obj), Some(p)) = (base.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                obj.insert(k.clone(), v.clone());
            }
        }
        serde_json::from_value::<Settings>(base).unwrap_or_else(|_| self.clone()).sanitized()
    }
}

pub struct SettingsStore {
    file: PathBuf,
    pub sounds_dir: PathBuf,
    data: Settings,
}

impl SettingsStore {
    pub fn open(dir: &Path) -> io::Result<Self> {
        let sounds_dir = dir.join("sounds");
        fs::create_dir_all(&sounds_dir)?;
        let file = dir.join("settings.json");
        let data = fs::read_to_string(&file)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .unwrap_or_default()
            .sanitized();
        Ok(Self { file, sounds_dir, data })
    }

    pub fn get(&self) -> &Settings {
        &self.data
    }

    pub fn set(&mut self, s: Settings) -> io::Result<&Settings> {
        self.data = s.sanitized();
        let tmp = self.file.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.data)?)?;
        fs::rename(&tmp, &self.file)?;
        Ok(&self.data)
    }

    pub fn update(&mut self, patch: &serde_json::Value) -> io::Result<&Settings> {
        let next = self.data.patched(patch);
        self.set(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_and_sanitize() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SettingsStore::open(dir.path()).unwrap();
        assert_eq!(store.get(), &Settings::default());

        store
            .update(&json!({"volume": 7, "soundId": "custom:gone", "phases": [{"startMin": 10, "intervalSec": 60}]}))
            .unwrap();
        let s = store.get().clone();
        assert_eq!(s.volume, 1.0);
        assert_eq!(s.sound_id, DEFAULT_SOUND);
        assert_eq!(s.phases.len(), 2, "phase at 0 is added");

        // custom sound whose file vanished is dropped on reload
        let f = store.sounds_dir.join("x.mp3");
        fs::write(&f, b"x").unwrap();
        store
            .update(&json!({"customSounds": [{"id": "custom:x", "name": "x", "file": f}], "soundId": "custom:x"}))
            .unwrap();
        assert_eq!(store.get().sound_id, "custom:x");
        fs::remove_file(&f).unwrap();
        let reopened = SettingsStore::open(dir.path()).unwrap();
        assert!(reopened.get().custom_sounds.is_empty());
        assert_eq!(reopened.get().sound_id, DEFAULT_SOUND);
    }

    #[test]
    fn role_schedules_sanitized() {
        let s = Settings::default().patched(&json!({
            "roleSchedules": {
                "jungle": [{"startMin": 0, "intervalSec": 60}],
                "utility": [{"startMin": 5, "intervalSec": 5}],
                "fill": [{"startMin": 0, "intervalSec": 60}]
            },
            "roleMode": "carry"
        }));
        assert_eq!(s.role_schedules.keys().collect::<Vec<_>>(), vec!["jungle", "utility"], "unknown role dropped");
        assert_eq!(s.role_schedules["utility"][0].start_min, 0.0, "normalized: phase at 0 added");
        assert_eq!(s.role_schedules["utility"][1].interval_sec, Some(15.0), "clamped");
        assert_eq!(s.role_mode, ROLE_AUTO, "bad role mode falls back to auto");
        assert_eq!(s.phases_for(Some("jungle"))[0].interval_sec, Some(60.0));
        assert_eq!(s.phases_for(Some("top")), s.phases.as_slice(), "uncustomised role uses default");
        assert_eq!(s.phases_for(None), s.phases.as_slice());
    }

    #[test]
    fn auto_accept_defaults_and_clamp() {
        let s = Settings::default();
        assert!(!s.auto_accept, "off by default");
        assert_eq!((s.auto_accept_delay, s.auto_accept_hotkey.as_str()), (2.0, "Alt+A"));
        let s = s.patched(&json!({"autoAccept": true, "autoAcceptDelay": 99, "autoAcceptHotkey": "  Ctrl+Shift+F9 "}));
        assert_eq!((s.auto_accept, s.auto_accept_delay, s.auto_accept_hotkey.as_str()), (true, 10.0, "Ctrl+Shift+F9"));
    }

    #[test]
    fn reads_electron_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"enabled":false,"phases":[{"startMin":0,"intervalSec":180},{"startMin":25,"intervalSec":null}],
               "soundId":"builtin:knock","volume":0.5,"customSounds":[],"summonersRiftOnly":false,"launchAtStartup":true}"#,
        )
        .unwrap();
        let s = SettingsStore::open(dir.path()).unwrap().get().clone();
        assert!(!s.enabled && !s.summoners_rift_only && s.launch_at_startup);
        assert_eq!(s.phases[1].interval_sec, None);
        assert_eq!(s.sound_id, "builtin:knock");
    }

    #[test]
    fn bans_are_keyed_by_mode_and_role() {
        assert_eq!(ban_key(Some(420), Some("middle")), "420:middle");
        assert_eq!(ban_key(Some(400), None), "400:any");
        assert_eq!(ban_key(Some(400), Some("")), "400:any");
        assert_eq!(ban_key(None, Some("nonsense")), "any:any");
        // the old single lastBan is dropped; zero ids are cleaned out
        let s: Settings = serde_json::from_value(json!({
            "lastBan": {"championId": 246, "name": "Qiyana"},
            "lastBans": {
                "420:middle": {"championId": 107, "name": "Rengar", "queue": "Ranked Solo/Duo", "role": "middle"},
                "400:top": {"championId": 0, "name": "None"}
            }
        }))
        .unwrap();
        let s = s.sanitized();
        assert_eq!(s.last_bans.len(), 1);
        assert_eq!(s.last_bans["420:middle"].name, "Rengar");
    }
}
