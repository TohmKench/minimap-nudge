# Minimap Nudge — notes for Claude

Windows tray app (Tauri 2 + Rust) that detects a League of Legends match through Riot's local Live Client Data API and plays a reminder sound on a configurable schedule. Owner: Thomas (TohmKench). The repo is public, and installed copies update themselves from this repo's GitHub Releases.

## Layout
- `src/`: settings page (plain HTML/CSS/JS, no build step). `bridge.js` exposes `window.nudge` on top of Tauri IPC. `schedule.js` is the JS copy of the scheduler, used only for the timeline preview.
- `src-tauri/src/`:
  - `lib.rs`: tray, window lifecycle (the window is destroyed on close; the app lives in the tray), commands, polling loop
  - `watcher.rs`: pure game state machine
  - `league.rs`: API client
  - `schedule.rs`: the real scheduler
  - `settings.rs`: JSON settings; built-in sounds are embedded
  - `audio.rs`: symphonia decode + rodio playback
  - `updates.rs`: self-updater
  - `logger.rs`
- `tests/schedule-vectors.json`: shared cases that BOTH schedulers must pass. If you change scheduling rules, update the vectors and both `schedule.rs` and `src/schedule.js`.

## Commands
```bash
npm test                 # JS vectors + all Rust tests (must pass before any release)
npm run dev              # run locally
npm run mock-game -- --start 170 --speed 10   # fake League match on :2999
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
- Pings follow the in-game clock. No backlog when joining mid-game; a ping more than 10 s late is skipped; quiet after GameEnd; "Summoner's Rift only" means mapNumber 11.
- Never auto-install updates; installing only happens when the user clicks.
- A bad audio file must never crash the app (decoders are wrapped in catch_unwind).
