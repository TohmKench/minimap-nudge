// Keeps the same window.nudge API the settings page used under Electron,
// implemented on Tauri's invoke/listen.
(function () {
  'use strict';
  const { invoke } = window.__TAURI__.core;
  const { listen } = window.__TAURI__.event;
  window.nudge = {
    getState: () => invoke('get_state'),
    update: (patch) => invoke('update_settings', { patch }),
    addSound: () => invoke('add_sound'),
    removeSound: (id) => invoke('remove_sound', { id }),
    preview: (id) => invoke('preview', { id: id ?? null }),
    onStatus: (fn) => listen('status', (e) => fn(e.payload)),
    onSettings: (fn) => listen('settings', (e) => fn(e.payload)),
    onPinged: (fn) => listen('pinged', () => fn()),
    checkUpdate: () => invoke('check_update'),
    setHotkeyCapture: (on) => invoke('set_hotkey_capture', { on }),
    installUpdate: () => invoke('install_update'),
    onUpdate: (fn) => listen('update', (e) => fn(e.payload)),
  };
})();
