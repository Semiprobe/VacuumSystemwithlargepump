//! VacuumControl Tauri backend.
//!
//! Modules:
//!   * `state`     — `SystemState`, `SeqState`, `PumpProfile`, etc.
//!   * `settings`  — persisted config (COM ports, thresholds, log dir).
//!   * `hardware`  — `Hardware` trait + `demo` and `real` implementations.
//!   * `sequencer` — pump-down / vent state machine running in a tokio task.
//!   * `csv_log`   — per-session CSV logger.
//!   * `commands`  — Tauri command surface (start/stop sequence, set valves,
//!                   read state, save/load settings).
//!
//! The polling loop ticks once per second, reads pressures + turbo state
//! from the active `Hardware`, advances the `Sequencer` if running, writes
//! a CSV row if logging, and emits a `state-update` event.

mod commands;
mod csv_log;
mod hardware;
mod sequencer;
mod settings;
mod state;

use commands::AppState;
use csv_log::CsvLogger;
use hardware::demo::DemoHardware;
use sequencer::Sequencer;
use settings::Settings;
use state::{SystemState, TurboState};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager};
use tokio::time::{sleep, Duration};

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings = Settings::load();
            let system   = SystemState {
                connected: true, // demo is always "connected"
                demo_mode: settings.demo_mode,
                ..SystemState::default()
            };
            let hw: Arc<dyn hardware::Hardware> = Arc::new(DemoHardware::new());
            let seq = Sequencer::new();
            let csv = Arc::new(CsvLogger::new());
            // Best-effort open of the CSV session at startup. Empty dir => no-op.
            let _ = csv.open_session(&settings.csv_log_dir);

            let app_state = AppState {
                settings: Mutex::new(settings),
                system:   Mutex::new(system),
                hw:       Mutex::new(hw),
                seq:      seq.clone(),
                csv:      csv.clone(),
            };
            app.manage(app_state);

            // Background poll: every 250 ms, ask the hardware for fresh
            // readings, fold them into SystemState, and emit to the UI.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                run_poll_loop(handle).await;
            });

            // Sequencer publishes a watch::Sender. Forward each new value
            // into a tauri event so the UI's progress bar / phase label
            // update without polling.
            let handle2 = app.handle().clone();
            let mut rx  = seq.updates.subscribe();
            tauri::async_runtime::spawn(async move {
                while rx.changed().await.is_ok() {
                    let upd = rx.borrow().clone();
                    let _ = handle2.emit("seq-update", &upd_payload(&upd));
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::list_serial_ports,
            commands::get_system,
            commands::set_valve,
            commands::set_turbo_power,
            commands::set_backing_pump,
            commands::start_pump_down,
            commands::start_vent,
            commands::abort_sequence,
        ])
        .run(tauri::generate_context!())
        .expect("Vacuum Control failed to start");
}

/// Serializable shape of the sequencer update — `Debug, Clone` aren't enough
/// to send through tauri's emit, so flatten to a plain JSON-friendly struct.
fn upd_payload(u: &sequencer::SeqUpdate) -> serde_json::Value {
    serde_json::json!({
        "state":    format!("{:?}", u.state),
        "progress": u.progress,
        "message":  u.message,
    })
}

async fn run_poll_loop(app: tauri::AppHandle) {
    loop {
        // Pull current refs without holding the AppState lock across await.
        let (hw, demo_flag) = {
            let st = app.state::<AppState>();
            let hw = st.hw.lock().unwrap().clone();
            let demo = st.settings.lock().unwrap().demo_mode;
            (hw, demo)
        };

        let pressures = hw.read_pressures().await.ok();
        let turbo     = hw.read_turbo().await.ok();

        // Fold into SystemState. We don't try to track valve state from the
        // hardware here — the UI mirrors what the user / sequencer commanded.
        // Real-hardware impls can override later if their controllers report
        // valve states back.
        {
            let st = app.state::<AppState>();
            let mut s = st.system.lock().unwrap();
            if let Some(p) = pressures {
                s.pirani_mbar = p.pirani_mbar;
                s.ion_mbar    = p.ion_mbar;
            }
            if let Some(t) = turbo {
                s.turbo_speed_pct = t.speed_pct;
                s.turbo_current_a = t.current_a;
                s.turbo_temp_c    = t.temp_c;
                s.turbo_state     = t.state;
                s.turbo_powered   = t.powered;
            } else {
                s.turbo_state = TurboState::Faulted;
            }
            s.demo_mode = demo_flag;
            // Push the row to the CSV log (no-op when no session is open).
            let st = app.state::<AppState>();
            st.csv.log_row(&s);
            let _ = app.emit("system-update", &*s);
        }

        sleep(Duration::from_millis(250)).await;
    }
}
