//! Automated pump-down / vent state machine. Runs as a single tokio task
//! so the UI is never blocked. Every transition emits a state update so
//! the frontend can show progress and current phase.

use crate::hardware::{Hardware, Valve};
use crate::settings::Settings;
use crate::state::SeqState;
use anyhow::{anyhow, Result};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::watch;
use tokio::time::{sleep, Duration, Instant};

/// Snapshot the sequencer publishes after every transition / poll. The UI
/// thread subscribes to a watch channel of these and re-renders.
#[derive(Debug, Clone)]
pub struct SeqUpdate {
    pub state:    SeqState,
    pub progress: f64,
    pub message:  String,
}

impl Default for SeqUpdate {
    fn default() -> Self {
        Self { state: SeqState::Idle, progress: 0.0, message: "Idle".into() }
    }
}

pub struct Sequencer {
    /// Set true to ask the running sequence to abort at its next safe point.
    abort_flag: Arc<AtomicBool>,
    /// Latest state update (for UI subscriptions).
    pub updates: watch::Sender<SeqUpdate>,
}

impl Sequencer {
    pub fn new() -> Arc<Self> {
        let (tx, _rx) = watch::channel(SeqUpdate::default());
        Arc::new(Self {
            abort_flag: Arc::new(AtomicBool::new(false)),
            updates:    tx,
        })
    }

    pub fn abort(&self) {
        self.abort_flag.store(true, Ordering::SeqCst);
    }

    fn clear_abort(&self) { self.abort_flag.store(false, Ordering::SeqCst); }
    fn aborted(&self)     -> bool { self.abort_flag.load(Ordering::SeqCst) }

    fn publish(&self, state: SeqState, progress: f64, msg: &str) {
        let _ = self.updates.send(SeqUpdate {
            state, progress, message: msg.to_string(),
        });
    }

    // ── Public entry points ─────────────────────────────────────────────

    /// Drive a full pump-down. Phases:
    ///   1. Init: close all valves, backing pump on.
    ///   2. Roughing: open V1; wait for Pirani ≤ crossover threshold (timeout).
    ///   3. Crossover: close V1; pause; open V2.
    ///   4. Turbo: power turbo; wait for AtSpeed (timeout).
    ///   5. AtVacuum: poll until ion gauge ≤ at-vacuum threshold (no timeout).
    pub async fn pump_down(self: Arc<Self>, hw: Arc<dyn Hardware>, set: Settings) -> Result<()> {
        self.clear_abort();
        self.publish(SeqState::PumpDownInit, 0.05, "Initialising — closing valves, backing pump on");

        hw.set_valve(Valve::Roughing, false).await?;
        hw.set_valve(Valve::Foreline, false).await?;
        hw.set_valve(Valve::Vent,     false).await?;
        hw.set_turbo_power(false).await?;
        hw.set_backing_pump(true).await?;
        self.sleep_or_abort(Duration::from_millis(1500)).await?;

        // Phase 1: roughing.
        self.publish(SeqState::PumpDownRoughing, 0.10,
            &format!("Roughing — waiting for Pirani ≤ {:.2e} mbar", set.crossover_pressure_mbar));
        hw.set_valve(Valve::Roughing, true).await?;
        self.wait_until(set.roughing_timeout_s, |_| async {
            let p = hw.read_pressures().await?;
            Ok(p.pirani_mbar.map(|v| v <= set.crossover_pressure_mbar).unwrap_or(false))
        }, SeqState::PumpDownRoughing, 0.10, 0.40,
           "Roughing", "Roughing timed out — chamber may have a leak").await?;

        // Phase 2: crossover.
        self.publish(SeqState::PumpDownCrossover, 0.45, "Crossover — closing roughing, opening foreline");
        hw.set_valve(Valve::Roughing, false).await?;
        self.sleep_or_abort(Duration::from_millis(1000)).await?;
        hw.set_valve(Valve::Foreline, true).await?;
        // Profile-aware: HiPace 800 also has a chamber gate. Open it.
        if matches!(set.pump_profile, crate::state::PumpProfile::HiPace800Scroll) {
            hw.set_valve(Valve::Gate, true).await?;
        }
        self.sleep_or_abort(Duration::from_millis(500)).await?;

        // Phase 3: turbo accel.
        self.publish(SeqState::PumpDownTurbo, 0.50,
            &format!("Turbo accelerating — waiting for ≥{:.0}% nominal", set.turbo_at_speed_pct));
        hw.set_turbo_power(true).await?;
        self.wait_until(set.turbo_accel_timeout_s, |_| async {
            self.check_turbo_safety(&*hw, &set).await?;
            let t = hw.read_turbo().await?;
            Ok(t.speed_pct >= set.turbo_at_speed_pct)
        }, SeqState::PumpDownTurbo, 0.50, 0.85,
           "Turbo accelerating", "Turbo failed to reach speed — check backing pump and foreline").await?;

        // Phase 4: at-vacuum poll. No timeout — UHV is allowed to take its
        // sweet time. User can click Abort if they're done waiting.
        self.publish(SeqState::AtVacuum, 0.85,
            &format!("Reaching base pressure — target {:.0e} mbar", set.at_vacuum_pressure_mbar));
        loop {
            self.check_abort()?;
            self.check_turbo_safety(&*hw, &set).await?;
            let p = hw.read_pressures().await?;
            // Either gauge meeting the threshold counts. Real systems flip
            // from Pirani to IG around 1e-3 mbar.
            let ok = p.ion_mbar.map(|v| v <= set.at_vacuum_pressure_mbar).unwrap_or(false)
                  || p.pirani_mbar.map(|v| v <= set.at_vacuum_pressure_mbar).unwrap_or(false);
            if ok { break; }
            sleep(Duration::from_millis(500)).await;
        }
        self.publish(SeqState::AtVacuum, 1.0, "At vacuum — chamber ready");
        Ok(())
    }

    /// Drive a full vent. Phases:
    ///   1. Stop turbo; wait for spindown threshold.
    ///   2. Close foreline (isolate turbo from chamber).
    ///   3. Open vent; wait for atmospheric pressure (or timeout).
    ///   4. Close vent.
    pub async fn vent(self: Arc<Self>, hw: Arc<dyn Hardware>, set: Settings) -> Result<()> {
        self.clear_abort();
        self.publish(SeqState::VentInit, 0.05, "Vent — stopping turbo");
        hw.set_turbo_power(false).await?;

        self.publish(SeqState::VentTurboSpindown, 0.10,
            &format!("Spindown — waiting for ≤{:.0}% nominal", set.turbo_spindown_pct));
        self.wait_until(set.spindown_timeout_s, |_| async {
            let t = hw.read_turbo().await?;
            Ok(t.speed_pct <= set.turbo_spindown_pct)
        }, SeqState::VentTurboSpindown, 0.10, 0.45,
           "Spindown", "Turbo did not spin down in time").await?;

        self.publish(SeqState::VentOpening, 0.50, "Isolating turbo, opening vent");
        hw.set_valve(Valve::Foreline, false).await?;
        if matches!(set.pump_profile, crate::state::PumpProfile::HiPace800Scroll) {
            hw.set_valve(Valve::Gate, false).await?;
        }
        self.sleep_or_abort(Duration::from_millis(500)).await?;
        hw.set_valve(Valve::Vent, true).await?;

        self.wait_until(set.vent_timeout_s, |_| async {
            let p = hw.read_pressures().await?;
            // If Pirani is over-range (None) AND we've been venting, that
            // also counts as "vented" — gauge is just out of its window.
            Ok(p.pirani_mbar.map(|v| v >= set.vent_complete_mbar).unwrap_or(true))
        }, SeqState::VentOpening, 0.50, 0.95,
           "Venting", "Vent did not complete — N₂ supply / valve issue").await?;

        hw.set_valve(Valve::Vent, false).await?;
        self.publish(SeqState::VentComplete, 1.0, "Vented to atmosphere — safe to open chamber");
        Ok(())
    }

    /// Force the rig into a safe state regardless of where in a sequence
    /// we are. Idempotent. Always: close all valves, turbo off, backing on
    /// (so foreline doesn't go atmospheric while turbo coasts down).
    pub async fn abort_to_safe(self: Arc<Self>, hw: Arc<dyn Hardware>) -> Result<()> {
        self.publish(SeqState::Aborted, 0.0, "Aborted — driving to safe state");
        // Tear-down order matters: kill turbo power first so it starts
        // spinning down while we close valves around it.
        let _ = hw.set_turbo_power(false).await;
        let _ = hw.set_valve(Valve::Roughing, false).await;
        let _ = hw.set_valve(Valve::Foreline, false).await;
        let _ = hw.set_valve(Valve::Vent,     false).await;
        let _ = hw.set_valve(Valve::Gate,     false).await;
        let _ = hw.set_backing_pump(true).await;
        self.publish(SeqState::Aborted, 1.0, "Safe state");
        Ok(())
    }

    // ── Internal helpers ────────────────────────────────────────────────

    fn check_abort(&self) -> Result<()> {
        if self.aborted() { Err(anyhow!("Aborted by user")) } else { Ok(()) }
    }

    async fn sleep_or_abort(&self, d: Duration) -> Result<()> {
        let until = Instant::now() + d;
        while Instant::now() < until {
            self.check_abort()?;
            let remaining = until - Instant::now();
            sleep(remaining.min(Duration::from_millis(100))).await;
        }
        Ok(())
    }

    async fn check_turbo_safety(&self, hw: &dyn Hardware, set: &Settings) -> Result<()> {
        let t = hw.read_turbo().await?;
        if let Some(temp) = t.temp_c {
            if temp >= set.turbo_overtemp_c {
                return Err(anyhow!("Turbo over-temperature ({:.1} °C ≥ {:.1} °C limit) — emergency stop",
                                   temp, set.turbo_overtemp_c));
            }
        }
        if matches!(t.state, crate::state::TurboState::Faulted) {
            return Err(anyhow!("Turbo controller reported a fault"));
        }
        Ok(())
    }

    /// Poll `cond` every 500 ms until it returns true. Emit progress between
    /// `prog_lo` and `prog_hi` linearly with elapsed/timeout (capped at hi).
    /// `timeout_s == 0` means no timeout.
    async fn wait_until<F, Fut>(
        &self,
        timeout_s: u32,
        cond: F,
        state: SeqState,
        prog_lo: f64,
        prog_hi: f64,
        label: &str,
        timeout_msg: &str,
    ) -> Result<()>
    where
        F:   Fn(()) -> Fut,
        Fut: std::future::Future<Output = Result<bool>>,
    {
        let start = Instant::now();
        let timeout = if timeout_s == 0 { None } else { Some(Duration::from_secs(timeout_s as u64)) };
        loop {
            self.check_abort()?;
            if cond(()).await? { return Ok(()); }
            if let Some(t) = timeout {
                let elapsed = start.elapsed();
                if elapsed >= t {
                    return Err(anyhow!("{} timeout — {}", label, timeout_msg));
                }
                let frac = (elapsed.as_secs_f64() / t.as_secs_f64()).min(1.0);
                let prog = prog_lo + (prog_hi - prog_lo) * frac;
                self.publish(state, prog, &format!("{} — {:.0}s elapsed", label, elapsed.as_secs()));
            }
            sleep(Duration::from_millis(500)).await;
        }
    }
}
