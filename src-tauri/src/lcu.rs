//! The League *client* (lobby, queue, champ select…), via its local LCU API.
//!
//! While the client runs it serves a local HTTPS API. Port and password come
//! from the `lockfile` in the League install folder, or from the
//! `LeagueClientUx` process's command line. We only read from it.
//!
//! The LCU API isn't officially supported by Riot. Anything unexpected here
//! just means "client state unknown"; game detection (league.rs) doesn't
//! depend on it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const SCAN_EVERY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creds {
    pub port: u16,
    pub password: String,
}

/// `LeagueClient:<pid>:<port>:<password>:https`
pub fn parse_lockfile(s: &str) -> Option<Creds> {
    let parts: Vec<&str> = s.trim().split(':').collect();
    if parts.len() < 5 {
        return None;
    }
    let port = parts[2].parse().ok()?;
    let password = parts[3].to_string();
    (!password.is_empty()).then_some(Creds { port, password })
}

/// `--app-port=1234 --remoting-auth-token=abc` from LeagueClientUx's command line.
pub fn parse_cmdline<S: AsRef<str>>(args: &[S]) -> Option<Creds> {
    let mut port = None;
    let mut token = None;
    for a in args {
        let a = a.as_ref().trim_matches('"');
        if let Some(v) = a.strip_prefix("--app-port=") {
            port = v.parse().ok();
        } else if let Some(v) = a.strip_prefix("--remoting-auth-token=") {
            token = Some(v.to_string());
        }
    }
    Some(Creds { port: port?, password: token.filter(|t| !t.is_empty())? })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    None,
    Lobby,
    Matchmaking,
    ReadyCheck,
    ChampSelect,
    GameStart,
    InProgress,
    Reconnect,
    PostGame,
    /// Spectating a live game (the client reports "WatchInProgress").
    Spectating,
    Other,
}

impl Phase {
    pub fn from_lcu(s: &str) -> Self {
        match s {
            "None" => Phase::None,
            "Lobby" => Phase::Lobby,
            "Matchmaking" => Phase::Matchmaking,
            "ReadyCheck" => Phase::ReadyCheck,
            "ChampSelect" => Phase::ChampSelect,
            "GameStart" => Phase::GameStart,
            "InProgress" => Phase::InProgress,
            "Reconnect" => Phase::Reconnect,
            "WaitingForStats" | "PreEndOfGame" | "EndOfGame" => Phase::PostGame,
            "WatchInProgress" => Phase::Spectating,
            _ => Phase::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    pub phase: Phase,
    pub queue_id: Option<i64>,
    pub queue: Option<String>,
    pub map_id: Option<i64>,
    pub party_size: Option<usize>,
    pub time_in_queue: Option<f64>,
    pub est_queue_time: Option<f64>,
    /// "Banning", "Picking", "Finalizing", "Planning"
    pub champ_select_stage: Option<String>,
    pub phase_time_left: Option<f64>,
    /// Your position in champ select: top / jungle / middle / bottom / utility (empty in blind pick).
    pub assigned_position: Option<String>,
}

/// Friendly names for common queues (the client's own descriptions vary by locale/patch).
pub fn queue_name(id: i64) -> Option<&'static str> {
    Some(match id {
        420 => "Ranked Solo/Duo",
        440 => "Ranked Flex",
        400 => "Normal Draft",
        430 => "Normal Blind",
        480 => "Swiftplay",
        490 => "Quickplay",
        700 | 720 => "Clash",
        450 => "ARAM",
        100 => "ARAM (Butcher's Bridge)",
        1700 | 1710 => "Arena",
        900 | 1010 | 1900 => "URF",
        830 | 840 | 850 | 870 | 880 | 890 => "Co-op vs. AI",
        1090 | 1100 | 1130 | 1160 => "TFT",
        0 => "Custom game",
        _ => return None,
    })
}

// ---- response shapes (only the fields we use; everything optional) ----

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Session {
    game_data: GameData,
    map: MapInfo,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct GameData {
    queue: Queue,
    is_custom_game: bool,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Queue {
    id: Option<i64>,
    map_id: Option<i64>,
    description: Option<String>,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct MapInfo {
    id: Option<i64>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Lobby {
    members: Vec<serde_json::Value>,
    game_config: LobbyConfig,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct LobbyConfig {
    queue_id: Option<i64>,
    map_id: Option<i64>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Search {
    time_in_queue: Option<f64>,
    estimated_queue_time: Option<f64>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct ChampSelect {
    timer: CsTimer,
    local_player_cell_id: Option<i64>,
    my_team: Vec<CsMember>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct CsMember {
    cell_id: Option<i64>,
    assigned_position: Option<String>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct CsTimer {
    adjusted_time_left_in_phase: Option<f64>,
    phase: Option<String>,
    is_infinite: bool,
}

pub struct Lcu {
    http: reqwest::Client,
    /// Champion id -> name, loaded once from the client's game data.
    champ_names: Mutex<std::collections::HashMap<i64, String>>,
    /// Folder to look in first (MINIMAP_NUDGE_LEAGUE_DIR, used for testing).
    dir_override: Option<PathBuf>,
    found_dir: Mutex<Option<PathBuf>>,
    last_scan: Mutex<Option<Instant>>,
    scan_cmdline: bool,
}

impl Lcu {
    pub fn new(dir_override: Option<PathBuf>) -> Self {
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true) // Riot's self-signed local cert
            .danger_accept_invalid_hostnames(true)
            .timeout(Duration::from_millis(1500))
            .connect_timeout(Duration::from_millis(800))
            .no_proxy()
            .build()
            .expect("lcu http client");
        let scan_cmdline = dir_override.is_none();
        Self {
            http,
            champ_names: Mutex::new(Default::default()),
            dir_override,
            found_dir: Mutex::new(None),
            last_scan: Mutex::new(None),
            scan_cmdline,
        }
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var_os("MINIMAP_NUDGE_LEAGUE_DIR").map(PathBuf::from))
    }

    /// Folder League *might* be installed in, best guesses first.
    fn candidate_dirs(&self) -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(d) = &self.dir_override {
            dirs.push(d.clone());
            return dirs;
        }
        if let Some(d) = self.found_dir.lock().unwrap().clone() {
            dirs.push(d);
        }
        dirs.extend(riot_installs_json());
        for drive in ["C", "D", "E", "F"] {
            dirs.push(PathBuf::from(format!(r"{drive}:\Riot Games\League of Legends")));
        }
        dirs.push(PathBuf::from("/Applications/League of Legends.app/Contents/LoL"));
        dirs
    }

    /// Port + password of the running client, or `None` if it isn't running.
    pub fn creds(&self) -> Option<Creds> {
        for d in self.candidate_dirs() {
            if let Some(c) = read_lockfile(&d) {
                *self.found_dir.lock().unwrap() = Some(d);
                return Some(c);
            }
        }
        if !self.scan_cmdline {
            return None;
        }
        // Not in a known place: look at running processes, at most every few seconds.
        {
            let mut last = self.last_scan.lock().unwrap();
            if last.is_some_and(|t| t.elapsed() < SCAN_EVERY) {
                return None;
            }
            *last = Some(Instant::now());
        }
        let (creds, dir) = scan_processes();
        if let Some(d) = dir {
            *self.found_dir.lock().unwrap() = Some(d);
        }
        creds
    }

    async fn get<T: for<'de> Deserialize<'de>>(&self, c: &Creds, path: &str) -> Option<T> {
        let res = self
            .http
            .get(format!("https://127.0.0.1:{}{}", c.port, path))
            .basic_auth("riot", Some(&c.password))
            .send()
            .await
            .ok()?;
        if !res.status().is_success() {
            return None;
        }
        res.json::<T>().await.ok()
    }

    /// Just the gameflow phase (one cheap request). `None` = client not running/reachable.
    pub async fn phase(&self) -> Option<Phase> {
        let c = self.creds()?;
        let raw: String = self.get(&c, "/lol-gameflow/v1/gameflow-phase").await?;
        Some(Phase::from_lcu(&raw))
    }

    /// What the client is doing right now. `None` = client not running/reachable.
    pub async fn snapshot(&self) -> Option<ClientInfo> {
        let c = self.creds()?;
        let raw_phase: String = self.get(&c, "/lol-gameflow/v1/gameflow-phase").await?;
        let phase = Phase::from_lcu(&raw_phase);
        let mut info = ClientInfo {
            phase: phase.clone(),
            queue_id: None,
            queue: None,
            map_id: None,
            party_size: None,
            time_in_queue: None,
            est_queue_time: None,
            champ_select_stage: None,
            phase_time_left: None,
            assigned_position: None,
        };
        if phase == Phase::None {
            return Some(info);
        }

        if let Some(s) = self.get::<Session>(&c, "/lol-gameflow/v1/session").await {
            info.queue_id = s.game_data.queue.id;
            info.map_id = s.game_data.queue.map_id.or(s.map.id);
            info.queue = s
                .game_data
                .queue
                .id
                .and_then(|id| if s.game_data.is_custom_game { Some("Custom game") } else { queue_name(id) })
                .map(str::to_string)
                .or(s.game_data.queue.description.filter(|d| !d.trim().is_empty()));
        }

        if matches!(phase, Phase::Lobby | Phase::Matchmaking | Phase::ReadyCheck) {
            if let Some(l) = self.get::<Lobby>(&c, "/lol-lobby/v2/lobby").await {
                info.party_size = Some(l.members.len()).filter(|n| *n > 0);
                if info.queue_id.is_none() {
                    info.queue_id = l.game_config.queue_id;
                    info.queue = l.game_config.queue_id.and_then(queue_name).map(str::to_string);
                }
                info.map_id = info.map_id.or(l.game_config.map_id);
            }
        }
        if matches!(phase, Phase::Matchmaking | Phase::ReadyCheck) {
            if let Some(s) = self.get::<Search>(&c, "/lol-matchmaking/v1/search").await {
                info.time_in_queue = s.time_in_queue;
                info.est_queue_time = s.estimated_queue_time.filter(|t| *t > 0.0);
            }
        }
        if phase == Phase::ChampSelect {
            if let Some(cs) = self.get::<ChampSelect>(&c, "/lol-champ-select/v1/session").await {
                info.champ_select_stage = cs.timer.phase.as_deref().and_then(stage_name).map(str::to_string);
                if !cs.timer.is_infinite {
                    info.phase_time_left = cs.timer.adjusted_time_left_in_phase.map(|ms| (ms / 1000.0).max(0.0));
                }
                info.assigned_position = cs
                    .my_team
                    .iter()
                    .find(|m| m.cell_id.is_some() && m.cell_id == cs.local_player_cell_id)
                    .and_then(|m| m.assigned_position.as_deref())
                    .map(|p| p.trim().to_ascii_lowercase())
                    .filter(|p| crate::settings::role_name(p).is_some());
            }
        }
        Some(info)
    }
}

/// `/lol-matchmaking/v1/ready-check`
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ReadyCheck {
    /// "Invalid" | "InProgress" | "EveryoneReady" | "StrangerNotReady" | "PartyNotReady" | "Error"
    pub state: String,
    /// "None" | "Accepted" | "Declined"
    pub player_response: String,
    pub timer: f64,
}

impl Lcu {
    pub async fn ready_check(&self) -> Option<ReadyCheck> {
        let c = self.creds()?;
        self.get(&c, "/lol-matchmaking/v1/ready-check").await
    }

    pub async fn champ_select_session(&self) -> Option<crate::banassist::CsSession> {
        let c = self.creds()?;
        self.get(&c, "/lol-champ-select/v1/session").await
    }

    /// Hovers (doesn't lock) a champion for a champ select action. True if the client took it.
    pub async fn hover_champion(&self, action_id: i64, champion_id: i64) -> bool {
        let Some(c) = self.creds() else { return false };
        match self
            .http
            .patch(format!("https://127.0.0.1:{}/lol-champ-select/v1/session/actions/{action_id}", c.port))
            .basic_auth("riot", Some(&c.password))
            .json(&serde_json::json!({ "championId": champion_id }))
            .send()
            .await
        {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }

    /// Champion name for an id (e.g. 107 -> "Rengar"), from the client's own game data.
    pub async fn champion_name(&self, id: i64) -> Option<String> {
        if let Some(n) = self.champ_names.lock().unwrap().get(&id) {
            return Some(n.clone());
        }
        #[derive(Deserialize)]
        struct Summary {
            id: i64,
            name: String,
        }
        let c = self.creds()?;
        let list: Vec<Summary> = self.get(&c, "/lol-game-data/assets/v1/champion-summary.json").await?;
        let mut names = self.champ_names.lock().unwrap();
        names.extend(list.into_iter().filter(|s| s.id > 0).map(|s| (s.id, s.name)));
        names.get(&id).cloned()
    }

    /// Clicks "Accept" on the match-found popup. True if the client took it.
    pub async fn accept_ready_check(&self) -> bool {
        let Some(c) = self.creds() else { return false };
        match self
            .http
            .post(format!("https://127.0.0.1:{}/lol-matchmaking/v1/ready-check/accept", c.port))
            .basic_auth("riot", Some(&c.password))
            .send()
            .await
        {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        }
    }
}

fn stage_name(p: &str) -> Option<&'static str> {
    Some(match p {
        "PLANNING" => "Planning",
        "BAN_PICK" => "Picks & bans",
        "FINALIZATION" => "Finalizing",
        "GAME_STARTING" => "Starting",
        _ => return None,
    })
}

fn read_lockfile(dir: &Path) -> Option<Creds> {
    parse_lockfile(&std::fs::read_to_string(dir.join("lockfile")).ok()?)
}

/// `C:\ProgramData\Riot Games\RiotClientInstalls.json` maps installed games to folders.
fn riot_installs_json() -> Vec<PathBuf> {
    let base = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let Ok(text) = std::fs::read_to_string(base.join("Riot Games").join("RiotClientInstalls.json")) else {
        return vec![];
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { return vec![] };
    let mut out = Vec::new();
    let mut collect = |v: &serde_json::Value| {
        if let Some(obj) = v.as_object() {
            for k in obj.keys() {
                if k.to_lowercase().contains("league of legends") {
                    let p = PathBuf::from(k.replace('/', "\\"));
                    let dir = if p.extension().is_some() { p.parent().map(Path::to_path_buf) } else { Some(p) };
                    out.extend(dir);
                }
            }
        }
    };
    collect(json.get("associated_client").unwrap_or(&serde_json::Value::Null));
    out
}

/// Find a running League client process: credentials from its command line,
/// and its folder (for the lockfile next time).
fn scan_processes() -> (Option<Creds>, Option<PathBuf>) {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let Ok(mut sys) = System::new() else {
        return (None, None);
    };
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always).with_cmd(UpdateKind::Always),
    );
    let mut creds = None;
    let mut dir = None;
    for p in sys.processes().values() {
        let name = p.name().to_string_lossy().to_lowercase();
        if !(name.starts_with("leagueclientux") || name.starts_with("leagueclient")) {
            continue;
        }
        if creds.is_none() {
            let args: Vec<String> = p.cmd().iter().map(|a| a.to_string_lossy().into_owned()).collect();
            creds = parse_cmdline(&args);
        }
        if dir.is_none() {
            if let Some(exe) = p.exe() {
                // LeagueClientUx.exe lives in the install folder (or one level down on some setups).
                for d in exe.ancestors().skip(1).take(2) {
                    if d.join("lockfile").is_file() {
                        dir = Some(d.to_path_buf());
                        break;
                    }
                }
            }
        }
    }
    if creds.is_none() {
        creds = dir.as_deref().and_then(read_lockfile);
    }
    (creds, dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lockfile_and_cmdline() {
        assert_eq!(
            parse_lockfile("LeagueClient:12345:54321:Xy-z_9:https\n"),
            Some(Creds { port: 54321, password: "Xy-z_9".into() })
        );
        assert_eq!(parse_lockfile("garbage"), None);
        assert_eq!(parse_lockfile("LeagueClient:1:notaport:pw:https"), None);
        assert_eq!(
            parse_cmdline(&["C:\\x\\LeagueClientUx.exe", "\"--remoting-auth-token=abc123\"", "--app-port=60123", "--other"]),
            Some(Creds { port: 60123, password: "abc123".into() })
        );
        assert_eq!(parse_cmdline(&["--app-port=1"]), None);
    }

    #[test]
    fn phases_and_queues() {
        assert_eq!(Phase::from_lcu("ChampSelect"), Phase::ChampSelect);
        assert_eq!(Phase::from_lcu("EndOfGame"), Phase::PostGame);
        assert_eq!(Phase::from_lcu("SomethingNew"), Phase::Other);
        assert_eq!(queue_name(420), Some("Ranked Solo/Duo"));
        assert_eq!(queue_name(123456), None);
    }

    #[test]
    fn missing_client_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let lcu = Lcu::new(Some(dir.path().to_path_buf()));
        assert_eq!(lcu.creds(), None);
    }
}
