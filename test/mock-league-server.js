'use strict';

/**
 * Pretends to be League on this PC so you can test without queueing.
 *
 * Game only (the in-game API on https://127.0.0.1:2999):
 *   npm run mock-game                          # match already running, starts at 2:50
 *   npm run mock-game -- --start 590 --speed 10
 *   npm run mock-game -- --mode ARAM           # check "Summoner's Rift only"
 *   npm run mock-game -- --end-after 120       # send GameEnd after 120 real seconds
 *   npm run mock-game -- --spectate            # a replay / spectated game (no active player)
 *
 * Client + game (also fakes the League client: lobby -> queue -> champ select -> game):
 *   npm run mock-game -- --client --league-dir C:\temp\fake-league [--position top] [--swap]
 *   --swap: at 13 s into champ select (after bans lock) you swap pick order with cell 4,
 *   whose player banned Zed; for 1.5 s the client reports cell 4's role as yours
 *   then start the app with MINIMAP_NUDGE_LEAGUE_DIR=C:\temp\fake-league
 *   (the real client must be closed; it uses the same lockfile idea)
 *
 * Ctrl+C = everything closed.
 */

const https = require('https');
const fs = require('fs');
const os = require('os');
const path = require('path');

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 ? process.argv[i + 1] : def;
};
const flag = (name) => process.argv.includes(`--${name}`);

const CLIENT = flag('client');
const START = Number(arg('start', CLIENT ? 0 : 170));
const SPEED = Number(arg('speed', 1));
const MODE = arg('mode', 'CLASSIC');
const END_AFTER = Number(arg('end-after', 0));
const PORT = Number(arg('port', 2999));
const LEAGUE_DIR = arg('league-dir', path.join(os.tmpdir(), 'mock-league'));
const BAN = Number(arg('ban', 107)); // what "you" ban if nothing is hovered when the ban locks (107 = Rengar)
const SPECTATE = flag('spectate'); // the game is a replay/spectated one (no active player)
const SWAP = flag('swap'); // swap pick order with a teammate after bans
const POSITION = arg('position', 'jungle'); // your assigned role in champ select (top/jungle/middle/bottom/utility, or "" for blind)
const MAPS = { CLASSIC: 11, ARAM: 12, CHERRY: 30 };
const QUEUES = { CLASSIC: 420, ARAM: 450, CHERRY: 1700 };

// Client timeline in real seconds (only with --client).
const TIMELINE = [
  { until: 8, phase: 'Lobby' },
  { until: 20, phase: 'Matchmaking' },
  { until: 24, phase: 'ReadyCheck' },
  { until: 40, phase: 'ChampSelect' },
  { until: 45, phase: 'GameStart' },
  { until: Infinity, phase: 'InProgress' },
];
const GAME_STARTS_AT = CLIENT ? 45 : 0; // real seconds after launch

// Throwaway self-signed cert for testing only (the real client also uses a self-signed cert).
const tls = {
  key: fs.readFileSync(path.join(__dirname, 'fixtures', 'mock-key.pem')),
  cert: fs.readFileSync(path.join(__dirname, 'fixtures', 'mock-cert.pem')),
};

const startedAt = Date.now();
const elapsed = () => (Date.now() - startedAt) / 1000;
const gameRunning = () => elapsed() >= GAME_STARTS_AT;
const gameTime = () => START + Math.max(0, elapsed() - GAME_STARTS_AT) * SPEED;
const ended = () => END_AFTER > 0 && elapsed() - GAME_STARTS_AT >= END_AFTER;
const clientPhase = () => {
  if (ended()) return 'EndOfGame';
  return TIMELINE.find((t) => elapsed() < t.until).phase;
};
const phaseStart = () => {
  let prev = 0;
  for (const t of TIMELINE) { if (elapsed() < t.until) return prev; prev = t.until; }
  return prev;
};

const json = (res, code, body) => {
  res.statusCode = code;
  res.setHeader('Content-Type', 'application/json');
  res.end(JSON.stringify(body));
};

// ---- in-game API (port 2999)
const game = https.createServer(tls, (req, res) => {
  if (!gameRunning()) return json(res, 404, {});
  if (req.url === '/liveclientdata/gamestats') {
    return json(res, 200, { gameMode: MODE, gameTime: gameTime(), mapName: 'Map11', mapNumber: MAPS[MODE] ?? 11, mapTerrain: 'Default' });
  }
  if (req.url === '/liveclientdata/activeplayername') {
    return SPECTATE
      ? json(res, 404, { errorCode: 'RESOURCE_NOT_FOUND', httpStatus: 404, message: "Spectator mode doesn't currently support this feature" })
      : json(res, 200, 'Thomas#EUW');
  }
  if (req.url === '/liveclientdata/eventdata') {
    const Events = [{ EventID: 0, EventName: 'GameStart', EventTime: 0.05 }];
    if (ended()) Events.push({ EventID: 1, EventName: 'GameEnd', EventTime: gameTime(), Result: 'Win' });
    return json(res, 200, { Events });
  }
  json(res, 404, {});
});
game.listen(PORT, '127.0.0.1');

// ---- League client API (random port + lockfile), only with --client
if (CLIENT) {
  const password = 'mock-' + Math.random().toString(36).slice(2, 10);
  const auth = 'Basic ' + Buffer.from(`riot:${password}`).toString('base64');
  let accepted = false;
  let hovered = 0; // champion hovered on your ban action
  // Like real draft: 0-4 s PLANNING (the ban action is already "in progress" and the client
  // accepts a hover, but clears it when bans open), then BAN_PICK. Your ban locks at 12 s
  // with whatever is hovered (or --ban).
  const csTime = () => clientPhase() === 'ChampSelect' ? elapsed() - phaseStart() : (clientPhase() === 'Lobby' || clientPhase() === 'Matchmaking' || clientPhase() === 'ReadyCheck' ? -1 : 99);
  const PLANNING_END = 4;
  let bansOpened = false;
  const banState = () => {
    const t = csTime();
    if (t < 0) bansOpened = false;
    if (t >= PLANNING_END && !bansOpened) {
      bansOpened = true;
      if (hovered) console.log(`\n  >> bans open: client cleared the hover (${hovered}) made during planning`);
      hovered = 0;
    }
    const done = t >= 12;
    return { id: 1, actorCellId: 2, type: 'ban', isInProgress: t >= 0 && !done, completed: done, championId: done ? (hovered || BAN) : hovered };
  };
  const client = https.createServer(tls, (req, res) => {
    if (req.headers.authorization !== auth) return json(res, 401, { message: 'unauthorized' });
    const phase = clientPhase();
    if (req.method === 'PATCH' && req.url === '/lol-champ-select/v1/session/actions/1') {
      let body = '';
      req.on('data', (c) => (body += c));
      req.on('end', () => {
        try { hovered = JSON.parse(body).championId || 0; } catch {}
        console.log(`\n  >> app hovered champion ${hovered} for your ban`);
        res.statusCode = 204;
        res.end();
      });
      return;
    }
    if (req.method === 'POST' && req.url === '/lol-matchmaking/v1/ready-check/accept') {
      if (phase === 'ReadyCheck') { accepted = true; console.log('\n  >> ready check ACCEPTED by the app'); }
      res.statusCode = 204;
      return res.end();
    }
    const queue = { id: QUEUES[MODE] ?? 420, mapId: MAPS[MODE] ?? 11, description: 'mock queue' };
    switch (req.url) {
      case '/lol-gameflow/v1/gameflow-phase': return json(res, 200, phase);
      case '/lol-gameflow/v1/session': return json(res, 200, { phase, gameData: { isCustomGame: false, queue }, map: { id: queue.mapId } });
      case '/lol-lobby/v2/lobby': return json(res, 200, { members: [{}, {}], gameConfig: { queueId: queue.id, mapId: queue.mapId } });
      case '/lol-matchmaking/v1/ready-check':
        return json(res, 200, phase === 'ReadyCheck'
          ? { state: 'InProgress', playerResponse: accepted ? 'Accepted' : 'None', timer: elapsed() - phaseStart() }
          : { state: 'Invalid', playerResponse: 'None', timer: 0 });
      case '/lol-matchmaking/v1/search': return json(res, 200, { searchState: 'Searching', timeInQueue: elapsed() - phaseStart(), estimatedQueueTime: 95 });
      case '/lol-champ-select/v1/session': {
        const left = Math.max(0, 30 - (elapsed() - phaseStart()) * 2) * 1000;
        const t = csTime();
        const done = t >= 12;
        const teammateBan = { id: 2, actorCellId: 4, type: 'ban', isInProgress: t >= 0 && !done, completed: done, championId: 238 };
        const actions = [[banState(), teammateBan]];
        const swapped = SWAP && t >= 13;
        if (SWAP && t >= 13 && t < 13.5) console.log('\n  >> you swapped pick order with cell 4 (their ban was Zed)');
        // right after the swap the roles still follow the cells for a moment
        const rolesMoved = swapped && t >= 14.5;
        return json(res, 200, {
          timer: { adjustedTimeLeftInPhase: left, phase: csTime() < PLANNING_END ? 'PLANNING' : 'BAN_PICK', isInfinite: false },
          localPlayerCellId: swapped ? 4 : 2,
          actions,
          myTeam: [{ cellId: 0, assignedPosition: 'top' }, { cellId: 2, assignedPosition: rolesMoved ? 'utility' : POSITION }, { cellId: 4, assignedPosition: rolesMoved ? POSITION : 'utility' }],
        });
      }
      case '/lol-game-data/assets/v1/champion-summary.json':
        return json(res, 200, [{ id: -1, name: 'None' }, { id: 64, name: 'Lee Sin' }, { id: 107, name: 'Rengar' }, { id: 121, name: "Kha'Zix" }]);
      default: return json(res, 404, {});
    }
  });
  client.listen(0, '127.0.0.1', () => {
    fs.mkdirSync(LEAGUE_DIR, { recursive: true });
    const lock = path.join(LEAGUE_DIR, 'lockfile');
    fs.writeFileSync(lock, `LeagueClient:${process.pid}:${client.address().port}:${password}:https`);
    const cleanup = () => { try { fs.unlinkSync(lock); } catch {} process.exit(0); };
    process.on('SIGINT', cleanup);
    process.on('SIGTERM', cleanup);
    console.log(`Mock League client: lockfile at ${lock}`);
  });
}

console.log(`Mock ${MODE} ${CLIENT ? 'client + game (game starts after 45 s)' : 'match'} — game clock from ${START}s at ${SPEED}x. Ctrl+C to close.`);
setInterval(() => {
  const t = Math.floor(gameTime());
  const clock = gameRunning() ? `game ${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}` : 'no game yet';
  process.stdout.write(`\r  ${CLIENT ? clientPhase().padEnd(12) : ''} ${clock}${ended() ? '  (GameEnd sent)' : ''}      `);
}, 1000).unref();
