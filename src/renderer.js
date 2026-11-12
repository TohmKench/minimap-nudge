/* global Schedule */
(function () {
'use strict';

const { alertTimes, formatClock } = Schedule;
const api = window.nudge;
const $ = (id) => document.getElementById(id);

const TIMELINE_SEC = 40 * 60;
let state = null; // { settings, sounds, status }

// ------------------------------------------------------------ helpers

function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (k === 'class') node.className = v;
    else if (k === 'dataset') Object.assign(node.dataset, v);
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (k in node) node[k] = v;
    else node.setAttribute(k, v);
  }
  for (const c of children) if (c != null) node.append(c);
  return node;
}

const minutesText = (sec) => String(+(sec / 60).toFixed(2));

async function save(patch) {
  state = await api.update(patch);
  render();
}

// ------------------------------------------------------------ status

const STATUS_TITLES = {
  waiting: 'Waiting for League',
  active: 'In game — watching the clock',
  paused: 'Reminders paused',
  ineligible: 'In game — staying quiet',
  ended: 'Game over',
};

const CLIENT_TITLES = {
  none: 'League client open',
  other: 'League client open',
  lobby: 'In lobby',
  matchmaking: 'In queue',
  readyCheck: 'Match found!',
  champSelect: 'Champion select',
  gameStart: 'Loading into game',
  inProgress: 'Loading into game',
  reconnect: 'Reconnect available',
  postGame: 'Post-game',
};

function firstPing() {
  const phases = state && state.settings ? state.settings.phases : [];
  return formatClock(Schedule.nextAlertAfter(phases, 0));
}

function pingNote(st) {
  return st.willPing
    ? el('span', {}, 'first ping at ', el('span', { class: 'mono' }, firstPing()))
    : 'this mode stays quiet (Summoner\u2019s Rift only)';
}

function clientSub(st) {
  const c = st.client;
  const parts = [];
  const add = (x) => { if (x != null && x !== '') parts.push(x); };
  switch (c.phase) {
    case 'lobby':
      add(c.queue);
      if (c.partySize > 1) add(`party of ${c.partySize}`);
      if (!parts.length) add('Pick a queue and it\u2019ll follow along.');
      break;
    case 'matchmaking':
      add(c.queue);
      if (c.timeInQueue != null) {
        add(el('span', {}, el('span', { class: 'mono' }, formatClock(c.timeInQueue)),
          c.estQueueTime ? ` in queue (est. ${formatClock(c.estQueueTime)})` : ' in queue'));
      }
      break;
    case 'readyCheck':
      add('Accept it in the client');
      break;
    case 'champSelect':
      add(c.queue);
      if (c.champSelectStage) add(c.phaseTimeLeft != null ? `${c.champSelectStage} \u2013 ${formatClock(c.phaseTimeLeft)} left` : c.champSelectStage);
      add(pingNote(st));
      break;
    case 'gameStart':
    case 'inProgress':
      add(c.queue);
      add(pingNote(st));
      break;
    case 'reconnect':
      add('Reconnect from the client \u2013 pings pick up where the game is');
      break;
    case 'postGame':
      add('GG \u2013 waiting for your next game');
      break;
    default:
      add('Queue up and it\u2019ll follow along.');
  }
  const out = [];
  parts.forEach((p, i) => { if (i) out.push(' \u00b7 '); out.push(p); });
  return out;
}

function renderStatus(st) {
  const header = $('status');
  header.dataset.state = st.state;
  header.dataset.phase = st.client ? st.client.phase : '';
  $('statusTitle').textContent =
    st.state === 'client' ? CLIENT_TITLES[st.client.phase] || 'League client open' : STATUS_TITLES[st.state] || 'Minimap Nudge';
  const sub = $('statusSub');
  sub.replaceChildren();

  if (st.state === 'active') {
    const left = st.nextAlert != null ? st.nextAlert - st.gameTime : null;
    sub.append(
      'Game clock ', el('span', { class: 'mono' }, formatClock(st.gameTime)),
      st.nextAlert != null
        ? el('span', {}, ' · next ping at ', el('span', { class: 'mono' }, formatClock(st.nextAlert)),
            ` (in ${formatClock(left)})`)
        : ' · no more pings scheduled this game'
    );
  } else if (st.state === 'ineligible') {
    sub.textContent = `${st.gameMode} isn't Summoner's Rift. Untick the option below to get pings everywhere.`;
  } else if (st.state === 'client') {
    sub.append(...clientSub(st));
  } else if (st.state === 'paused') {
    sub.textContent = st.client ? `${st.line.replace(/^Alerts paused · /, '')} \u2013 flip the switch to turn reminders back on.` : 'Flip the switch to turn reminders back on.';
  } else if (st.state === 'ended') {
    sub.textContent = 'Waiting for the next match.';
  } else {
    sub.textContent = 'Runs in the tray. Open League and it\u2019ll show what you\u2019re doing here.';
  }
}

// ------------------------------------------------------------ schedule

function readPhasesFromDom() {
  return [...document.querySelectorAll('.phase')].map((row, i) => {
    const start = i === 0 ? 0 : Number(row.querySelector('.start').value);
    const silent = row.querySelector('.silent input').checked;
    const every = Number(row.querySelector('.every-input').value);
    return {
      startMin: Number.isFinite(start) ? Math.max(0, start) : 0,
      intervalSec: silent ? null : Math.round((Number.isFinite(every) && every > 0 ? every : 3) * 60),
    };
  });
}

function onPhasesChanged() {
  save({ phases: readPhasesFromDom() });
}

function phaseRow(phase, i, total) {
  const silent = phase.intervalSec === null;
  const lastInterval = silent ? 180 : phase.intervalSec;

  const startCell =
    i === 0
      ? el('span', { class: 'lbl' }, 'From the start')
      : el('span', {},
          el('span', { class: 'lbl' }, 'From '),
          el('input', { type: 'number', class: 'start', min: '1', step: '1', value: String(phase.startMin), onchange: onPhasesChanged }),
          el('span', { class: 'lbl' }, ' min'));

  const every = el('span', { class: 'every' },
    el('span', { class: 'lbl' }, 'every '),
    el('input', {
      type: 'number', class: 'every-input', min: '0.25', step: '0.25',
      value: minutesText(lastInterval), disabled: silent, title: 'Minutes between pings (0.5 = 30 seconds)',
      onchange: onPhasesChanged,
    }),
    el('span', { class: 'lbl' }, ' min'));

  const silentBox = el('label', { class: 'silent', title: 'No pings during this stretch' },
    el('input', { type: 'checkbox', checked: silent, onchange: onPhasesChanged }), 'Silent');

  const remove = i === 0 || total === 1
    ? el('span')
    : el('button', { class: 'icon', title: 'Remove', 'aria-label': 'Remove this change', onclick: () => {
        const phases = readPhasesFromDom();
        phases.splice(i, 1);
        save({ phases });
      } }, '✕');

  return el('div', { class: `phase${silent ? ' is-silent' : ''}` }, startCell, every, el('span'), silentBox, remove);
}

function renderSchedule(phases) {
  const box = $('phases');
  // Don't clobber a field the user is typing in.
  if (box.contains(document.activeElement) && document.activeElement.type === 'number') return;
  box.replaceChildren(...phases.map((p, i) => phaseRow(p, i, phases.length)));
  renderTimeline(phases);
}

function renderTimeline(phases) {
  const pct = (t) => `${(t / TIMELINE_SEC) * 100}%`;
  const line = $('timeline');
  const nodes = [];

  phases.forEach((p, i) => {
    const start = p.startMin * 60;
    const end = phases[i + 1] ? phases[i + 1].startMin * 60 : TIMELINE_SEC;
    if (p.intervalSec === null && start < TIMELINE_SEC) {
      nodes.push(el('div', { class: 'quiet', style: `left:${pct(start)};width:${pct(Math.min(end, TIMELINE_SEC) - start)}` }));
    }
    if (i > 0 && start < TIMELINE_SEC) nodes.push(el('div', { class: 'boundary', style: `left:${pct(start)}` }));
  });

  const times = [...alertTimes(phases, TIMELINE_SEC)];
  for (const t of times) nodes.push(el('div', { class: 'tick', style: `left:calc(${pct(t)} - 1px)`, title: formatClock(t) }));
  line.replaceChildren(...nodes);

  const preview = $('preview');
  preview.replaceChildren();
  if (times.length === 0) {
    preview.textContent = 'No pings in the first 40 minutes.';
  } else {
    const shown = times.slice(0, 9).map(formatClock).join(', ');
    preview.append('Pings at ', el('span', { class: 'mono' }, shown), times.length > 9 ? ' …' : '');
  }
}

$('addPhase').addEventListener('click', () => {
  const phases = readPhasesFromDom();
  const last = phases[phases.length - 1];
  phases.push({ startMin: last.startMin + 10, intervalSec: last.intervalSec ?? 60 });
  save({ phases });
});

// ------------------------------------------------------------ sounds

function renderSounds() {
  const { sounds, settings } = state;
  $('sounds').replaceChildren(
    ...sounds.map((s) =>
      el('label', { class: 'sound' },
        el('input', {
          type: 'radio', name: 'sound', value: s.id, checked: s.id === settings.soundId,
          onchange: () => { save({ soundId: s.id }); api.preview(s.id); },
        }),
        el('span', { class: 'name' }, s.name),
        s.builtin ? null : el('span', { class: 'tag' }, 'yours'),
        el('button', { class: 'ghost', title: 'Play', onclick: (e) => { e.preventDefault(); api.preview(s.id); } }, '▶'),
        s.builtin ? null : el('button', {
          class: 'icon', title: 'Delete this sound',
          onclick: async (e) => { e.preventDefault(); state = await api.removeSound(s.id); render(); },
        }, '✕'))
    )
  );
}

$('addSound').addEventListener('click', async () => {
  const res = await api.addSound();
  state = res;
  const err = $('soundError');
  err.hidden = !res.error;
  err.textContent = res.error || '';
  render();
  if (!res.error) api.preview();
});

const volume = $('volume');
volume.addEventListener('input', () => { $('volumeVal').textContent = `${volume.value}%`; });
volume.addEventListener('change', async () => {
  await save({ volume: Number(volume.value) / 100 });
  api.preview();
});

// ------------------------------------------------------------ options

$('enabled').addEventListener('change', (e) => save({ enabled: e.target.checked }));
$('srOnly').addEventListener('change', (e) => save({ summonersRiftOnly: e.target.checked }));
$('startup').addEventListener('change', (e) => save({ launchAtStartup: e.target.checked }));

// ------------------------------------------------------------ ping flash
// Sounds play in the Rust backend; the page just flashes the header.

api.onPinged(() => {
  const header = $('status');
  header.classList.remove('flash');
  void header.offsetWidth; // restart animation
  header.classList.add('flash');
});

// ------------------------------------------------------------ updates

function renderUpdate(u) {
  if (!u) return;
  $('update').dataset.phase = u.phase;
  $('updateTitle').textContent = `Minimap Nudge ${u.current}`;
  const checked = u.lastChecked ? ` · checked ${u.lastChecked}` : '';
  const sub = {
    idle: 'Checks GitHub for new versions automatically.',
    checking: 'Checking GitHub…',
    uptodate: `Up to date${checked}`,
    available: `Version ${u.version} is available`,
    downloading: `Downloading ${u.version}… ${u.progress != null ? Math.round(u.progress * 100) + '%' : ''}`,
    installing: 'Installing — the app will restart in a moment',
    error: u.error || 'Update check failed',
  }[u.phase];
  $('updateSub').textContent = sub;

  const busy = ['checking', 'downloading', 'installing'].includes(u.phase);
  $('installUpdate').hidden = !['available', 'downloading', 'installing'].includes(u.phase);
  $('installUpdate').disabled = busy;
  $('installUpdate').textContent = busy ? 'Updating…' : `Update to ${u.version}`;
  $('checkUpdate').disabled = busy;

  const bar = $('updateProgress');
  bar.hidden = !(u.phase === 'downloading' || u.phase === 'installing');
  bar.firstElementChild.style.width = `${Math.round((u.progress || 0) * 100)}%`;

  const notes = $('updateNotes');
  notes.hidden = !(u.phase === 'available' && u.notes);
  notes.textContent = u.notes || '';
}

$('checkUpdate').addEventListener('click', async () => renderUpdate(await api.checkUpdate()));
$('installUpdate').addEventListener('click', async () => {
  try {
    await api.installUpdate();
  } catch (e) {
    /* status event already shows the error */
  }
});
api.onUpdate((u) => { if (state) state.update = u; renderUpdate(u); });

// ------------------------------------------------------------ wiring

function render() {
  const s = state.settings;
  $('enabled').checked = s.enabled;
  $('srOnly').checked = s.summonersRiftOnly;
  $('startup').checked = s.launchAtStartup;
  volume.value = Math.round(s.volume * 100);
  $('volumeVal').textContent = `${volume.value}%`;
  renderSchedule(s.phases);
  renderSounds();
  if (state.status) renderStatus(state.status);
  renderUpdate(state.update);
}

api.onStatus((st) => { if (state) { state.status = st; renderStatus(st); } });
api.onSettings((settings) => { if (state) { state.settings = settings; render(); } });

api.getState().then((s) => { state = s; render(); });

})();
