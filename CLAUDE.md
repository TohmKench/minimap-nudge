# Minimap Nudge — notes for Claude

Windows tray app (Tauri 2 + Rust) that detects a League of Legends match through Riot's local Live Client Data API and plays a reminder sound on a configurable schedule. Owner: Thomas (TohmKench). The repo is public, and installed copies update themselves from this repo's GitHub Releases.

## Layout
- `src/`: settings page (plain HTML/CSS/JS, no build step). `bridge.js` exposes `window.nudge` on top of Tauri IPC. `schedule.js` is the JS copy of the scheduler, used only for the timeline preview.
- `src-tauri/src/`:
  - `lib.rs`: tray, window lifecycle (the window is destroyed on close; the app lives in the tray), commands, polling loop
  - `watcher.rs`: pure game state machine, which also merges in the client state for the status line
  - `league.rs`: in-game API client (Live Client Data, port 2999; official)
  - `lcu.rs`: League *client* API (lobby/queue/champ select). Credentials come from the `lockfile` in the install folder or the LeagueClientUx command line. Read-only and unofficial: failures must degrade to "Waiting for League", never break game detection
  - `schedule.rs`: the real scheduler
  - `settings.rs`: JSON settings; built-in sounds are embedded
  - `audio.rs`: symphonia decode + rodio playback
  - `autoaccept.rs`: pure decision logic for auto-accepting the ready check (lib.rs does the LCU GET/POST)
  - `updates.rs`: self-updater
  - `logger.rs`
- `tests/schedule-vectors.json`: shared cases that BOTH schedulers must pass. If you change scheduling rules, update the vectors and both `schedule.rs` and `src/schedule.js`.

## Commands
```bash
npm test                 # JS vectors + all Rust tests (must pass before any release)
npm run dev              # run locally
npm run mock-game -- --start 170 --speed 10   # fake League match on :2999
npm run mock-game -- --client --league-dir /tmp/fake-league   # fake client: lobby -> queue -> champ select -> game
MINIMAP_NUDGE_LEAGUE_DIR=/tmp/fake-league npm run dev            # point the app at that fake client
```
On Linux the Tauri system deps are needed (webkit2gtk-4.1, ayatana-appindicator, librsvg, alsa).

## Releasing an update (this is how users get new versions)
1. Make the change, run `npm test`, and commit it to `main`.
2. `npm run release -- <x.y.z> "<short user-facing notes>"`
   - This bumps the version in tauri.conf.json, package.json, package-lock.json, Cargo.toml and Cargo.lock, writes the notes to `RELEASE_NOTES.md`, commits "Release vX" and pushes `main`. No tags are pushed; some environments block tag pushes.
   - Use a patch version for fixes, minor for features. It must be higher than the current version.
3. `.github/workflows/release.yml` runs on every push to main. If there's no release for that version yet, it runs the tests, builds the NSIS installer on windows-latest, signs it, creates tag + GitHub Release `vX.Y.Z` (notes from RELEASE_NOTES.md) and uploads `latest.json`. You can re-run it from the Actions tab (workflow_dispatch).
4. Installed apps check `releases/latest/download/latest.json` at startup and every 6 h, show "Update to X" in settings and the tray, and install when the user clicks it.

Check a release: `curl -sL https://github.com/TohmKench/minimap-nudge/releases/latest/download/latest.json`

## Rules
- NEVER commit the signing key (`*.key`). It lives only in the repo secret `TAURI_SIGNING_PRIVATE_KEY` and in Thomas's backup.
- Never change `plugins.updater.pubkey` or `endpoints` in tauri.conf.json. Installed copies would stop accepting updates.
- Don't hand-edit version numbers; use the release script.
- Don't delete or move an existing release/tag. If a release is broken, fix it and release a new patch version.
- `bundle.createUpdaterArtifacts` is on, so a local `tauri build` needs `TAURI_SIGNING_PRIVATE_KEY`. Use `npx tauri build --no-bundle` for local compile checks.
- Pushing needs GitHub credentials on the machine (`gh auth login`, or an SSH key added to TohmKench).

## Behaviour to preserve
- Role profiles: `roleSchedules` (per role id: top/jungle/middle/bottom/utility) override the default `phases`. `roleMode` is "auto" (role from champ select's assignedPosition, kept through the game and cleared back in lobby) or a fixed role. `Watcher::effective()` swaps in the active role's schedule; always schedule through it.
- Auto-accept is the only thing that *writes* to the client (POST /lol-matchmaking/v1/ready-check/accept). It's off by default, waits `autoAcceptDelay` (default 2 s), accepts once per pop and never overrides a manual accept or decline. Don't add other client automation (auto pick/ban/lock, chat) without asking Thomas: that's a much greyer area.
- The global shortcut (`autoAcceptHotkey`, default Alt+A, tauri-plugin-global-shortcut) toggles auto-accept and plays toggle-on/off sounds. It's re-registered whenever the setting changes, and released while the settings page is recording a new one.
- Never add enemy summoner spell, ultimate or cooldown tracking/timers (manual or automatic). Riot's third-party policy prohibits it and it puts the user's account at risk.
- Outside a game the header/tray show the client phase (lobby, queue + timer, ready check, champ select + timer, loading, post-game). In-game state always wins over client state. LCU is polled only while not in a game.
- Pings follow the in-game clock. No backlog when joining mid-game; a ping more than 10 s late is skipped; quiet after GameEnd; "Summoner's Rift only" means mapNumber 11.
- Never auto-install updates; installing only happens when the user clicks.
- A bad audio file must never crash the app (decoders are wrapped in catch_unwind).
