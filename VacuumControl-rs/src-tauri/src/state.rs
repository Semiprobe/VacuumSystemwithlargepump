use serde::{Deserialize, Serialize};

/// Hardware identity of the turbo + backing pump combo. Picks the valve
/// topology and which gauge channels exist. App A = HiPace 1200 + Duo 35
/// (3 named valves V1/V2/V3, dual gauge: Ion + Pirani). App B = HiPace 800 +
/// Scroll (4 valves: gate / backing / vent / scroll-relay, Pirani only).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum PumpProfile {
    #[default]
    HiPace1200Duo,
    HiPace800Scroll,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum TurboState {
    #[default]
    Stopped,
    Accelerating,
    AtSpeed,
    Decelerating,
    Faulted,
}

/// State machine phase for the automated sequences. Mirrors App A's enum
/// (the more thorough one); App B's bool flags map cleanly into this.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SeqState {
    #[default]
    Idle,
    PumpDownInit,
    PumpDownRoughing,
    PumpDownCrossover,
    PumpDownTurbo,
    AtVacuum,
    VentInit,
    VentTurboSpindown,
    VentOpening,
    VentComplete,
    Aborted,
}

/// Live snapshot of the system. Pushed to the UI on every poll tick.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SystemState {
    /// Pirani gauge reading in mbar. None when over/under-range or no gauge.
    pub pirani_mbar:    Option<f64>,
    /// Ion gauge reading in mbar. None for Pirani-only profiles or when off.
    pub ion_mbar:       Option<f64>,

    /// Turbo nominal-speed percentage [0, 100].
    pub turbo_speed_pct: f64,
    /// Turbo drive current in amperes.
    pub turbo_current_a: f64,
    /// Bearing temperature in °C. App B watches for >85 °C → emergency stop.
    pub turbo_temp_c:    Option<f64>,
    pub turbo_state:     TurboState,
    pub turbo_powered:   bool,

    pub backing_pump_on: bool,

    /// Valve open/closed flags. Keyed by their well-known names. The HiPace
    /// 1200 profile uses V1/V2/V3; the HiPace 800 profile uses
    /// gate/backing/vent. Frontend reads whichever it expects.
    pub v1_roughing: bool,
    pub v2_foreline: bool,
    pub v3_vent:     bool,
    pub gate_valve:  bool,
    pub backing_valve: bool,

    pub seq_state:    SeqState,
    /// 0..=1.0; UI shows as percent. Empty / NaN means "no progress
    /// estimate" (e.g. waiting on a pressure threshold with unknown ETA).
    pub seq_progress: f64,
    /// Human-readable single-line status for the seq label.
    pub seq_message:  String,

    pub connected:    bool,
    pub demo_mode:    bool,
    pub last_error:   Option<String>,
}
