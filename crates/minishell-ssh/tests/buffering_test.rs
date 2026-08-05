/// ── SSH channel buffering regression tests ──
///
/// These tests drive the PRODUCTION seam `minishell_ssh::drain_reads`
/// (the always-drain / eof-after-drain ordering used by `run_session_loop`),
/// NOT a copied mirror of it. `SimChannel` fakes libssh2's internal
/// transport buffering (it eagerly consumes the TCP socket into its own
/// buffer, so data can be present there without `poll()` on the raw fd
/// reporting `POLLIN`).
use std::io::{Read, Write, ErrorKind};
use std::net::{TcpListener, TcpStream, Shutdown};
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use minishell_ssh::drain_reads;

/// Simulates libssh2's Channel::read:
/// - Eagerly reads ALL available TCP data into an internal buffer
/// - Returns at most `buf.len()` bytes per call
/// - Surplus stays in internal buffer — invisible to poll() on the raw fd
struct SimChannel {
    stream: TcpStream,
    inner: Vec<u8>,
    pos: usize,
    eof_flag: bool,
}

impl SimChannel {
    fn new(stream: TcpStream) -> Self {
        Self { stream, inner: Vec::new(), pos: 0, eof_flag: false }
    }

    fn buffered(&self) -> usize { self.inner.len().saturating_sub(self.pos) }
}

impl Read for SimChannel {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.inner.len() {
            let mut tcp_buf = [0u8; 65536];
            match self.stream.read(&mut tcp_buf) {
                Ok(0) => { self.eof_flag = true; return Ok(0); }
                Ok(n) => { self.inner = tcp_buf[..n].to_vec(); self.pos = 0; }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    return Err(std::io::Error::new(ErrorKind::WouldBlock, "would block"));
                }
                Err(e) => return Err(e),
            }
        }
        let avail = self.inner.len().saturating_sub(self.pos);
        let n = std::cmp::min(avail, buf.len());
        buf[..n].copy_from_slice(&self.inner[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Sets up a SimChannel with a sender writing data_size bytes.
/// Data is in the kernel buffer but NOT yet consumed by SimChannel.
fn setup_channel(data_size: usize) -> (SimChannel, RawFd, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));

    let s = stop.clone();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(&vec![b'D'; data_size]);
            let _ = stream.flush();
            while !s.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    let rx = TcpStream::connect(addr).unwrap();
    rx.set_nonblocking(true).unwrap();
    let fd = rx.as_raw_fd();
    let ch = SimChannel::new(rx);

    // Wait for data to arrive in kernel buffer
    thread::sleep(Duration::from_millis(100));

    (ch, fd, stop)
}

/// Like setup_channel but allows the sender to close (for eof testing).
/// Drains into a buffer that records what drain_reads delivered.
fn setup_channel_with_close(data_size: usize) -> (SimChannel, RawFd, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let sender = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(&vec![b'E'; data_size]);
            let _ = stream.flush();
            // Close immediately — sends FIN
            let _ = stream.shutdown(Shutdown::Write);
        }
    });

    let rx = TcpStream::connect(addr).unwrap();
    rx.set_nonblocking(true).unwrap();
    let fd = rx.as_raw_fd();
    let ch = SimChannel::new(rx);

    // Wait for data + FIN to arrive
    thread::sleep(Duration::from_millis(200));

    (ch, fd, sender)
}

fn drain_all(ch: &mut SimChannel) -> (usize, bool) {
    let mut out = Vec::new();
    let eof = drain_reads(ch, &mut out).unwrap();
    (out.len(), eof)
}

// ─────────────────────────────────────────────
//  Regression: all data delivered across poll cycles
// ─────────────────────────────────────────────
#[test]
fn regression_all_data_delivered_across_multiple_poll_cycles() {
    // Simulates a real session: data arrives in bursts across poll cycles.
    // Each cycle calls the production drain_reads once; no byte may be lost.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();

    let sender = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let _ = stream.write_all(&[b'A'; 12288]);
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(50));
            let _ = stream.write_all(&[b'B'; 5120]);
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(50));
            let _ = stream.shutdown(Shutdown::Write);
            while !s.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    let rx = TcpStream::connect(addr).unwrap();
    rx.set_nonblocking(true).unwrap();
    let mut ch = SimChannel::new(rx);
    thread::sleep(Duration::from_millis(200));

    let mut total = 0usize;
    let mut eof = false;
    while !eof {
        let (n, done) = drain_all(&mut ch);
        total += n;
        eof = done;
        thread::sleep(Duration::from_millis(5));
    }

    eprintln!("REGRESSION: read {total}/17408 bytes (burst1=12288 + burst2=5120)");
    assert_eq!(total, 17408, "regression: all data must be delivered");
    assert!(eof);
    stop.store(true, Ordering::Relaxed);
    sender.join().unwrap();
}

// ─────────────────────────────────────────────
//  H1: internal buffer is drained even without POLLIN
// ─────────────────────────────────────────────
#[test]
fn h1_drain_all_buffered_data_despite_no_pollin() {
    let (mut ch, _fd, stop) = setup_channel(50000);

    // First drain pulls ALL 50000 bytes out of the internal buffer even
    // though a later poll() on the raw fd (already consumed) reports nothing.
    let (n1, done1) = drain_all(&mut ch);
    eprintln!("H1: n={n1}, done={done1}, buffered={}", ch.buffered());
    assert_eq!(n1, 50000);
    assert!(!done1, "peer is still open — no EOF yet");

    // Second drain: nothing left.
    let (n2, _) = drain_all(&mut ch);
    assert_eq!(n2, 0);
    assert_eq!(ch.buffered(), 0);

    stop.store(true, Ordering::Relaxed);
}

// ─────────────────────────────────────────────
//  H2: data is delivered even when the peer has already closed
// ─────────────────────────────────────────────
#[test]
fn h2_data_delivered_before_eof_on_early_close() {
    // Peer writes data then immediately closes (FIN). poll() may report
    // POLLHUP|POLLIN together — the drain must complete before the caller
    // acts on hangup, otherwise data is lost. drain_reads drains to EOF
    // before run_session_loop ever checks POLLHUP.
    let (mut ch, _fd, sender) = setup_channel_with_close(20000);

    let (n, eof) = drain_all(&mut ch);
    eprintln!("H2: n={n}, eof={eof}, buffered={}", ch.buffered());
    assert_eq!(n, 20000, "all data must be delivered despite early FIN");
    assert_eq!(ch.buffered(), 0);
    assert!(eof, "EOF reached after the buffered data");

    sender.join().unwrap();
}

// ─────────────────────────────────────────────
//  H3: EOF flag never truncates buffered data
// ─────────────────────────────────────────────
#[test]
fn h3_eof_only_after_all_buffered_data_read() {
    // drain_reads terminates only on read()==Ok(0) — the true end of the
    // channel. A transient EOF condition while bytes remain buffered cannot
    // truncate output, because the drain continues until WouldBlock/Ok(0).
    let (mut ch, _fd, sender) = setup_channel_with_close(20000);

    // One drain call returns all 20000 bytes AND the EOF verdict.
    let (n, eof) = drain_all(&mut ch);
    assert_eq!(n, 20000, "no data lost to an EOF condition");
    assert!(eof);
    assert_eq!(ch.buffered(), 0);

    sender.join().unwrap();
}