use crate::state::PumpProfile;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// User-visible, persisted config. Everything that *was* hardcoded in App B
/// (COM port, thresholds, log dir) lives here and is editable from the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub pump_profile: PumpProfile,
    pub demo_mode:    bool,

    /// Pfeiffer TPG 366 multi-gauge. RS232 ASCII mnemonic. Used by both
    /// profiles when configured (HiPace 800 setups can also have a TPG).
    pub tpg_port:    String,
    pub tpg_baud:    u32,

    /// Pfeiffer TC 1200 / equivalent turbo controller. Binary telegram.
    pub tc_port:     String,
    pub tc_baud:     u32,
    /// Address byte in the binary telegram (default 1 for single-controller
    /// setups, but configurable so multi-pump test rigs work too).
    pub tc_address:  u8,

    /// Sequence thresholds (App A's tunables, exposed in the UI).
    pub crossover_pressure_mbar:   f64, // V1→V2 handoff (default 0.1)
    pub at_vacuum_pressure_mbar:   f64, // pumpdown success (default 1e-5)
    pub turbo_at_speed_pct:        f64, // "AtSpeed" threshold (default 95)
    pub turbo_spindown_pct:        f64, // vent spindown threshold (default 10)
    pub vent_complete_mbar:        f64, // vent success (default 800)
    pub turbo_overtemp_c:          f64, // App B safety (default 85)

    /// Per-phase timeouts in seconds. 0 = no timeout.
    pub roughing_timeout_s:   u32,
    pub turbo_accel_timeout_s: u32,
    pub spindown_timeout_s:   u32,
    pub vent_timeout_s:       u32,

    /// CSV session log directory. Empty = disabled. Default mirrors App B.
    pub csv_log_dir: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pump_profile: PumpProfile::default(),
            demo_mode:    true, // safe default — won't drive real hardware

            tpg_port:    "COM3".into(),
            tpg_baud:    9600,
            tc_port:     "COM4".into(),
            tc_baud:     9600,
            tc_address:  1,

            crossover_pressure_mbar: 0.1,
            at_vacuum_pressure_mbar: 1.0e-5,
            turbo_at_speed_pct:      95.0,
            turbo_spindown_pct:      10.0,
            vent_complete_mbar:      800.0,
            turbo_overtemp_c:        85.0,

            roughing_timeout_s:    600,  // 10 min
            turbo_accel_timeout_s: 900,  // 15 min
            spindown_timeout_s:    1800, // 30 min
            vent_timeout_s:        300,  // 5 min

            csv_log_dir: String::new(),
        }
    }
}

fn settings_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("SemiProbe")
        .join("VacuumControl")
        .join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(s) = serde_json::from_str::<Self>(&data) {
                return s;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let path = settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }
}
