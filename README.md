# Minimap Nudge (Tauri / Rust)

A Windows tray app that notices when you're in a League of Legends match and plays a sound every few minutes to remind you to check the minimap. You can change the interval as the game goes on and use built-in or uploaded sounds.

Version 2 is a rewrite of the Electron version. The settings page is the same, but everything that runs in the background is now Rust, and the settings window is fully closed when you close it. While you play, only one small native process is running.

| | 1.0 (Electron) | 2.0 (Tauri) |
|---|---|---|
| Download | 110 MB | 2.6 MB installer / 7 MB exe |
| Background processes | 6 | 1 (+ WebView2 only while settings are open) |

## Install

Download the `…_x64-setup.exe` installer from the [latest release](https://github.com/TohmKench/minimap-nudge/releases/latest). It installs for your user only (no admin needed) and adds a Start-menu entry.

## Updates

The app checks this repo's releases at startup and every 6 hours. When there's a new version, **Settings → Updates** and the tray menu show **Update to x.y.z** with the release notes. Click it and the app downloads the installer, verifies its signature, installs it and restarts. Nothing installs without your click. Updates only work for installs made with the installer, not for copies run from a zip.

The app needs the Microsoft Edge WebView2 Runtime, which Windows 10 and 11 already have. If it's missing, the installer downloads it. Neither file is code-signed, so SmartScreen will warn you: click **More info → Run anyway**.

Settings, uploaded sounds and the log are stored in `%APPDATA%\com.thomas.minimapnudge\`. Use **Open log folder** in the tray menu to get there. Settings files from 1.0 are compatible.

## Behaviour

- **Detection:** it polls Riot's official Live Client Data API (`https://127.0.0.1:2999`) once a second. Pings follow the in-game clock, so pauses and reconnects are handled.
- **Client tracking:** outside a game it reads the League client's local API to show what you're doing: lobby (queue and party size), in queue (timer and estimate), match found, champion select (stage, time left, and whether pings will play in that mode), loading, and post-game. The tray icon is grey when League is closed, dim teal while the client is open, and bright teal in a game. This API is read-only but not officially supported by Riot; if it changes, the app just shows "Waiting for League" until the game starts, and pinging is unaffected.
- **Schedule:** phases such as "every 3 min, from 10:00 every 1 min, from 25:00 silent".
  - Each phase pings when it starts.
  - Pings less than 15 s after the previous one are merged.
  - The shortest interval is 15 s.
  - If you join partway through a game, it picks up at the next ping without playing the ones you missed.
  - It goes quiet on the end-of-game screen.
- **Role schedules:** give Top, Jungle, Mid, Bot or Support its own schedule; a role without one uses the default. With **Auto**, your role is picked up in champion select (blind pick and ARAM have none, so the default is used). You can also fix the role in Settings or the tray's **Role** menu.
- **Replays and spectating:** no pings. The header shows "Watching a replay".
- **Auto-hover your last ban:** the app remembers the champion you last banned and, when your next ban turn starts, hovers it for you. It never locks it: you still click Ban, or pick someone else. Ban a different champion and that becomes the new one. It won't hover over something you've already hovered, or a champion that's already banned. Toggle it in Settings → Queue & champ select or the tray.
- **Auto-accept:** when a match is found the app can accept it for you (Settings → Match found). It's off by default and waits 2 s (adjustable) so you can still decline; it never overrides a manual accept or decline. Toggle it from anywhere, even in game, with **Alt+A**: you'll hear a rising beep for on and a falling one for off. Click the shortcut box in Settings and press new keys to rebind it, or ✕ to remove it. The shortcut is reserved system-wide, so pick something you don't use in game. It uses the League client's local API (not officially supported by Riot; not allowed on the Korean server).
- **Sounds:**
  - 5 built-in sounds, plus uploads in mp3, wav, ogg, flac or m4a format (max 10 MB, the first 30 s are used).
  - Uploads are checked when you add them, and a file that can't be decoded is rejected.
  - The sound device is opened for each ping, so switching headsets doesn't break anything.
- **Tray menu:** status, Settings…, Alerts enabled, Test sound, Open log folder, Quit.
- **Options:** Summoner's Rift only, and Start with Windows (starts hidden in the tray).

## Development

You need Node 18+ and Rust 1.80+, plus the Tauri prerequisites for your OS (on Windows: the MSVC build tools and WebView2).

```bash
npm install
npm run dev          # run with live reload
npm test             # JS schedule tests + all Rust tests
npm run build        # Windows: installer in src-tauri/target/release/bundle/nsis/
npm run mock-game    # fake League match on :2999 (close League first)
npm run mock-game -- --start 590 --speed 10 --mode ARAM --end-after 60
npm run mock-game -- --client --league-dir C:\temp\fake-league   # fake client too; then run the app with
                                                               # MINIMAP_NUDGE_LEAGUE_DIR=C:\temp\fake-league
```

### Tests

| Suite | What it covers |
|---|---|
| `tests/schedule-vectors.json` | Shared cases run against **both** the Rust scheduler and the JS copy used for the settings-page preview, so they can't drift apart |
| `src-tauri/src/*` unit tests | Schedule, watcher state machine, settings (including reading 1.0 files), audio decoding of every format, junk and truncated files never crash, playback with no sound device |
| `src-tauri/tests/lcu_mock.rs` | The League-client code against a fake client: lockfile discovery, password, every phase, ARAM, client closing, wrong password |
| `src-tauri/tests/watcher_mock.rs` | The real HTTP client against an in-process fake League server (self-signed HTTPS) with a test-controlled clock: phase changes, joining mid-game, ARAM, GameEnd, disabled, game closing |
| `test/schedule.test.js` | The JS side of the shared vectors |

### Releasing

`npm run release -- 2.2.0 "What changed"` bumps the version everywhere, writes `RELEASE_NOTES.md`, commits and pushes `main`. GitHub Actions (`.github/workflows/release.yml`) sees the new version, then tests, builds and signs the installer, and publishes release `v2.2.0` with `latest.json`, which installed apps pick up. See `CLAUDE.md` for the full process and rules.

### Cross-compiling for Windows from Ubuntu

`scripts/build-windows-from-linux.sh` builds the Windows exe and installer on Ubuntu 24.04 (e.g. asgard), using MinGW plus a rebuilt standard library. The script explains the one-time setup. MinGW builds need `WebView2Loader.dll` next to the exe; the installer includes it automatically.

`.github/workflows/windows.yml` runs the tests and a compile check on GitHub's Windows machines for every push.

## Layout

```
src/                 settings page (HTML/CSS/JS) + bridge.js (Tauri IPC) + schedule.js
src-tauri/src/
  lib.rs             tray, window lifecycle, commands, polling loop
  watcher.rs         game state machine (pure) + poll_once
  league.rs          Live Client Data API client (in game)
  lcu.rs             League client API (lobby, queue, champ select)
  schedule.rs        phase → ping times (port of schedule.js)
  settings.rs        JSON settings + built-in sounds (embedded in the exe)
  audio.rs           symphonia decoding + rodio playback thread
  updates.rs         self-updater (GitHub Releases, signed)
  logger.rs          rotating log file
test/                JS tests, Node mock server, test certificate
tests/               shared schedule vectors, sample audio files
```
