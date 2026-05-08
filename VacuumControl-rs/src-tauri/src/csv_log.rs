//! Per-session CSV log. Mirrors App B's behaviour but with a configurable
//! directory (App B hardcoded C:\VacuumLogs). One file per session, named
//! `<yyyyMMdd>_<HHmmss>.csv`. Polling code calls `log_row` once per tick
//! when a writer is open; flush happens on each row so a crash doesn't
//! lose the last few seconds.

use crate::state::SystemState;
use anyhow::Result;
use chrono::Local;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Mutex;

const HEADER: &str =
    "timestamp,pirani_mbar,ion_mbar,turbo_speed_pct,turbo_current_a,turbo_temp_c,turbo_state,seq_state,seq_message\n";

pub struct CsvLogger {
    inner: Mutex<Option<CsvWriter>>,
}

struct CsvWriter {
    path:  PathBuf,
    file:  BufWriter<File>,
}

impl CsvLogger {
    pub fn new() -> Self { Self { inner: Mutex::new(None) } }

    /// Open a fresh session file in `dir`. Idempotent: closes any prior
    /// session first. Pass an empty string to disable logging entirely.
    pub fn open_session(&self, dir: &str) -> Result<Option<PathBuf>> {
        let mut guard = self.inner.lock().unwrap();
        *guard = None; // close previous session
        if dir.trim().is_empty() { return Ok(None); }

        let dir_path = PathBuf::from(dir);
        create_dir_all(&dir_path)?;
        let stamp = Local::now().format("%Y%m%d_%H%M%S").to_string();
        let path  = dir_path.join(format!("{stamp}.csv"));
        let mut file = BufWriter::new(
            OpenOptions::new().create(true).append(true).open(&path)?
        );
        file.write_all(HEADER.as_bytes())?;
        file.flush()?;
        let result_path = path.clone();
        *guard = Some(CsvWriter { path, file });
        Ok(Some(result_path))
    }

    /// Append one row. Silently no-ops if no session is open.
    pub fn log_row(&self, s: &SystemState) {
        let mut guard = self.inner.lock().unwrap();
        let Some(w) = guard.as_mut() else { return; };
        let ts = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let pirani = s.pirani_mbar.map(|v| format!("{:.6e}", v)).unwrap_or_default();
        let ion    = s.ion_mbar.map(|v| format!("{:.6e}", v)).unwrap_or_default();
        let temp   = s.turbo_temp_c.map(|v| format!("{:.1}", v)).unwrap_or_default();
        // seq_message can contain commas — quote it.
        let msg_q = s.seq_message.replace('"', "\"\"");
        let line = format!(
            "{},{},{},{:.2},{:.3},{},{:?},{:?},\"{}\"\n",
            ts, pirani, ion,
            s.turbo_speed_pct, s.turbo_current_a, temp,
            s.turbo_state, s.seq_state, msg_q,
        );
        let _ = w.file.write_all(line.as_bytes());
        let _ = w.file.flush(); // flush every row — cheap, crash-safe
    }

    /// Path of the active session file, if any. Useful for the UI to show
    /// "logging to: …" so the user can find the file later.
    #[allow(dead_code)]
    pub fn current_path(&self) -> Option<PathBuf> {
        self.inner.lock().unwrap().as_ref().map(|w| w.path.clone())
    }
}
