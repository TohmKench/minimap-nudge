(function () {
'use strict';

/**
 * Reminder schedule.
 *
 * A schedule is a list of phases. Each phase starts at a game minute and has
 * an interval in seconds (or null = silent, no reminders during that phase).
 *
 *   [{ startMin: 0, intervalSec: 180 }, { startMin: 10, intervalSec: 60 }]
 *   -> pings at 3:00, 6:00, 9:00, 10:00, 11:00, 12:00 ...
 *
 * Rules:
 *  - No ping at 0:00.
 *  - A new phase begins with a ping at its start time, so the new rhythm
 *    kicks in immediately instead of waiting out the old interval.
 *  - Pings closer than MIN_GAP_SEC (15 s) to the previous one are dropped, so a
 *    phase starting just after a ping doesn't double-beep.
 */

const MIN_INTERVAL_SEC = 15;
const MIN_GAP_SEC = 15; // never above MIN_INTERVAL_SEC, or short intervals get thinned out
const MAX_GAME_SEC = 4 * 60 * 60; // safety bound for the search loop

function normalizePhases(phases) {
  const list = (Array.isArray(phases) ? phases : [])
    .map((p) => ({
      startMin: Math.max(0, Number(p && p.startMin) || 0),
      intervalSec:
        p && (p.intervalSec === null || p.intervalSec === 0 || p.silent)
          ? null
          : Math.max(MIN_INTERVAL_SEC, Math.round(Number(p && p.intervalSec) || 180)),
    }))
    .sort((a, b) => a.startMin - b.startMin);

  // Collapse duplicate start times (last one wins) and guarantee a phase at 0.
  const byStart = new Map();
  for (const p of list) byStart.set(p.startMin, p);
  const out = [...byStart.values()].sort((a, b) => a.startMin - b.startMin);
  if (out.length === 0 || out[0].startMin !== 0) {
    out.unshift({ startMin: 0, intervalSec: out.length ? out[0].intervalSec : 180 });
  }
  return out;
}

function phaseIndexAt(phases, t) {
  let idx = 0;
  for (let i = 0; i < phases.length; i++) {
    if (phases[i].startMin * 60 <= t) idx = i;
    else break;
  }
  return idx;
}

/** Yields ping times (seconds of game time) in ascending order. */
function* alertTimes(rawPhases, untilSec = MAX_GAME_SEC) {
  const phases = normalizePhases(rawPhases);
  let cursor = 0; // last scheduled point (emitted or not)
  let lastEmitted = -Infinity;

  while (cursor < untilSec) {
    const idx = phaseIndexAt(phases, cursor);
    const interval = phases[idx].intervalSec;
    const nextPhase = phases[idx + 1];
    const nextStart = nextPhase ? nextPhase.startMin * 60 : Infinity;

    let candidate = interval === null ? Infinity : cursor + interval;
    if (nextStart <= candidate) candidate = nextStart;
    if (!Number.isFinite(candidate)) return; // silent until the end
    if (candidate > untilSec) return;

    const silentHere = phases[phaseIndexAt(phases, candidate)].intervalSec === null;
    if (!silentHere && candidate > 0 && candidate - lastEmitted >= MIN_GAP_SEC) {
      lastEmitted = candidate;
      yield candidate;
    }
    cursor = candidate;
  }
}

/** First ping strictly after game time t (seconds), or null if none. */
function nextAlertAfter(phases, t) {
  for (const a of alertTimes(phases)) {
    if (a > t + 1e-6) return a;
  }
  return null;
}

function formatClock(sec) {
  if (sec === null || sec === undefined || !Number.isFinite(sec)) return '--:--';
  const s = Math.max(0, Math.floor(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
}

const api = {
  normalizePhases,
  alertTimes,
  nextAlertAfter,
  formatClock,
  MIN_INTERVAL_SEC,
  MIN_GAP_SEC,
};

// Shared by the main process (CommonJS) and the settings page (<script> tag).
if (typeof module !== 'undefined' && module.exports) module.exports = api;
else globalThis.Schedule = api;
})();
