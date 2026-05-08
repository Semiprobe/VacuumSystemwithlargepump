//! Demo simulator. Pulls heavily from App A's physics:
//!   - Pressure approaches a target on a log scale (exponential decay
//!     in log10 space) so it reads naturally from atmospheric down to
//!     UHV without weird linear glitches near the floor.
//!   - Turbo speed ramps with a first-order lag toward a target set by
//!     whether power is on, foreline is open, etc.
//!
//! Real hardware impls live alongside (tpg366.rs, tc1200.rs); the
//! sequencer talks to whichever via the `Hardware` trait.

use super::{Hardware, PressureReading, TurboReading, Valve};
use crate::state::TurboState;
use anyhow::Result;
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Mutable internals tracked between ticks. Wrapped in a Mutex inside an Arc
/// so the polling task and the trait methods share one source of truth.
struct DemoInner {
    last_tick: Instant,
    /// log10(mbar). Atmosphere ≈ 3 (1000 mbar); UHV ≈ -10.
    log_pressure: f64,
    /// 0..=100. Backed by simple first-order lag toward `target_speed_pct`.
    speed_pct:    f64,
    /// Bearing temperature (°C). Heats while pumping, cools otherwise.
    temp_c:       f64,
    turbo_powered: bool,
    backing_on:   bool,
    v1_roughing:  bool,
    v2_foreline:  bool,
    v3_vent:      bool,
    gate_open:    bool,
}

impl Default for DemoInner {
    fn default() -> Self {
        Self {
            last_tick:     Instant::now(),
            log_pressure:  3.0, // atmosphere
            speed_pct:     0.0,
            temp_c:        25.0,
            turbo_powered: false,
            backing_on:    false,
            v1_roughing:   false,
            v2_foreline:   false,
            v3_vent:       false,
            gate_open:     false,
        }
    }
}

#[derive(Clone)]
pub struct DemoHardware {
    inner: Arc<Mutex<DemoInner>>,
}

impl DemoHardware {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(DemoInner::default())) }
    }

    /// Advance the simulation by however much wall time has passed since
    /// `last_tick`. Called inside every read so the read returns a current
    /// value without needing a separate background task.
    fn tick(&self) {
        let mut s = self.inner.lock().unwrap();
        let now = Instant::now();
        let dt = (now - s.last_tick).as_secs_f64().clamp(0.0, 1.0);
        s.last_tick = now;

        // Pressure model: pick a log-space target and approach it. Targets
        // mirror App A's tuning so timing in pump-down sequences feels real.
        //   - V3 vent open: race back up to atmosphere fast (tau ≈ 8 s).
        //   - V1 roughing + backing on: approach 5e-2 mbar (tau ≈ 60 s).
        //   - V2 foreline + turbo at speed: approach 1e-7 (tau ≈ 90 s).
        //   - Else: pressure drifts up slightly (real chambers leak / outgas).
        let target = if s.v3_vent {
            3.0
        } else if s.v2_foreline && s.speed_pct > 80.0 {
            -7.0
        } else if s.v1_roughing && s.backing_on {
            (5.0e-2_f64).log10()
        } else {
            // Slow drift up if isolated.
            (s.log_pressure + 0.02).min(3.0)
        };
        let tau = if s.v3_vent { 8.0 }
                  else if s.v2_foreline && s.speed_pct > 80.0 { 90.0 }
                  else if s.v1_roughing && s.backing_on { 60.0 }
                  else { 1200.0 };
        s.log_pressure += (target - s.log_pressure) * (dt / tau).min(1.0);

        // Turbo speed: needs power AND foreline support to spin up. Without
        // foreline (or if backing pump off) it can't reach speed.
        let speed_target = if s.turbo_powered && s.v2_foreline && s.backing_on {
            100.0
        } else if s.turbo_powered {
            // Powered without backing: stall around 30% — common real failure mode.
            30.0
        } else {
            0.0
        };
        let speed_tau = if s.turbo_powered { 25.0 } else { 18.0 };
        s.speed_pct += (speed_target - s.speed_pct) * (dt / speed_tau).min(1.0);
        s.speed_pct = s.speed_pct.clamp(0.0, 100.0);

        // Temperature: heats while spinning, cools otherwise. Asymptote at
        // 60 °C in normal running, plenty of margin to App B's 85 °C limit.
        let temp_target = 25.0 + (s.speed_pct / 100.0) * 35.0;
        s.temp_c += (temp_target - s.temp_c) * (dt / 90.0).min(1.0);
    }

    fn turbo_state_for(speed: f64, powered: bool) -> TurboState {
        match (powered, speed) {
            (false, s) if s < 1.0  => TurboState::Stopped,
            (false, _)             => TurboState::Decelerating,
            (true,  s) if s >= 95.0 => TurboState::AtSpeed,
            (true,  _)             => TurboState::Accelerating,
        }
    }
}

#[async_trait]
impl Hardware for DemoHardware {
    async fn read_pressures(&self) -> Result<PressureReading> {
        self.tick();
        let s = self.inner.lock().unwrap();
        let p = 10f64.powf(s.log_pressure);
        // Pirani spec: useful from ~1e-4 to 1000 mbar. Outside that range
        // a real gauge reports over/under-range — represent as None.
        let pirani = if (1.0e-4..=1100.0).contains(&p) { Some(p) } else { None };
        // Ion gauge spec: ~1e-10 to 1e-2 mbar, AND it's only on when foreline
        // is open and turbo at-speed (real interlock — no IG at high pressure).
        let ig_ok = s.v2_foreline && s.speed_pct > 80.0 && p < 1.0e-2;
        let ion = if ig_ok { Some(p) } else { None };
        Ok(PressureReading { pirani_mbar: pirani, ion_mbar: ion })
    }

    async fn read_turbo(&self) -> Result<TurboReading> {
        self.tick();
        let s = self.inner.lock().unwrap();
        Ok(TurboReading {
            speed_pct:  s.speed_pct,
            current_a:  if s.turbo_powered { 0.5 + s.speed_pct * 0.025 } else { 0.0 },
            temp_c:     Some(s.temp_c),
            state:      Self::turbo_state_for(s.speed_pct, s.turbo_powered),
            powered:    s.turbo_powered,
        })
    }

    async fn set_valve(&self, valve: Valve, open: bool) -> Result<()> {
        let mut s = self.inner.lock().unwrap();
        match valve {
            Valve::Roughing => s.v1_roughing = open,
            Valve::Foreline => s.v2_foreline = open,
            Valve::Vent     => s.v3_vent     = open,
            Valve::Gate     => s.gate_open   = open,
        }
        Ok(())
    }

    async fn set_turbo_power(&self, on: bool) -> Result<()> {
        self.inner.lock().unwrap().turbo_powered = on;
        Ok(())
    }

    async fn set_backing_pump(&self, on: bool) -> Result<()> {
        self.inner.lock().unwrap().backing_on = on;
        Ok(())
    }

    fn is_demo(&self) -> bool { true }
}
