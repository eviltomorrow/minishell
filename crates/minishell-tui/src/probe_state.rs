use std::collections::HashMap;
use std::time::Instant;

use ratatui::style::{Color, Style};

use minishell_core::Machine;
use minishell_ssh::probe::{self, ProbeHandle, ProbeResult, ProbeStatus};

const PROBE_DOTS: usize = 5;

pub enum SummaryLevel {
    Probing,
    Empty,
    AllOk,
    Partial,
    None,
}

/// Owns the probe domain for the TUI: the in-flight handle, the collected
/// results, and everything the rest of the UI needs to render them.
///
/// This is the single place that knows about probing — `table.rs` renders
/// a ready-made `(text, style)` cell without importing a networking crate.
pub struct ProbeState {
    handle: Option<ProbeHandle>,
    results: HashMap<i64, ProbeResult>,
    started: Option<Instant>,
}

impl ProbeState {
    pub fn new() -> Self {
        ProbeState {
            handle: None,
            results: HashMap::new(),
            started: None,
        }
    }

    /// Kicks off a background probe of every machine (non-blocking).
    pub fn start(&mut self, machines: &[Machine]) {
        self.results.clear();
        self.started = Some(Instant::now());
        self.handle = Some(probe::start_probe(
            machines,
            probe::DEFAULT_WORKERS,
            probe::PROBE_TIMEOUT,
        ));
    }

    pub fn is_done(&self) -> bool {
        self.handle.as_ref().map_or(true, |h| h.is_done())
    }

    /// Collects finished probe results. Returns `true` if any arrived.
    pub fn poll(&mut self) -> bool {
        let mut updated = false;
        if let Some(handle) = &self.handle {
            while let Some((id, result)) = handle.try_recv() {
                self.results.insert(id, result);
                updated = true;
            }
        }
        updated
    }

    fn phase(&self) -> usize {
        self.started
            .map(|s| s.elapsed().as_millis() as usize / 150)
            .unwrap_or(0)
    }

    /// Renders the status cell for one machine. `None` cells animate while a
    /// probe is in flight and show an idle dot once it finishes.
    pub fn cell_for(&self, id: i64) -> (String, Style) {
        match self.results.get(&id) {
            Some(r) if r.status == ProbeStatus::Ok => {
                let ms = r.latency_ms.unwrap_or(0);
                let color = if ms < 100 {
                    Color::Green
                } else if ms < 500 {
                    Color::Yellow
                } else {
                    Color::Red
                };
                (format!("● {}ms", ms), Style::default().fg(color))
            }
            Some(_) => ("● down".to_string(), Style::default().fg(Color::Red)),
            None if !self.is_done() => {
                let lit = self.phase() % PROBE_DOTS;
                let dots: String = (0..PROBE_DOTS)
                    .map(|i| if i == lit { '▪' } else { '▫' })
                    .collect();
                (dots, Style::default().fg(Color::DarkGray))
            }
            None => ("·".to_string(), Style::default().fg(Color::DarkGray)),
        }
    }

    /// Builds the bottom-bar summary text and a coarse level that the caller
    /// maps onto its own styles.
    pub fn summary(&self, machines: &[Machine]) -> (String, SummaryLevel) {
        let total = machines.len();
        if !self.is_done() {
            return ("探测中…".to_string(), SummaryLevel::Probing);
        }
        if total == 0 {
            return ("0/0 可达".to_string(), SummaryLevel::Empty);
        }
        let ok = machines
            .iter()
            .filter(|m| matches!(self.results.get(&m.id), Some(r) if r.status == ProbeStatus::Ok))
            .count();
        let down = machines
            .iter()
            .filter(|m| matches!(self.results.get(&m.id), Some(r) if r.status == ProbeStatus::Down))
            .count();
        let unprobed = total - ok - down;
        let text = if ok > 0 {
            format!("{}/{} 可达 · {} 不可达 · {} 未测", ok, total, down, unprobed)
        } else {
            format!("0/{} 可达 · {} 不可达 · {} 未测", total, down, unprobed)
        };
        let level = if ok == total {
            SummaryLevel::AllOk
        } else if ok > 0 {
            SummaryLevel::Partial
        } else {
            SummaryLevel::None
        };
        (text, level)
    }
}
