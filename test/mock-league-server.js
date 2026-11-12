'use strict';

/**
 * Pretends to be a running League match on https://127.0.0.1:2999 so you can
 * test the app without queueing up.
 *
 *   npm run mock-game                 # starts at 2:50 game time, real-time
 *   npm run mock-game -- --start 590 --speed 10
 *   npm run mock-game -- --mode ARAM  # check "Summoner's Rift only"
 *   npm run mock-game -- --end-after 120   # fire GameEnd after 120 real seconds
 *
 * Ctrl+C = game closed.
 */

const https = require('https');
const fs = require('fs');
const path = require('path');

const arg = (name, def) => {
  const i = process.argv.indexOf(`--${name}`);
  return i > -1 ? process.argv[i + 1] : def;
};
const START = Number(arg('start', 170));
const SPEED = Number(arg('speed', 1));
const MODE = arg('mode', 'CLASSIC');
const END_AFTER = Number(arg('end-after', 0));
const PORT = Number(arg('port', 2999));
const MAPS = { CLASSIC: 11, ARAM: 12, CHERRY: 30 };

// Throwaway self-signed cert for testing only (the real client also uses a self-signed cert).
const tls = {
  key: fs.readFileSync(path.join(__dirname, 'fixtures', 'mock-key.pem')),
  cert: fs.readFileSync(path.join(__dirname, 'fixtures', 'mock-cert.pem')),
};

const startedAt = Date.now();
const gameTime = () => START + ((Date.now() - startedAt) / 1000) * SPEED;
const ended = () => END_AFTER > 0 && (Date.now() - startedAt) / 1000 >= END_AFTER;

const server = https.createServer(tls, (req, res) => {
  res.setHeader('Content-Type', 'application/json');
  if (req.url === '/liveclientdata/gamestats') {
    return res.end(JSON.stringify({
      gameMode: MODE, gameTime: gameTime(), mapName: 'Map11', mapNumber: MAPS[MODE] ?? 11, mapTerrain: 'Default',
    }));
  }
  if (req.url === '/liveclientdata/eventdata') {
    const Events = [{ EventID: 0, EventName: 'GameStart', EventTime: 0.05 }];
    if (ended()) Events.push({ EventID: 1, EventName: 'GameEnd', EventTime: gameTime(), Result: 'Win' });
    return res.end(JSON.stringify({ Events }));
  }
  res.statusCode = 404;
  res.end('{}');
});

server.listen(PORT, '127.0.0.1', () => {
  console.log(`Mock ${MODE} match on https://127.0.0.1:${PORT} — starting at ${START}s, ${SPEED}x speed. Ctrl+C to "close the game".`);
});
setInterval(() => {
  const t = Math.floor(gameTime());
  process.stdout.write(`\r  game clock ${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}${ended() ? '  (GameEnd sent)' : ''}   `);
}, 1000).unref();

