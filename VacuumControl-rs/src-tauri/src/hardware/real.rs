//! Real-hardware impl. Talks to:
//!   - Pfeiffer TPG 366 MaxiGauge (RS232, ASCII mnemonic protocol).
//!     Mnemonic + <CR><LF>; the gauge replies with ACK (0x06) then on next
//!     query returns the data, also <CR><LF>-terminated.
//!   - Pfeiffer TC 1200 turbo controller (RS232, ASCII "telegram" protocol).
//!     Telegram: aaa AA ppp ll DDDD CC <CR>, where AA = 00 query / 10 control.
//!     Checksum = sum of all preceding ASCII bytes mod 256, two hex digits.
//!
//! Errors during a single read/write don't tear down the connection — the
//! sequencer's wait_until polls anyway, and the UI will keep the last good
//! reading until a fresh one lands.

use super::{Hardware, PressureReading, TurboReading, Valve};
use crate::settings::Settings;
use crate::state::{PumpProfile, TurboState};
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serialport::SerialPort;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Both serial ports + the profile that decides where each valve maps.
pub struct RealHardware {
    tpg:     Arc<Mutex<Box<dyn SerialPort>>>,
    tc:      Arc<Mutex<Box<dyn SerialPort>>>,
    tc_addr: u8,
    profile: PumpProfile,
}

impl RealHardware {
    pub fn open(set: &Settings) -> Result<Self> {
        let tpg = serialport::new(&set.tpg_port, set.tpg_baud)
            .timeout(Duration::from_millis(500))
            .open()
            .map_err(|e| anyhow!("TPG open ({}): {}", set.tpg_port, e))?;
        let tc = serialport::new(&set.tc_port, set.tc_baud)
            .timeout(Duration::from_millis(500))
            .open()
            .map_err(|e| anyhow!("TC open ({}): {}", set.tc_port, e))?;
        Ok(Self {
            tpg:     Arc::new(Mutex::new(tpg)),
            tc:      Arc::new(Mutex::new(tc)),
            tc_addr: set.tc_address,
            profile: set.pump_profile,
        })
    }
}

// ── TPG 366 helpers ─────────────────────────────────────────────────────────
//
// Two-step transaction per query:
//   tx: "PRn\r\n"  rx: 0x06 (ACK) — gauge confirms it understood
//   tx: 0x05 (ENQ) rx: "<status>,<pressure>\r\n"
//
// Status codes (per Pfeiffer manual): 0=OK, 1=underrange, 2=overrange,
// 3=sensor error, 4=sensor off, 5=no sensor, 6=ID error. Anything ≠ 0 is
// reported as None to the caller so the UI shows a dash.

fn tpg_query(port: &Arc<Mutex<Box<dyn SerialPort>>>, mnemonic: &str) -> Result<String> {
    let mut p = port.lock().unwrap();
    let cmd = format!("{}\r\n", mnemonic);
    p.write_all(cmd.as_bytes())?;
    p.flush()?;
    // ACK byte
    let mut ack = [0u8; 1];
    let _ = p.read_exact(&mut ack); // tolerate spurious bytes
    // ENQ
    p.write_all(&[0x05])?;
    p.flush()?;
    // Read until \r\n
    let mut buf = Vec::with_capacity(64);
    let mut byte = [0u8; 1];
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        if p.read_exact(&mut byte).is_err() { break; }
        if byte[0] == b'\r' { continue; }
        if byte[0] == b'\n' { break; }
        buf.push(byte[0]);
        if buf.len() > 256 { break; }
    }
    Ok(String::from_utf8_lossy(&buf).trim().to_string())
}

fn tpg_read_channel(port: &Arc<Mutex<Box<dyn SerialPort>>>, channel: u8) -> Result<Option<f64>> {
    let line = tpg_query(port, &format!("PR{}", channel))?;
    let parts: Vec<&str> = line.splitn(2, ',').collect();
    if parts.len() != 2 { return Err(anyhow!("TPG malformed reply: '{}'", line)); }
    let status: u8 = parts[0].trim().parse().map_err(|_| anyhow!("TPG bad status '{}'", parts[0]))?;
    if status != 0 { return Ok(None); }
    let p: f64 = parts[1].trim().parse().map_err(|_| anyhow!("TPG bad pressure '{}'", parts[1]))?;
    Ok(Some(p))
}

/// SR4 sets relay-output thresholds; SEN switches sensors on/off.
/// For simple "set relay N on/off" we use the SPS mnemonic. Number is
/// 1..=6; state is 0=off, 1=on.
fn tpg_set_relay(port: &Arc<Mutex<Box<dyn SerialPort>>>, relay: u8, on: bool) -> Result<()> {
    let _ = tpg_query(port, &format!("SPS,{},{}", relay, if on { 1 } else { 0 }))?;
    Ok(())
}

// ── TC 1200 helpers ─────────────────────────────────────────────────────────
//
// Telegram = 16 + ll bytes, all ASCII:
//   aaa  3-digit address, e.g. "001"
//   AA   action: "00" query, "10" control
//   ppp  3-digit parameter number
//   ll   2-digit data length
//   DDDD data, length ll
//   CC   2-digit checksum (sum of preceding bytes mod 256, two hex)
//   CR
//
// Boolean parameters (P010 turbo on/off, P012 vent open) take "111111" for
// true / "000000" for false in a 6-byte data field.
//
// Numeric parameters (P309 speed Hz, P310 current 0.01 A, P315 nominal Hz)
// come back as zero-padded decimals in a 6-byte field.

fn tc_telegram(addr: u8, action: u8, param: u16, data: &str) -> String {
    let body = format!("{:03}{:02}{:03}{:02}{}", addr, action, param, data.len(), data);
    let cs: u32 = body.bytes().map(|b| b as u32).sum::<u32>() % 256;
    format!("{}{:03}\r", body, cs)
}

fn tc_query(port: &Arc<Mutex<Box<dyn SerialPort>>>, addr: u8, param: u16) -> Result<String> {
    let mut p = port.lock().unwrap();
    let tx = tc_telegram(addr, 0, param, "=?");
    p.write_all(tx.as_bytes())?;
    p.flush()?;
    let mut buf = Vec::with_capacity(64);
    let mut byte = [0u8; 1];
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(700) {
        if p.read_exact(&mut byte).is_err() { break; }
        if byte[0] == b'\r' { break; }
        buf.push(byte[0]);
        if buf.len() > 256 { break; }
    }
    let line = String::from_utf8_lossy(&buf).to_string();
    // Reply layout: aaa10pppllDDDDCC. Strip everything except DDDD.
    if line.len() < 13 { return Err(anyhow!("TC short reply: '{}'", line)); }
    let ll: usize = line[8..10].parse().unwrap_or(0);
    if line.len() < 10 + ll { return Err(anyhow!("TC truncated reply: '{}'", line)); }
    Ok(line[10..10 + ll].to_string())
}

fn tc_write(port: &Arc<Mutex<Box<dyn SerialPort>>>, addr: u8, param: u16, data: &str) -> Result<()> {
    let mut p = port.lock().unwrap();
    let tx = tc_telegram(addr, 10, param, data);
    p.write_all(tx.as_bytes())?;
    p.flush()?;
    // Read and discard the ack reply (same telegram echoed back).
    let mut sink = [0u8; 64];
    let _ = p.read(&mut sink);
    Ok(())
}

fn tc_read_u32(port: &Arc<Mutex<Box<dyn SerialPort>>>, addr: u8, param: u16) -> Result<u32> {
    let raw = tc_query(port, addr, param)?;
    raw.trim().parse().map_err(|_| anyhow!("TC param {} unparseable: '{}'", param, raw))
}

// ── Hardware trait ──────────────────────────────────────────────────────────

#[async_trait]
impl Hardware for RealHardware {
    async fn read_pressures(&self) -> Result<PressureReading> {
        let tpg1 = self.tpg.clone();
        let tpg2 = self.tpg.clone();
        // Channel 2 = Pirani, Channel 1 = Ion gauge (matches App A wiring).
        // Ion gauge can be absent on Pirani-only profiles — tolerate failure.
        let pirani = tokio::task::spawn_blocking(move || tpg_read_channel(&tpg2, 2))
            .await
            .map_err(|e| anyhow!("Pirani read panic: {}", e))?
            .unwrap_or(None);
        let ion = match self.profile {
            PumpProfile::HiPace1200Duo => {
                tokio::task::spawn_blocking(move || tpg_read_channel(&tpg1, 1))
                    .await
                    .map_err(|e| anyhow!("Ion gauge read panic: {}", e))?
                    .unwrap_or(None)
            }
            PumpProfile::HiPace800Scroll => None,
        };
        Ok(PressureReading { pirani_mbar: pirani, ion_mbar: ion })
    }

    async fn read_turbo(&self) -> Result<TurboReading> {
        let port  = self.tc.clone();
        let addr  = self.tc_addr;

        // Speed Hz (P309), nominal Hz (P315), current 0.01 A (P310), temp °C (P326), error code (P303).
        let port1 = port.clone();
        let port2 = port.clone();
        let port3 = port.clone();
        let port4 = port.clone();
        let port5 = port.clone();

        let speed_hz   = tokio::task::spawn_blocking(move || tc_read_u32(&port1, addr, 309)).await
            .map_err(|e| anyhow!("TC speed panic: {}", e))?
            .unwrap_or(0);
        let nominal_hz = tokio::task::spawn_blocking(move || tc_read_u32(&port2, addr, 315)).await
            .map_err(|e| anyhow!("TC nominal panic: {}", e))?
            .unwrap_or(1000); // safe default — yields 0% rather than div-by-zero
        let cur_units  = tokio::task::spawn_blocking(move || tc_read_u32(&port3, addr, 310)).await
            .map_err(|e| anyhow!("TC current panic: {}", e))?
            .unwrap_or(0);
        let temp_c     = tokio::task::spawn_blocking(move || tc_read_u32(&port4, addr, 326)).await
            .map_err(|e| anyhow!("TC temp panic: {}", e))?
            .ok();
        let err_code   = tokio::task::spawn_blocking(move || tc_read_u32(&port5, addr, 303)).await
            .map_err(|e| anyhow!("TC error code panic: {}", e))?
            .unwrap_or(0);

        let pct = (speed_hz as f64) / (nominal_hz.max(1) as f64) * 100.0;
        let powered = pct > 1.0; // controller flag would be cleaner; this is good enough
        let state = if err_code != 0 {
            TurboState::Faulted
        } else if !powered && pct < 1.0 {
            TurboState::Stopped
        } else if pct >= 95.0 {
            TurboState::AtSpeed
        } else if powered {
            TurboState::Accelerating
        } else {
            TurboState::Decelerating
        };

        Ok(TurboReading {
            speed_pct: pct,
            current_a: cur_units as f64 * 0.01,
            temp_c:    temp_c.map(|t| t as f64),
            state,
            powered,
        })
    }

    async fn set_valve(&self, valve: Valve, open: bool) -> Result<()> {
        let port_tpg = self.tpg.clone();
        let port_tc  = self.tc.clone();
        let addr = self.tc_addr;
        let val_str = if open { "111111" } else { "000000" };

        match (self.profile, valve) {
            // HiPace 1200 + Duo profile (App A's wiring):
            //   V1 (roughing) = TPG relay 1
            //   V2 (foreline) = TC accessory output 788
            //   V3 (vent)     = TC P012
            //   Gate          = no-op (no separate chamber gate)
            (PumpProfile::HiPace1200Duo, Valve::Roughing) => {
                tokio::task::spawn_blocking(move || tpg_set_relay(&port_tpg, 1, open)).await
                    .map_err(|e| anyhow!("relay set panic: {}", e))??;
            }
            (PumpProfile::HiPace1200Duo, Valve::Foreline) => {
                tokio::task::spawn_blocking(move || tc_write(&port_tc, addr, 788, val_str)).await
                    .map_err(|e| anyhow!("TC write panic: {}", e))??;
            }
            (PumpProfile::HiPace1200Duo, Valve::Vent) => {
                tokio::task::spawn_blocking(move || tc_write(&port_tc, addr, 12, val_str)).await
                    .map_err(|e| anyhow!("TC write panic: {}", e))??;
            }
            (PumpProfile::HiPace1200Duo, Valve::Gate) => {
                // No physical gate on this profile; keep silent so the
                // sequencer can issue Gate without special-casing.
            }
            // HiPace 800 + Scroll profile (App B's wiring):
            //   Roughing = TPG relay 1
            //   Foreline = TPG relay 2  (App B's "backing valve")
            //   Vent     = TPG relay 4
            //   Gate     = TPG relay 3
            (PumpProfile::HiPace800Scroll, v) => {
                let n: u8 = match v {
                    Valve::Roughing => 1,
                    Valve::Foreline => 2,
                    Valve::Gate     => 3,
                    Valve::Vent     => 4,
                };
                tokio::task::spawn_blocking(move || tpg_set_relay(&port_tpg, n, open)).await
                    .map_err(|e| anyhow!("relay set panic: {}", e))??;
            }
        }
        Ok(())
    }

    async fn set_turbo_power(&self, on: bool) -> Result<()> {
        let port = self.tc.clone();
        let addr = self.tc_addr;
        let val  = if on { "111111" } else { "000000" };
        tokio::task::spawn_blocking(move || tc_write(&port, addr, 10, val)).await
            .map_err(|e| anyhow!("TC turbo power panic: {}", e))??;
        Ok(())
    }

    async fn set_backing_pump(&self, on: bool) -> Result<()> {
        // HiPace 1200 wiring: backing pump on TPG relay 5 (App A).
        // HiPace 800 wiring:  scroll pump on TPG relay 5 too.
        let port = self.tpg.clone();
        tokio::task::spawn_blocking(move || tpg_set_relay(&port, 5, on)).await
            .map_err(|e| anyhow!("backing pump set panic: {}", e))??;
        Ok(())
    }

    fn is_demo(&self) -> bool { false }
}
