//! Hardware abstraction. The sequencer talks to real and simulated rigs
//! through this trait, so adding a new pump model is a new impl rather
//! than touching control logic.

pub mod demo;
pub mod real;

use crate::state::TurboState;
use anyhow::Result;

/// Names exposed at the hardware layer. The sequencer issues these by name;
/// the impl maps them onto whatever physical relay / parameter actually
/// drives them on this profile.
#[derive(Debug, Clone, Copy)]
pub enum Valve {
    /// Roughing (chamber → backing pump). HiPace 1200 profile = V1; HiPace 800 = backing valve.
    Roughing,
    /// Foreline (turbo → backing pump). HiPace 1200 = V2; HiPace 800 = backing-side accessory relay.
    Foreline,
    /// Vent (atmospheric/N2 in). HiPace 1200 = V3; HiPace 800 = vent valve.
    Vent,
    /// High-vacuum gate (chamber → turbo). HiPace 1200 doesn't have a separate gate — leave
    /// no-op. HiPace 800 = gate valve.
    Gate,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PressureReading {
    /// Pirani gauge in mbar, None if over/under-range or absent.
    pub pirani_mbar: Option<f64>,
    /// Ion gauge in mbar, None for Pirani-only profiles or when off.
    pub ion_mbar:    Option<f64>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TurboReading {
    pub speed_pct:  f64,
    pub current_a:  f64,
    pub temp_c:     Option<f64>,
    pub state:      TurboState,
    pub powered:    bool,
}

/// All control + sensing the sequencer needs. Impls must be Send + Sync so
/// the async sequencer task can hold &dyn Hardware across awaits.
#[async_trait::async_trait]
pub trait Hardware: Send + Sync {
    async fn read_pressures(&self) -> Result<PressureReading>;
    async fn read_turbo(&self)     -> Result<TurboReading>;

    async fn set_valve(&self,         valve: Valve, open: bool) -> Result<()>;
    async fn set_turbo_power(&self,   on: bool)                 -> Result<()>;
    async fn set_backing_pump(&self,  on: bool)                 -> Result<()>;

    /// True only when running against simulation. Let the UI label itself.
    #[allow(dead_code)]
    fn is_demo(&self) -> bool;
}
