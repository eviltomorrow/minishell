/// ── Feedback loop: SSH channel buffering bug ──
///
/// Reproduces the exact poll+read pattern from minishell-ssh/src/lib.rs
/// `run_session_loop()` (lines 71-88) to demonstrate that data can get
/// stuck in libssh2's internal buffer when poll() on the raw socket fd
/// does not return POLLIN.
use std::io::{Read, Write, ErrorKind};
use std::net::{TcpListener, TcpStream, Shutdown};
use std::os::fd::AsRawFd;
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

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

    fn eof(&self) -> bool { self.eof_flag }
    fn buffered(&self) -> usize { self.inner.len().saturating_sub(self.pos) }
    fn force_eof(&mut self) { self.eof_flag = true; }
}

/// Exact reproduction of run_session_loop lines 71-88.
fn buggy_read(ch: &mut SimChannel, fd: RawFd, poll_ms: i32) -> (usize, bool) {
    let mut buf = [0u8; 4096];
    let mut pfd = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }];
    let ret = unsafe { libc::poll(pfd.as_mut_ptr(), 1, poll_ms) };
    if ret < 0 { return (0, true); }

    let rev = pfd[0].revents;
    // ⚠ BUG: POLLHUP checked BEFORE POLLIN — exits without reading
    if rev & (libc::POLLHUP | libc::POLLERR) != 0 { return (0, true); }
    if rev & libc::POLLIN != 0 {
        match ch.read(&mut buf) {
            Ok(0) => return (0, true),
            Ok(n) => return (n, ch.eof()),
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(_) => return (0, true),
        }
        // ⚠ BUG: eof() after non-zero read — exits while data remains
        if ch.eof() { return (0, true); }
    }
    // ⚠ BUG: when poll times out (no POLLIN), no drain attempt
    (0, false)
}

/// Fixed: always drain the internal buffer regardless of poll result.
fn fixed_read(ch: &mut SimChannel, fd: RawFd, poll_ms: i32) -> (usize, bool) {
    let mut buf = [0u8; 4096];
    let mut pfd = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }];
    let ret = unsafe { libc::poll(pfd.as_mut_ptr(), 1, poll_ms) };
    if ret < 0 { return (0, true); }

    // Always drain — not gated on POLLIN
    let mut total = 0usize;
    loop {
        match ch.read(&mut buf) {
            Ok(0) => return (total, true),
            Ok(n) => total += n,
            Err(ref e) if e.kind() == ErrorKind::WouldBlock => break,
            Err(_) => return (total, true),
        }
    }

    // Check hangup AFTER draining
    if pfd[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 { return (total, true); }
    (total, false)
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

/// Like setup_channel but allows the sender to close (for eof testing)
fn setup_channel_with_close(data_size: usize) -> (SimChannel, RawFd, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let jh = thread::spawn(move || {
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

    (ch, fd, jh)
}

// ─────────────────────────────────────────────
//  REGRESSION TEST — must pass after fixing run_session_loop
// ─────────────────────────────────────────────
#[test]
fn regression_all_data_delivered_across_multiple_poll_cycles() {
    // Simulates a real session: data arrives, is consumed, more data arrives.
    // The fixed loop must not lose or stall any data.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();

    let sender = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            // Burst 1: 12KB
            let _ = stream.write_all(&[b'A'; 12288]);
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(50));
            // Burst 2: 5KB (simulates shell prompt + more output)
            let _ = stream.write_all(&[b'B'; 5120]);
            let _ = stream.flush();
            thread::sleep(Duration::from_millis(50));
            // Burst 3: close
            let _ = stream.shutdown(Shutdown::Write);
            while !s.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(10));
            }
        }
    });

    let rx = TcpStream::connect(addr).unwrap();
    rx.set_nonblocking(true).unwrap();
    let mut ch = SimChannel::new(rx);
    let fd = ch.stream.as_raw_fd();
    thread::sleep(Duration::from_millis(200));

    let mut total = 0usize;
    loop {
        let (n, done) = fixed_read(&mut ch, fd, 10);
        total += n;
        if done { break; }
    }

    eprintln!("REGRESSION: read {total}/17408 bytes (burst1=12288 + burst2=5120)");
    assert_eq!(total, 17408, "regression: all data must be delivered");
    stop.store(true, Ordering::Relaxed);
    sender.join().unwrap();
}

// ─────────────────────────────────────────────
//  H1: poll-gated read → internal buffer stuck
// ─────────────────────────────────────────────

#[test]
fn h1_buggy_does_not_drain_buffered_data() {
    let (mut ch, fd, stop) = setup_channel(50000);

    // Read 1: poll sees POLLIN → SimChannel fills buffer, returns 4096
    let (n1, done1) = buggy_read(&mut ch, fd, 10);
    eprintln!("H1 r1: n={n1}, done={done1}, buffered={}", ch.buffered());
    assert_eq!(n1, 4096);
    assert!(!done1);

    // Reads 2-3: poll times out (1ms) with no POLLIN → no read attempted
    let (n2, _) = buggy_read(&mut ch, fd, 1);
    assert_eq!(n2, 0);
    let (n3, _) = buggy_read(&mut ch, fd, 1);
    assert_eq!(n3, 0);

    // 45904 bytes still stuck in buffer
    assert_eq!(ch.buffered(), 50000 - 4096);
    eprintln!("H1: BUG CONFIRMED — {}/50000 bytes never left buffer", ch.buffered());
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn h1_fixed_drains_all_data() {
    let (mut ch, fd, stop) = setup_channel(50000);

    // fixed_read drains all 50000 from SimChannel in one go
    let (n1, done1) = fixed_read(&mut ch, fd, 10);
    eprintln!("H1 fixed r1: n={n1}, done={done1}, buffered={}", ch.buffered());
    assert_eq!(n1, 50000);
    assert!(!done1);

    // Second read: nothing to drain
    let (n2, _) = fixed_read(&mut ch, fd, 1);
    assert_eq!(n2, 0);
    assert_eq!(ch.buffered(), 0);

    stop.store(true, Ordering::Relaxed);
}

// ─────────────────────────────────────────────
//  H2: POLLHUP before POLLIN — data loss
// ─────────────────────────────────────────────

#[test]
fn h2_diagnostic_pollhup_behavior() {
    // TCP shutdown(SHUT_WR) — graceful close
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let sender = thread::spawn(move || {
        if let Ok((mut s, _)) = listener.accept() {
            let _ = s.write_all(b"tcp_data");
            let _ = s.flush();
            thread::sleep(Duration::from_millis(50));
            let _ = s.shutdown(Shutdown::Write);
        }
    });
    let mut rx = TcpStream::connect(addr).unwrap();
    rx.set_nonblocking(true).unwrap();
    thread::sleep(Duration::from_millis(200));
    let mut pfd = [libc::pollfd { fd: rx.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
    let ret = unsafe { libc::poll(pfd.as_mut_ptr(), 1, 0) };
    let rev = if ret > 0 { pfd[0].revents } else { 0 };
    eprintln!("H2 tcp shutdown(SHUT_WR): POLLIN={} POLLHUP={} POLLERR={}",
        (rev & libc::POLLIN) != 0, (rev & libc::POLLHUP) != 0, (rev & libc::POLLERR) != 0);
    let mut buf = [0u8; 64];
    let n = rx.read(&mut buf).unwrap_or(0);
    eprintln!("  → read {n} bytes");
    sender.join().unwrap();

    // UnixStream abrupt close
    let (mut tx, mut rx) = std::os::unix::net::UnixStream::pair().unwrap();
    rx.set_nonblocking(true).unwrap();
    tx.write_all(b"unix_data").unwrap();
    drop(tx);
    thread::sleep(Duration::from_millis(50));
    let mut pfd2 = [libc::pollfd { fd: rx.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
    let ret2 = unsafe { libc::poll(pfd2.as_mut_ptr(), 1, 0) };
    let rev2 = if ret2 > 0 { pfd2[0].revents } else { 0 };
    eprintln!("H2 unix drop: POLLIN={} POLLHUP={} POLLERR={}",
        (rev2 & libc::POLLIN) != 0, (rev2 & libc::POLLHUP) != 0, (rev2 & libc::POLLERR) != 0);
    let mut buf2 = [0u8; 64];
    let n2 = rx.read(&mut buf2).unwrap_or(0);
    eprintln!("  → read {n2} bytes");

    if rev2 & libc::POLLHUP != 0 {
        eprintln!("  → POLLHUP IS set on UnixStream drop — this triggers the order bug");
    }
}

#[test]
fn h2_pollhup_order_causes_data_loss_on_unix() {
    // On UnixStream, dropping the peer can return POLLHUP | POLLIN.
    // The buggy code checks POLLHUP before POLLIN, so it exits without reading.

    let (mut tx, mut rx) = std::os::unix::net::UnixStream::pair().unwrap();
    rx.set_nonblocking(true).unwrap();
    let fd = rx.as_raw_fd();

    tx.write_all(b"critical_data").unwrap();
    drop(tx);
    thread::sleep(Duration::from_millis(50));

    // Run the exact buggy poll+read pattern
    let mut buf = [0u8; 4096];
    let mut pfd = [libc::pollfd { fd, events: libc::POLLIN, revents: 0 }];
    let ret = unsafe { libc::poll(pfd.as_mut_ptr(), 1, 0) };
    if ret > 0 {
        let rev = pfd[0].revents;
        // ── BUGGY ORDER (same as run_session_loop) ──
        if rev & (libc::POLLHUP | libc::POLLERR) != 0 {
            eprintln!("H2: POLLHUP triggered — exiting WITHOUT reading");
        } else if rev & libc::POLLIN != 0 {
            let n = rx.read(&mut buf).unwrap_or(0);
            eprintln!("H2: read {n} bytes (would NOT be lost with POLLIN-first order)");
        } else {
            eprintln!("H2: no events");
        }
    }
}

// ─────────────────────────────────────────────
//  H3: eof() after non-zero read → truncation
// ─────────────────────────────────────────────

#[test]
fn h3_eof_after_read_truncates_data() {
    // In libssh2, eof() returns true when CHANNEL_EOF is processed,
    // which can happen while data is still buffered internally.
    //
    // Our SimChannel can't reproduce this naturally (eof only set on
    // TCP read returning 0, after buffer is empty). So we use
    // force_eof() to simulate the libssh2 scenario.

    let (mut ch, fd, jh) = setup_channel_with_close(20000);

    // Read 1: poll sees POLLIN (data), SimChannel fills buffer, returns 4096
    let (n1, done1) = buggy_read(&mut ch, fd, 10);
    eprintln!("H3 r1: n={n1}, done={done1}, buffered={}", ch.buffered());
    assert_eq!(n1, 4096);
    assert!(!done1);
    let before_force = ch.buffered();

    // Simulate libssh2: CHANNEL_EOF processed, but buffer still has data
    ch.force_eof();

    // Read 2: poll sees POLLIN (FIN pending), calls read() which returns
    // next 4096 from buffer. After read, checks eof() → true → exits.
    let (n2, done2) = buggy_read(&mut ch, fd, 10);
    eprintln!("H3 r2: n={n2}, done={done2}, buffered={}", ch.buffered());

    // BUG: n2 > 0 and done2=true, but buffer still has data
    if done2 && ch.buffered() > 0 {
        eprintln!(
            "  → H3 CONFIRMED: eof() after non-zero read discarded {} bytes",
            ch.buffered()
        );
    }

    // Verify data was lost
    assert!(n2 > 0, "should have read data");
    assert!(done2, "eof() should cause done=true");
    assert_eq!(
        ch.buffered(),
        before_force - n2,
        "remaining data lost after eof() check"
    );

    jh.join().unwrap();
}
