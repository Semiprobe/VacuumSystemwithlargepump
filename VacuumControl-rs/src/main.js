import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

// ── DOM scaffold ────────────────────────────────────────────────────────────
const root = document.getElementById('app');
root.className = 'app';
root.innerHTML = `
  <!-- Status panel (left) -->
  <aside class="panel" id="status-panel">
    <h2>System Status</h2>

    <div class="metric">
      <span class="metric__label">Pirani</span>
      <span class="metric__value metric__value--big" id="pirani-val">— mbar</span>
    </div>
    <div class="metric">
      <span class="metric__label">Ion Gauge</span>
      <span class="metric__value metric__value--big" id="ion-val">— mbar</span>
    </div>

    <h3>Turbo</h3>
    <div class="metric">
      <span class="metric__label">Speed</span>
      <span class="metric__value" id="turbo-speed">— %</span>
    </div>
    <div class="metric">
      <span class="metric__label">Current</span>
      <span class="metric__value" id="turbo-current">— A</span>
    </div>
    <div class="metric">
      <span class="metric__label">Temp</span>
      <span class="metric__value" id="turbo-temp">— °C</span>
    </div>
    <div class="metric">
      <span class="metric__label">State</span>
      <span class="metric__value metric__value--small" id="turbo-state">Stopped</span>
    </div>

    <h3>Components</h3>
    <div class="indicator"><span class="indicator__dot" id="dot-turbo"></span><span>Turbo power</span></div>
    <div class="indicator"><span class="indicator__dot" id="dot-backing"></span><span>Backing pump</span></div>
    <div class="indicator"><span class="indicator__dot" id="dot-v1"></span><span>V1 — roughing</span></div>
    <div class="indicator"><span class="indicator__dot" id="dot-v2"></span><span>V2 — foreline</span></div>
    <div class="indicator"><span class="indicator__dot" id="dot-v3"></span><span>V3 — vent</span></div>
    <div class="indicator" id="row-gate" style="display:none">
      <span class="indicator__dot" id="dot-gate"></span><span>Gate valve</span>
    </div>

    <h3>Mode</h3>
    <div class="indicator"><span class="indicator__dot" id="dot-demo"></span><span id="mode-label">Demo</span></div>
  </aside>

  <!-- Controls panel (middle) -->
  <main class="panel">
    <h2>Automated Sequences</h2>

    <div class="row">
      <button class="btn btn--primary btn--full" id="btn-pumpdown">▼ Pump Down</button>
      <button class="btn btn--warn btn--full"    id="btn-vent">▲ Vent</button>
    </div>
    <button class="btn btn--danger btn--full" id="btn-abort">■ Abort / Safe State</button>

    <div class="progress" id="progress-row">
      <div class="progress__bar" id="progress-bar" style="width:0%"></div>
      <div class="progress__text" id="progress-text">Idle — 0%</div>
    </div>
    <div class="seq-msg" id="seq-message">Idle</div>

    <h2 style="margin-top:12px">Manual Controls</h2>

    <h3>Pump</h3>
    <div class="row">
      <button class="btn btn--full" data-tgl="turbo">Turbo: OFF</button>
      <button class="btn btn--full" data-tgl="backing">Backing: OFF</button>
    </div>

    <h3>Valves</h3>
    <div class="row">
      <button class="btn btn--full" data-valve="V1">V1: CLOSED</button>
      <button class="btn btn--full" data-valve="V2">V2: CLOSED</button>
    </div>
    <div class="row">
      <button class="btn btn--full" data-valve="V3">V3: CLOSED</button>
      <button class="btn btn--full" data-valve="Gate" id="btn-gate" style="display:none">Gate: CLOSED</button>
    </div>

    <button class="btn btn--ghost btn--full" id="btn-settings" style="margin-top:auto">⚙ Settings…</button>
  </main>

  <!-- Log panel (right) -->
  <aside class="panel">
    <h2>Log</h2>
    <div class="log" id="log"></div>
    <button class="btn btn--ghost" id="btn-clear">Clear</button>
  </aside>
`;

// ── Element refs ────────────────────────────────────────────────────────────
const $ = (id) => document.getElementById(id);
const piraniVal   = $('pirani-val');
const ionVal      = $('ion-val');
const turboSpeed  = $('turbo-speed');
const turboCurrent= $('turbo-current');
const turboTemp   = $('turbo-temp');
const turboStateL = $('turbo-state');
const dots = {
  turbo:   $('dot-turbo'),
  backing: $('dot-backing'),
  v1: $('dot-v1'), v2: $('dot-v2'), v3: $('dot-v3'),
  gate: $('dot-gate'),
  demo: $('dot-demo'),
};
const modeLabel  = $('mode-label');
const progressBar = $('progress-bar');
const progressText= $('progress-text');
const seqMessage  = $('seq-message');
const logEl       = $('log');

// ── Local mirror of system state (so toggle buttons know what to flip) ─────
let systemState = {};
let manualValveState = { V1: false, V2: false, V3: false, Gate: false };
let manualPumpState  = { turbo: false, backing: false };

// ── Logging helper ──────────────────────────────────────────────────────────
function appendLog(line, level = '') {
  const el = document.createElement('div');
  el.className = 'log__line' + (level ? ` log__line--${level}` : '');
  const ts = new Date().toLocaleTimeString();
  el.textContent = `[${ts}] ${line}`;
  logEl.appendChild(el);
  logEl.scrollTop = logEl.scrollHeight;
  // Cap at ~500 lines so the DOM doesn't bloat over a long session.
  while (logEl.childElementCount > 500) logEl.removeChild(logEl.firstChild);
}

// ── Format helpers ──────────────────────────────────────────────────────────
function fmtMbar(v) {
  if (v == null || !isFinite(v)) return '— mbar';
  if (v >= 1.0)   return `${v.toFixed(2)} mbar`;
  if (v >= 1e-3)  return `${v.toExponential(2)} mbar`;
  return `${v.toExponential(2)} mbar`;
}

function setIndicator(el, state) {
  el.className = 'indicator__dot ' + ({
    on:  'indicator__dot--on',
    off: 'indicator__dot--off',
    warn:'indicator__dot--warn',
    err: 'indicator__dot--err',
  }[state] || 'indicator__dot--off');
}

// ── Subscribe to backend events ────────────────────────────────────────────
listen('system-update', (ev) => {
  systemState = ev.payload;
  piraniVal.textContent = fmtMbar(systemState.pirani_mbar);
  ionVal.textContent    = fmtMbar(systemState.ion_mbar);
  turboSpeed.textContent  = `${(systemState.turbo_speed_pct ?? 0).toFixed(1)} %`;
  turboCurrent.textContent= `${(systemState.turbo_current_a ?? 0).toFixed(2)} A`;
  turboTemp.textContent   = systemState.turbo_temp_c == null ? '— °C' : `${systemState.turbo_temp_c.toFixed(1)} °C`;
  turboStateL.textContent = systemState.turbo_state ?? 'Stopped';

  setIndicator(dots.turbo,   systemState.turbo_powered ? 'on' : 'off');
  setIndicator(dots.demo,    systemState.demo_mode     ? 'warn' : 'on');
  modeLabel.textContent = systemState.demo_mode ? 'Demo Mode' : 'Hardware';

  // Over-temp warning
  if (systemState.turbo_temp_c != null && systemState.turbo_temp_c > 85) {
    setIndicator(dots.turbo, 'err');
  }
});

// Track the last seq state we *logged* so the log only gets one line per
// phase transition, not one every 500 ms while waiting on a threshold.
// The progress bar + status message still update every tick.
let lastLoggedSeqState = null;
listen('seq-update', (ev) => {
  const u = ev.payload;
  const pct = Math.round((u.progress ?? 0) * 100);
  progressBar.style.width = `${pct}%`;
  progressText.textContent = `${u.state} — ${pct}%`;
  seqMessage.textContent = u.message || '';
  if (u.state !== lastLoggedSeqState) {
    appendLog(`[${u.state}] ${u.message}`, u.state === 'Aborted' ? 'warn' : 'info');
    lastLoggedSeqState = u.state;
  }
});

listen('seq-error', (ev) => {
  appendLog(`Error: ${ev.payload}`, 'error');
});

// ── Sequence buttons ───────────────────────────────────────────────────────
$('btn-pumpdown').addEventListener('click', async () => {
  appendLog('User: start pump down', 'info');
  try { await invoke('start_pump_down'); }
  catch (e) { appendLog(`start_pump_down failed: ${e}`, 'error'); }
});
$('btn-vent').addEventListener('click', async () => {
  appendLog('User: start vent', 'info');
  try { await invoke('start_vent'); }
  catch (e) { appendLog(`start_vent failed: ${e}`, 'error'); }
});
$('btn-abort').addEventListener('click', async () => {
  appendLog('User: abort → safe state', 'warn');
  try { await invoke('abort_sequence'); }
  catch (e) { appendLog(`abort_sequence failed: ${e}`, 'error'); }
});

// ── Manual valve / pump toggles ────────────────────────────────────────────
document.querySelectorAll('[data-valve]').forEach((btn) => {
  const name = btn.dataset.valve;
  btn.addEventListener('click', async () => {
    const newOpen = !manualValveState[name];
    try {
      await invoke('set_valve', { name, open: newOpen });
      manualValveState[name] = newOpen;
      btn.textContent = `${name}: ${newOpen ? 'OPEN' : 'CLOSED'}`;
      // Mirror into the indicator dot too.
      const dot = name === 'Gate' ? dots.gate : dots[name.toLowerCase()];
      if (dot) setIndicator(dot, newOpen ? 'on' : 'off');
      appendLog(`Valve ${name} → ${newOpen ? 'OPEN' : 'CLOSED'}`);
    } catch (e) { appendLog(`set_valve(${name}) failed: ${e}`, 'error'); }
  });
});
document.querySelectorAll('[data-tgl]').forEach((btn) => {
  const which = btn.dataset.tgl;
  btn.addEventListener('click', async () => {
    const cmd = which === 'turbo' ? 'set_turbo_power' : 'set_backing_pump';
    const newOn = !manualPumpState[which];
    try {
      await invoke(cmd, { on: newOn });
      manualPumpState[which] = newOn;
      btn.textContent = `${which === 'turbo' ? 'Turbo' : 'Backing'}: ${newOn ? 'ON' : 'OFF'}`;
      appendLog(`${which} → ${newOn ? 'ON' : 'OFF'}`);
    } catch (e) { appendLog(`${cmd} failed: ${e}`, 'error'); }
  });
});

// ── Settings dialog ─────────────────────────────────────────────────────────
$('btn-settings').addEventListener('click', openSettings);
async function openSettings() {
  let s;
  try { s = await invoke('get_settings'); }
  catch (e) { appendLog(`get_settings failed: ${e}`, 'error'); return; }

  const overlay = document.createElement('div');
  overlay.className = 'dialog-overlay';
  overlay.innerHTML = `
    <div class="dialog">
      <h2 style="margin:0;color:var(--amber);font-size:11px;letter-spacing:1px">SETTINGS</h2>
      <div class="field-grid">
        <div class="field">
          <label>Pump profile</label>
          <select id="s-profile">
            <option value="HiPace1200Duo">HiPace 1200 + Duo 35</option>
            <option value="HiPace800Scroll">HiPace 800 + Scroll</option>
          </select>
        </div>
        <div class="field">
          <label>Demo mode</label>
          <select id="s-demo">
            <option value="true">On (simulated)</option>
            <option value="false">Off (real hardware)</option>
          </select>
        </div>
      </div>

      <h3>Serial Ports</h3>
      <div class="field-grid">
        <div class="field"><label>TPG 366 port</label><input id="s-tpg-port"/></div>
        <div class="field"><label>TPG 366 baud</label><input id="s-tpg-baud" type="number"/></div>
        <div class="field"><label>TC 1200 port</label><input id="s-tc-port"/></div>
        <div class="field"><label>TC 1200 baud</label><input id="s-tc-baud" type="number"/></div>
        <div class="field"><label>TC 1200 address</label><input id="s-tc-addr" type="number" min="1" max="255"/></div>
      </div>

      <h3>Sequence Thresholds</h3>
      <div class="field-grid">
        <div class="field"><label>Crossover pressure (mbar)</label><input id="s-th-cross" type="number" step="0.001"/></div>
        <div class="field"><label>At-vacuum pressure (mbar)</label><input id="s-th-vac" type="number" step="0.0000001"/></div>
        <div class="field"><label>Turbo at-speed (%)</label><input id="s-th-spd" type="number" min="50" max="100"/></div>
        <div class="field"><label>Spindown threshold (%)</label><input id="s-th-down" type="number" min="0" max="50"/></div>
        <div class="field"><label>Vent complete (mbar)</label><input id="s-th-vent" type="number"/></div>
        <div class="field"><label>Turbo over-temp (°C)</label><input id="s-th-temp" type="number"/></div>
      </div>

      <h3>Timeouts (seconds, 0 = none)</h3>
      <div class="field-grid">
        <div class="field"><label>Roughing</label><input id="s-to-rough" type="number"/></div>
        <div class="field"><label>Turbo accel</label><input id="s-to-acc" type="number"/></div>
        <div class="field"><label>Spindown</label><input id="s-to-down" type="number"/></div>
        <div class="field"><label>Vent</label><input id="s-to-vent" type="number"/></div>
      </div>

      <h3>Logging</h3>
      <div class="field"><label>CSV log directory (blank = disabled)</label><input id="s-csv"/></div>

      <div class="dialog__footer">
        <button class="btn btn--ghost" id="s-cancel">Cancel</button>
        <button class="btn btn--primary" id="s-save">Save</button>
      </div>
    </div>
  `;
  document.body.appendChild(overlay);

  // Hydrate
  $('s-profile').value = s.pump_profile;
  $('s-demo').value    = String(s.demo_mode);
  $('s-tpg-port').value = s.tpg_port;
  $('s-tpg-baud').value = s.tpg_baud;
  $('s-tc-port').value  = s.tc_port;
  $('s-tc-baud').value  = s.tc_baud;
  $('s-tc-addr').value  = s.tc_address;
  $('s-th-cross').value = s.crossover_pressure_mbar;
  $('s-th-vac').value   = s.at_vacuum_pressure_mbar;
  $('s-th-spd').value   = s.turbo_at_speed_pct;
  $('s-th-down').value  = s.turbo_spindown_pct;
  $('s-th-vent').value  = s.vent_complete_mbar;
  $('s-th-temp').value  = s.turbo_overtemp_c;
  $('s-to-rough').value = s.roughing_timeout_s;
  $('s-to-acc').value   = s.turbo_accel_timeout_s;
  $('s-to-down').value  = s.spindown_timeout_s;
  $('s-to-vent').value  = s.vent_timeout_s;
  $('s-csv').value      = s.csv_log_dir;

  $('s-cancel').addEventListener('click', () => overlay.remove());
  $('s-save').addEventListener('click', async () => {
    const updated = {
      pump_profile:           $('s-profile').value,
      demo_mode:              $('s-demo').value === 'true',
      tpg_port:               $('s-tpg-port').value,
      tpg_baud:               +$('s-tpg-baud').value,
      tc_port:                $('s-tc-port').value,
      tc_baud:                +$('s-tc-baud').value,
      tc_address:             +$('s-tc-addr').value,
      crossover_pressure_mbar:+$('s-th-cross').value,
      at_vacuum_pressure_mbar:+$('s-th-vac').value,
      turbo_at_speed_pct:     +$('s-th-spd').value,
      turbo_spindown_pct:     +$('s-th-down').value,
      vent_complete_mbar:     +$('s-th-vent').value,
      turbo_overtemp_c:       +$('s-th-temp').value,
      roughing_timeout_s:     +$('s-to-rough').value,
      turbo_accel_timeout_s:  +$('s-to-acc').value,
      spindown_timeout_s:     +$('s-to-down').value,
      vent_timeout_s:         +$('s-to-vent').value,
      csv_log_dir:            $('s-csv').value,
    };
    try {
      await invoke('save_settings', { settings: updated });
      appendLog('Settings saved', 'info');
      overlay.remove();
      // Show / hide the gate valve UI based on profile.
      reflectProfile(updated.pump_profile);
    } catch (e) { appendLog(`save_settings failed: ${e}`, 'error'); }
  });
}

function reflectProfile(profile) {
  const showGate = profile === 'HiPace800Scroll';
  $('row-gate').style.display = showGate ? 'flex' : 'none';
  $('btn-gate').style.display = showGate ? '' : 'none';
}

$('btn-clear').addEventListener('click', () => {
  logEl.innerHTML = '';
});

// ── Init ───────────────────────────────────────────────────────────────────
(async () => {
  try {
    const s = await invoke('get_settings');
    reflectProfile(s.pump_profile);
    appendLog(`Initialised — ${s.demo_mode ? 'DEMO' : 'HARDWARE'} mode, profile ${s.pump_profile}`, 'info');
  } catch (e) {
    appendLog(`Init failed: ${e}`, 'error');
  }
})();
