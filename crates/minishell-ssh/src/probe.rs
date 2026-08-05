use std::collections::VecDeque;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use minishell_core::Machine;

pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
pub const DEFAULT_WORKERS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeStatus {
    Ok,
    Down,
    Unknown,
}

#[derive(Debug, Clone, Copy)]
pub struct ProbeResult {
    pub status: ProbeStatus,
    pub latency_ms: Option<u64>,
}

impl ProbeResult {
    pub fn ok(latency: Duration) -> Self {
        ProbeResult {
            status: ProbeStatus::Ok,
            latency_ms: Some(latency.as_millis() as u64),
        }
    }

    pub fn down() -> Self {
        ProbeResult {
            status: ProbeStatus::Down,
            latency_ms: None,
        }
    }

    pub fn unknown() -> Self {
        ProbeResult {
            status: ProbeStatus::Unknown,
            latency_ms: None,
        }
    }
}

pub fn probe_host(host: &str, port: i32, timeout: Duration) -> ProbeResult {
    let start = Instant::now();
    let addrs = match (host, port as u16).to_socket_addrs() {
        Ok(addrs) => addrs,
        Err(_) => return ProbeResult::down(),
    };
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_) => return ProbeResult::ok(start.elapsed()),
            Err(_) => continue,
        }
    }
    ProbeResult::down()
}

pub struct ProbeHandle {
    rx: mpsc::Receiver<(i64, ProbeResult)>,
    thread: Option<JoinHandle<()>>,
}

impl ProbeHandle {
    pub fn try_recv(&self) -> Option<(i64, ProbeResult)> {
        match self.rx.try_recv() {
            Ok(pair) => Some(pair),
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => None,
        }
    }

    pub fn is_done(&self) -> bool {
        self.thread.as_ref().map_or(true, |t| t.is_finished())
    }

    pub fn join(self) {
        if let Some(t) = self.thread {
            let _ = t.join();
        }
    }
}

pub fn start_probe(machines: &[Machine], workers: usize, timeout: Duration) -> ProbeHandle {
    let (tx, rx) = mpsc::channel();
    let queue: Arc<Mutex<VecDeque<(i64, Machine)>>> = Arc::new(Mutex::new(
        machines.iter().map(|m| (m.id, m.clone())).collect(),
    ));
    let worker_count = workers.max(1).min(queue.lock().unwrap().len().max(1));

    let thread = std::thread::spawn(move || {
        let mut handles = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let queue = Arc::clone(&queue);
            let tx = tx.clone();
            handles.push(std::thread::spawn(move || {
                loop {
                    let item = queue.lock().unwrap().pop_front();
                    match item {
                        Some((id, machine)) => {
                            let result = probe_host(machine.effective_host(), machine.port, timeout);
                            let _ = tx.send((id, result));
                        }
                        None => break,
                    }
                }
            }));
        }
        drop(tx);
        for handle in handles {
            let _ = handle.join();
        }
    });

    ProbeHandle {
        rx,
        thread: Some(thread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn machine(id: i64, ip: &str, port: i32) -> Machine {
        Machine {
            id,
            num: id as i32,
            nat_ip: "-".into(),
            ip: ip.into(),
            username: "root".into(),
            password: "-".into(),
            port,
            private_key_path: "-".into(),
            device: "-".into(),
            remark: "-".into(),
        }
    }

    #[test]
    fn probe_ok_on_open_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let result = probe_host("127.0.0.1", port as i32, Duration::from_secs(2));
        assert_eq!(result.status, ProbeStatus::Ok);
        assert!(result.latency_ms.is_some());
    }

    #[test]
    fn probe_down_on_closed_port() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let result = probe_host("127.0.0.1", port as i32, Duration::from_secs(2));
        assert_eq!(result.status, ProbeStatus::Down);
        assert!(result.latency_ms.is_none());
    }

    #[test]
    fn probe_down_on_unresolvable_host() {
        let result = probe_host("invalid.host.invalid", 22, Duration::from_millis(200));
        assert_eq!(result.status, ProbeStatus::Down);
    }

    #[test]
    fn probe_times_out() {
        let start = Instant::now();
        let result = probe_host("203.0.113.1", 22, Duration::from_millis(200));
        assert_eq!(result.status, ProbeStatus::Down);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn concurrent_results_match_machines_one_to_one() {
        let mut machines = Vec::new();
        let mut ok_ids = std::collections::HashSet::new();
        let mut listeners = Vec::new();
        for i in 0..10 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            machines.push(machine(i as i64, "127.0.0.1", port as i32));
            ok_ids.insert(i as i64);
            listeners.push(listener);
        }
        for i in 10..20 {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            drop(listener);
            machines.push(machine(i as i64, "127.0.0.1", port as i32));
        }

        let handle = start_probe(&machines, 4, Duration::from_secs(2));
        let mut results = std::collections::HashMap::new();
        loop {
            if let Some((id, result)) = handle.try_recv() {
                results.insert(id, result.status);
            } else if handle.is_done() {
                break;
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        handle.join();

        assert_eq!(results.len(), 20);
        for (id, status) in &results {
            if ok_ids.contains(id) {
                assert_eq!(*status, ProbeStatus::Ok);
            } else {
                assert_eq!(*status, ProbeStatus::Down);
            }
        }
    }
}
