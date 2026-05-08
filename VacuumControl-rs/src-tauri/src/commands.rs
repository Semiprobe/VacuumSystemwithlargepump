//! Tauri-exposed commands. Frontend calls these via `invoke('name', args)`.

use crate::csv_log::CsvLogger;
use crate::hardware::{demo::DemoHardware, real::RealHardware, Hardware, Valve};
use crate::sequencer::Sequencer;
use crate::settings::Settings;
use crate::state::SystemState;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

pub struct AppState {
    pub settings: Mutex<Settings>,
    pub system:   Mutex<SystemState>,
    /// Boxed because Hardware is dyn — the concrete type swaps when the
    /// user toggles demo mode or changes the pump profile.
    pub hw:       Mutex<Arc<dyn Hardware>>,
    pub seq:      Arc<Sequencer>,
    pub csv:      Arc<CsvLogger>,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(settings: Settings, state: State<'_, AppState>) -> Result<(), String> {
    settings.save();
    *state.settings.lock().unwrap() = settings.clone();
    // Swap the hardware impl whenever something that affects the connection
    // changed (demo flag, ports, profile). Easiest: always rebuild on save.
    let new_hw: Arc<dyn Hardware> = if settings.demo_mode {
        Arc::new(DemoHardware::new())
    } else {
        match RealHardware::open(&settings) {
            Ok(h) => Arc::new(h),
            Err(e) => {
                // Fall back to demo so the app stays usable, but surface the
                // error and force demo_mode true in saved settings so the
                // UI label is honest about what's actually running.
                let mut s = state.settings.lock().unwrap();
                s.demo_mode = true;
                s.save();
                state.system.lock().unwrap().last_error =
                    Some(format!("Could not open real hardware: {e}. Reverted to demo mode."));
                Arc::new(DemoHardware::new())
            }
        }
    };
    *state.hw.lock().unwrap() = new_hw;
    // Re-open the CSV session if the dir changed (new dir, or freshly enabled).
    if let Err(e) = state.csv.open_session(&settings.csv_log_dir) {
        state.system.lock().unwrap().last_error = Some(format!("CSV log open failed: {e}"));
    }
    Ok(())
}

#[tauri::command]
pub fn list_serial_ports() -> Vec<String> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.port_name)
        .collect()
}

#[tauri::command]
pub fn get_system(state: State<'_, AppState>) -> SystemState {
    state.system.lock().unwrap().clone()
}

#[tauri::command]
pub async fn set_valve(
    name: String,
    open: bool,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let valve = match name.as_str() {
        "Roughing" | "V1" => Valve::Roughing,
        "Foreline" | "V2" => Valve::Foreline,
        "Vent"     | "V3" => Valve::Vent,
        "Gate"            => Valve::Gate,
        other => return Err(format!("Unknown valve: {other}")),
    };
    let hw = state.hw.lock().unwrap().clone();
    hw.set_valve(valve, open).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_turbo_power(on: bool, state: State<'_, AppState>) -> Result<(), String> {
    let hw = state.hw.lock().unwrap().clone();
    hw.set_turbo_power(on).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_backing_pump(on: bool, state: State<'_, AppState>) -> Result<(), String> {
    let hw = state.hw.lock().unwrap().clone();
    hw.set_backing_pump(on).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_pump_down(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let seq = state.seq.clone();
    let hw  = state.hw.lock().unwrap().clone();
    let set = state.settings.lock().unwrap().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = seq.clone().pump_down(hw.clone(), set).await {
            // Failure → drive to safe state and surface the error.
            let _ = app.emit("seq-error", e.to_string());
            let _ = seq.abort_to_safe(hw).await;
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn start_vent(state: State<'_, AppState>, app: AppHandle) -> Result<(), String> {
    let seq = state.seq.clone();
    let hw  = state.hw.lock().unwrap().clone();
    let set = state.settings.lock().unwrap().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = seq.clone().vent(hw.clone(), set).await {
            let _ = app.emit("seq-error", e.to_string());
            let _ = seq.abort_to_safe(hw).await;
        }
    });
    Ok(())
}

#[tauri::command]
pub async fn abort_sequence(state: State<'_, AppState>) -> Result<(), String> {
    state.seq.abort();
    let hw  = state.hw.lock().unwrap().clone();
    let seq = state.seq.clone();
    tauri::async_runtime::spawn(async move {
        let _ = seq.abort_to_safe(hw).await;
    });
    Ok(())
}
