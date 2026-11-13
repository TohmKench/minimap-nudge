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

// ------------------------------------------------------------ role schedules

let editRole = 'default'; // which schedule the editor shows

const roleName = (id) => (state.roles || []).find((r) => r.id === id)?.name || id;
const roleSchedules = () => state.settings.roleSchedules || {};
// What will actually run for a role (its own schedule, else the default).
const phasesForRole = (role) => (role && roleSchedules()[role]) || state.settings.phases;
// What the editor is editing: the default, or the selected role's own schedule (null = none yet).
const editedPhases = () => (editRole === 'default' ? state.settings.phases : roleSchedules()[editRole] || null);

function savePhases(phases) {
  if (editRole === 'default') return save({ phases });
  return save({ roleSchedules: { ...roleSchedules(), [editRole]: phases } });
}

function renderRoleTabs() {
  const active = state.status && state.status.role;
  const tabs = [{ id: 'default', name: 'Default' }, ...(state.roles || [])];
  $('roleTabs').replaceChildren(...tabs.map((t) =>
    el('button', {
      class: 'tab', role: 'tab', 'aria-selected': String(t.id === editRole),
      title: t.id === 'default' ? 'Used for any role without its own schedule' : `Schedule for ${t.name}`,
      onclick: () => { editRole = t.id; renderSchedule(); },
    },
    t.name,
    t.id !== 'default' && roleSchedules()[t.id] ? el('span', { class: 'dot', title: 'Has its own schedule' }) : null,
    (t.id === active || (t.id === 'default' && !(active && roleSchedules()[active]))) ? el('span', { class: 'live' }, 'in use') : null)));

  const note = $('roleNote');
  note.replaceChildren();
  if (editRole === 'default') {
    note.hidden = true;
  } else if (!roleSchedules()[editRole]) {
    note.hidden = false;
    note.append(
      el('span', {}, `${roleName(editRole)} uses your default schedule.`),
      el('button', { class: 'ghost', onclick: () => savePhases(state.settings.phases.map((p) => ({ ...p }))) },
        `Customise for ${roleName(editRole)}`));
  } else {
    note.hidden = false;
    note.append(
      el('span', {}, `${roleName(editRole)} has its own schedule.`),
      el('button', { class: 'ghost', onclick: () => {
        const rest = { ...roleSchedules() };
        delete rest[editRole];
        save({ roleSchedules: rest });
      } }, 'Use default instead'));
  }
}

function renderRoleMode() {
  const sel = $('roleMode');
  const opts = [{ id: 'auto', name: 'Auto – role from champ select' }, ...(state.roles || []).map((r) => ({ id: r.id, name: `Always ${r.name}` }))];
  sel.replaceChildren(...opts.map((o) => el('option', { value: o.id, selected: o.id === state.settings.roleMode }, o.name)));
}

function renderRoleActive(st) {
  const box = $('roleActive');
  box.replaceChildren();
  if (!st) return;
  if (st.role) {
    const how = st.roleSource === 'manual' ? 'set by you' : 'from champ select';
    box.append('Right now: ', el('b', {}, roleName(st.role)), ` (${how}) – `,
      st.roleSchedule ? 'using its own schedule.' : 'using the default schedule.');
  } else if (state.settings.roleMode === 'auto') {
    box.textContent = 'Right now: default schedule. Your role is picked up in champ select (blind pick and ARAM have none).';
  }
}

$('roleMode').addEventListener('change', (e) => save({ roleMode: e.target.value }));

function firstPing() {
  const phases = state && state.settings ? phasesForRole(state.status && state.status.role) : [];
  return formatClock(Schedule.nextAlertAfter(phases, 0));
}

function pingNote(st) {
  if (!st.willPing) return 'this mode stays quiet (Summoner\u2019s Rift only)';
  const which = st.role && st.roleSchedule ? `${roleName(st.role)} schedule, ` : '';
  return el('span', {}, `${which}first ping at `, el('span', { class: 'mono' }, firstPing()));
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
    case 'readyCheck': {
      const rc = st.readyCheck;
      if (rc && (rc.autoAccepted || rc.response === 'accepted')) add(rc.autoAccepted ? 'Accepted automatically \u2713' : 'Accepted \u2713');
      else if (rc && rc.response === 'declined') add('Declined');
      else if (rc && rc.auto && rc.acceptIn != null) add(`Auto-accepting in ${Math.ceil(rc.acceptIn)}s \u2013 decline in the client to skip`);
      else add('Accept it in the client');
      break;
    }
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

// Every number box holds something usable (not mid-edit like "" or "-").
function phaseFieldsValid() {
  return [...document.querySelectorAll('#phases input[type="number"]:not(:disabled)')].every((inp) => {
    const n = Number(inp.value);
    return inp.value.trim() !== '' && Number.isFinite(n) && (inp.classList.contains('start') ? n >= 0 : n > 0);
  });
}

let saveTimer = null;

// While typing / clicking the arrows: update the timeline + preview at once,
// save shortly after the last keystroke.
function onPhasesInput() {
  if (!phaseFieldsValid()) return;
  renderTimeline(Schedule.normalizePhases(readPhasesFromDom()));
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => savePhases(readPhasesFromDom()), 400);
}

function onPhasesChanged() {
  clearTimeout(saveTimer);
  if (!phaseFieldsValid()) return; // left empty: focusout restores the saved value
  savePhases(readPhasesFromDom());
}

function phaseRow(phase, i, total) {
  const silent = phase.intervalSec === null;
  const lastInterval = silent ? 180 : phase.intervalSec;

  const startCell =
    i === 0
      ? el('span', { class: 'lbl' }, 'From the start')
      : el('span', {},
          el('span', { class: 'lbl' }, 'From '),
          el('input', { type: 'number', class: 'start', min: '1', step: '1', value: String(phase.startMin), oninput: onPhasesInput, onchange: onPhasesChanged }),
          el('span', { class: 'lbl' }, ' min'));

  const every = el('span', { class: 'every' },
    el('span', { class: 'lbl' }, 'every '),
    el('input', {
      type: 'number', class: 'every-input', min: '0.25', step: '0.25',
      value: minutesText(lastInterval), disabled: silent, title: 'Minutes between pings (0.5 = 30 seconds)',
      oninput: onPhasesInput,
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
        savePhases(phases);
      } }, '✕');

  return el('div', { class: `phase${silent ? ' is-silent' : ''}` }, startCell, every, el('span'), silentBox, remove);
}

function renderSchedule() {
  renderRoleTabs();
  const box = $('phases');
  const own = editedPhases();
  // A role without its own schedule: no editor, timeline shows the default it'll use.
  box.hidden = own === null;
  $('addPhase').disabled = own === null;
  const phases = own || state.settings.phases;
  // Don't rebuild the rows under a field the user is typing in (the timeline
  // still updates); they're rebuilt when focus leaves the schedule.
  const typing = box.contains(document.activeElement) && document.activeElement.type === 'number';
  if (!typing && own) box.replaceChildren(...phases.map((p, i) => phaseRow(p, i, phases.length)));
  renderTimeline(phases);
}

// Leaving the schedule: re-sort rows and restore anything left empty.
$('phases').addEventListener('focusout', () => {
  setTimeout(() => {
    if (state && !$('phases').contains(document.activeElement)) renderSchedule();
  }, 0);
});

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
  savePhases(phases);
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

// ------------------------------------------------------------ match found / shortcut

const prettyKeys = (acc) => (acc ? acc.split('+').join(' + ') : 'None');

function renderAccept() {
  const s = state.settings;
  $('autoAccept').checked = s.autoAccept;
  if (document.activeElement !== $('acceptDelay')) $('acceptDelay').value = String(s.autoAcceptDelay);
  if (!recording) $('hotkeyBtn').textContent = prettyKeys(s.autoAcceptHotkey);
  $('hotkeyClear').hidden = !s.autoAcceptHotkey;
  const err = $('hotkeyError');
  err.hidden = !state.hotkeyError;
  err.textContent = state.hotkeyError || '';
}

$('autoAccept').addEventListener('change', (e) => save({ autoAccept: e.target.checked }));
$('acceptDelay').addEventListener('change', (e) => {
  const n = Number(e.target.value);
  if (Number.isFinite(n)) save({ autoAcceptDelay: Math.max(0, Math.min(10, Math.round(n))) });
});

// Recording a new shortcut: click the box, press the keys.
let recording = false;
const MODS = ['Control', 'Alt', 'Shift', 'Meta', 'AltGraph', 'OS'];

function keyName(code) {
  let m;
  if ((m = /^Key([A-Z])$/.exec(code))) return m[1];
  if ((m = /^Digit(\d)$/.exec(code))) return m[1];
  return code; // F1–F24, Numpad5, ArrowUp, Space, Backquote, …
}

async function stopRecording() {
  recording = false;
  $('hotkeyBtn').classList.remove('recording');
  window.removeEventListener('keydown', onRecordKey, true);
  state = await api.setHotkeyCapture(false);
  render();
}

async function onRecordKey(e) {
  e.preventDefault();
  e.stopPropagation();
  if (e.key === 'Escape') return stopRecording();
  if (MODS.includes(e.key)) return; // wait for the actual key
  const mods = [];
  if (e.ctrlKey) mods.push('Ctrl');
  if (e.altKey) mods.push('Alt');
  if (e.shiftKey) mods.push('Shift');
  if (e.metaKey) mods.push('Super');
  const isF = /^F\d{1,2}$/.test(e.code);
  if (!mods.length && !isF) {
    $('hotkeyBtn').textContent = 'Add Ctrl, Alt or Shift\u2026';
    return;
  }
  const acc = [...mods, keyName(e.code)].join('+');
  recording = false;
  $('hotkeyBtn').classList.remove('recording');
  window.removeEventListener('keydown', onRecordKey, true);
  await save({ autoAcceptHotkey: acc });
  state = await api.setHotkeyCapture(false);
  render();
}

$('hotkeyBtn').addEventListener('click', async () => {
  if (recording) return stopRecording();
  recording = true;
  $('hotkeyBtn').classList.add('recording');
  $('hotkeyBtn').textContent = 'Press keys\u2026 (Esc to cancel)';
  await api.setHotkeyCapture(true); // release the old shortcut while recording
  window.addEventListener('keydown', onRecordKey, true);
});
$('hotkeyClear').addEventListener('click', () => save({ autoAcceptHotkey: '' }));

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
  renderSchedule();
  renderRoleMode();
  renderAccept();
  renderSounds();
  if (state.status) renderStatus(state.status);
  renderRoleActive(state.status);
  renderUpdate(state.update);
}

let lastRoleKey = '';
api.onStatus((st) => {
  if (!state) return;
  state.status = st;
  renderStatus(st);
  const key = `${st.role}|${st.roleSource}|${st.roleSchedule}`;
  if (key !== lastRoleKey) {
    lastRoleKey = key;
    renderRoleActive(st);
    renderRoleTabs();
  }
});
api.onSettings((settings) => { if (state) { state.settings = settings; render(); } });

api.getState().then((s) => { state = s; render(); });

})();
