//! Self-update from GitHub Releases (tauri-plugin-updater).
//!
//! - Checks `latest.json` on the repo's latest release at startup (after a short
//!   delay) and every 6 hours. Never installs on its own: the user clicks Update
//!   (settings page or tray).
//! - Every update is signed in CI with the project's private key; the app only
//!   installs packages whose signature matches the public key in tauri.conf.json.
//! - On Windows the NSIS installer runs in passive mode (progress bar only) and
//!   relaunches the app.

use crate::logger::log;
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

pub const FIRST_CHECK_DELAY: Duration = Duration::from_secs(20);
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Installing,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    pub phase: Phase,
    pub current: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    /// 0..1 while downloading, when the size is known.
    pub progress: Option<f64>,
    pub error: Option<String>,
    pub last_checked: Option<String>,
}

pub struct Updates {
    status: Mutex<UpdateStatus>,
    pending: Mutex<Option<Update>>,
}

impl Updates {
    pub fn new(current: &str) -> Self {
        Self {
            status: Mutex::new(UpdateStatus {
                phase: Phase::Idle,
                current: current.to_string(),
                version: None,
                notes: None,
                progress: None,
                error: None,
                last_checked: None,
            }),
            pending: Mutex::new(None),
        }
    }

    pub fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap().clone()
    }
}

fn set(app: &AppHandle, f: impl FnOnce(&mut UpdateStatus)) -> UpdateStatus {
    let st = {
        let u = app.state::<Updates>();
        let mut s = u.status.lock().unwrap();
        f(&mut s);
        s.clone()
    };
    let _ = app.emit("update", &st);
    crate::on_update_status(app, &st);
    st
}

fn updater(app: &AppHandle) -> Result<tauri_plugin_updater::Updater, String> {
    let mut b = app.updater_builder().timeout(Duration::from_secs(30));
    // For testing against a local feed only.
    if let Ok(url) = std::env::var("MINIMAP_NUDGE_UPDATE_URL") {
        let url = url.parse().map_err(|e| format!("bad update URL: {e}"))?;
        b = b.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    b.build().map_err(|e| e.to_string())
}

/// Ask GitHub whether there's a newer version. Safe to call any time.
pub async fn check(app: &AppHandle) -> UpdateStatus {
    {
        let phase = app.state::<Updates>().status().phase;
        if matches!(phase, Phase::Checking | Phase::Downloading | Phase::Installing) {
            return app.state::<Updates>().status();
        }
    }
    set(app, |s| {
        s.phase = Phase::Checking;
        s.error = None;
    });
    let result = match updater(app) {
        Ok(u) => u.check().await.map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    let now = chrono::Local::now().format("%H:%M").to_string();
    match result {
        Ok(Some(update)) => {
            log(&format!("update available: {} -> {}", update.current_version, update.version));
            let (version, notes) = (update.version.clone(), update.body.clone());
            *app.state::<Updates>().pending.lock().unwrap() = Some(update);
            set(app, |s| {
                s.phase = Phase::Available;
                s.version = Some(version);
                s.notes = notes.filter(|n| !n.trim().is_empty());
                s.last_checked = Some(now);
            })
        }
        Ok(None) => {
            *app.state::<Updates>().pending.lock().unwrap() = None;
            set(app, |s| {
                s.phase = Phase::UpToDate;
                s.version = None;
                s.notes = None;
                s.last_checked = Some(now);
            })
        }
        Err(e) => {
            log(&format!("update check failed: {e}"));
            set(app, |s| {
                s.phase = Phase::Error;
                s.error = Some(friendly(&e));
                s.last_checked = Some(now);
            })
        }
    }
}

/// Download, verify the signature, install, relaunch.
pub async fn install(app: &AppHandle) -> Result<(), String> {
    let pending = app.state::<Updates>().pending.lock().unwrap().clone();
    let update = match pending {
        Some(u) => u,
        None => {
            // Nothing cached (e.g. clicked from an old status) — check first.
            if check(app).await.phase != Phase::Available {
                return Err("No update available.".into());
            }
            app.state::<Updates>().pending.lock().unwrap().clone().ok_or("No update available.")?
        }
    };

    log(&format!("installing update {}", update.version));
    set(app, |s| {
        s.phase = Phase::Downloading;
        s.progress = Some(0.0);
        s.error = None;
    });

    let mut downloaded: u64 = 0;
    let mut last_emit = 0.0_f64;
    let progress_app = app.clone();
    let done_app = app.clone();
    let result = update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                if let Some(total) = total.filter(|t| *t > 0) {
                    let p = (downloaded as f64 / total as f64).min(1.0);
                    if p - last_emit >= 0.02 || p >= 1.0 {
                        last_emit = p;
                        set(&progress_app, |s| s.progress = Some(p));
                    }
                }
            },
            move || {
                set(&done_app, |s| {
                    s.phase = Phase::Installing;
                    s.progress = Some(1.0);
                });
            },
        )
        .await;

    match result {
        Ok(()) => {
            // Windows never gets here: the installer takes over and the app exits.
            log("update installed; restarting");
            app.restart();
        }
        Err(e) => {
            let msg = e.to_string();
            log(&format!("update failed: {msg}"));
            set(app, |s| {
                s.phase = Phase::Error;
                s.progress = None;
                s.error = Some(friendly(&msg));
            });
            Err(friendly(&msg))
        }
    }
}

fn friendly(e: &str) -> String {
    let l = e.to_lowercase();
    if l.contains("signature") {
        "The update's signature didn't match, so it wasn't installed.".into()
    } else if l.contains("dns") || l.contains("connect") || l.contains("timed out") || l.contains("network") {
        "Couldn't reach GitHub. Check your connection and try again.".into()
    } else if l.contains("404") || l.contains("status code") {
        "No release feed found on GitHub yet.".into()
    } else {
        format!("Update failed: {e}")
    }
}

/// Background checks: once shortly after launch, then every few hours.
pub fn spawn_periodic(app: AppHandle) {
    if std::env::var("MINIMAP_NUDGE_NO_UPDATE_CHECK").is_ok() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(
            std::env::var("MINIMAP_NUDGE_UPDATE_URL").map(|_| Duration::from_secs(2)).unwrap_or(FIRST_CHECK_DELAY),
        )
        .await;
        loop {
            check(&app).await;
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}
