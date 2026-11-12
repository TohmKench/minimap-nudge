#!/usr/bin/env node
// Bumps the version everywhere, writes RELEASE_NOTES.md, commits and pushes to
// main. GitHub Actions (release.yml) sees the new version, then builds, signs and
// publishes release vX.Y.Z (it creates the tag itself), and installed apps offer
// the update. No tag push needed.
//
//   npm run release -- 2.2.0 "Shorter pings
//   - New bell sound"
//   npm run release -- 2.2.0 "notes" --no-push     (commit only)
import { execSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const [version, notes, ...flags] = process.argv.slice(2);
const push = !flags.includes('--no-push');
const sh = (c) => execSync(c, { stdio: ['ignore', 'pipe', 'inherit'] }).toString().trim();
const fail = (m) => { console.error(`release: ${m}`); process.exit(1); };

if (!/^\d+\.\d+\.\d+$/.test(version || '')) fail('usage: npm run release -- <x.y.z> "<release notes>"');
if (!notes || !notes.trim()) fail('release notes are required (they show in the app\'s update banner)');

const conf = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const cmp = (a, b) => { const x = a.split('.').map(Number), y = b.split('.').map(Number); for (let i = 0; i < 3; i++) if (x[i] !== y[i]) return x[i] - y[i]; return 0; };
if (cmp(version, conf.version) <= 0) fail(`${version} must be higher than the current ${conf.version}`);
if (sh('git status --porcelain')) fail('working tree has uncommitted changes — commit or stash them first');
const branch = sh('git rev-parse --abbrev-ref HEAD');
if (push && branch !== 'main') fail(`releases are published from main (you're on ${branch})`);

const edit = (file, fn) => writeFileSync(file, fn(readFileSync(file, 'utf8')));
edit('src-tauri/tauri.conf.json', (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`));
edit('package.json', (s) => s.replace(/("version":\s*")[^"]+(")/, `$1${version}$2`));
edit('package-lock.json', (s) => {
  const j = JSON.parse(s);
  j.version = version;
  if (j.packages && j.packages['']) j.packages[''].version = version;
  return JSON.stringify(j, null, 2) + '\n';
});
edit('src-tauri/Cargo.toml', (s) => s.replace(/^version = "[^"]+"/m, `version = "${version}"`));
edit('src-tauri/Cargo.lock', (s) => s.replace(/(name = "minimap-nudge"\nversion = ")[^"]+(")/, `$1${version}$2`));

writeFileSync('RELEASE_NOTES.md', notes.trim() + '\n');
sh('git add RELEASE_NOTES.md src-tauri/tauri.conf.json package.json package-lock.json src-tauri/Cargo.toml src-tauri/Cargo.lock');
sh(`git commit -m "Release v${version}"`);
console.log(`Committed "Release v${version}".`);

if (push) {
  execSync('git push origin main', { stdio: 'inherit' });
  console.log(`Pushed. GitHub Actions is building v${version}; installed apps will offer it once the release is published (~10 min).`);
  console.log('Watch it: https://github.com/TohmKench/minimap-nudge/actions');
} else {
  console.log('Not pushed. Run: git push origin main');
}
