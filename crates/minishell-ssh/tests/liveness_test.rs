/// ── SSH session liveness + non-blocking write regression tests ──
///
/// The reported bug: after the laptop suspends and wakes, an SSH session whose
/// network path is silently black-holed leaves the TUI frozen in the shell
/// forever. No `POLLHUP`/`POLLERR` ever arrives, keystrokes are accepted
/// locally and never delivered, and no reconnect prompt appears.
///
/// The detection fix lives in the kernel: `configure_liveness` arms
/// `TCP_USER_TIMEOUT` plus keepalive tunables so the socket errors out in under
/// a minute instead of the ~15-minute `tcp_retries2` / 2-hour `SO_KEEPALIVE`
/// defaults.
///
/// A genuinely black-holed peer cannot be produced in-process — it needs the
/// network path to drop packets (root/iptables, or a real suspend), so the
/// end-to-end symptom has **no automated seam here**. These tests lock down the
/// two halves that do have one:
///   1. the socket options the detection depends on are actually applied;
///   2. keystrokes survive a partially-full non-blocking send buffer — the
///      other reason "pressing Enter does nothing".
use std::io::{ErrorKind, Write};
use std::net::{TcpListener, TcpStream};
use std::os::fd::AsRawFd;

use minishell_ssh::{configure_liveness, flush_pending};

fn getsockopt_int(fd: libc::c_int, level: libc::c_int, name: libc::c_int) -> libc::c_int {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            level,
            name,
            &mut value as *mut libc::c_int as *mut libc::c_void,
            &mut len,
        )
    };
    assert_eq!(rc, 0, "getsockopt(level={level}, name={name}) failed");
    value
}

#[test]
fn configure_liveness_arms_dead_peer_detection() {
    // TCP options are per-connection state, so the socket must be connected.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tcp = TcpStream::connect(listener.local_addr().unwrap()).unwrap();

    configure_liveness(&tcp);

    let fd = tcp.as_raw_fd();
    assert_eq!(getsockopt_int(fd, libc::SOL_SOCKET, libc::SO_KEEPALIVE), 1);

    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        assert_eq!(
            getsockopt_int(fd, libc::IPPROTO_TCP, libc::TCP_USER_TIMEOUT),
            minishell_ssh::SOCK_USER_TIMEOUT_MS as libc::c_int,
        );
        assert_eq!(
            getsockopt_int(fd, libc::IPPROTO_TCP, libc::TCP_KEEPIDLE),
            minishell_ssh::SOCK_KEEPALIVE_IDLE_SECS as libc::c_int,
        );
        assert_eq!(
            getsockopt_int(fd, libc::IPPROTO_TCP, libc::TCP_KEEPINTVL),
            minishell_ssh::SOCK_KEEPALIVE_INTVL_SECS as libc::c_int,
        );
        assert_eq!(
            getsockopt_int(fd, libc::IPPROTO_TCP, libc::TCP_KEEPCNT),
            minishell_ssh::SOCK_KEEPALIVE_CNT as libc::c_int,
        );

        // The point of the whole change: a dead peer must be detected in well
        // under the multi-minute kernel defaults, not eventually.
        let worst_case_secs = minishell_ssh::SOCK_USER_TIMEOUT_MS / 1000
            + minishell_ssh::SOCK_KEEPALIVE_IDLE_SECS
            + minishell_ssh::SOCK_KEEPALIVE_INTVL_SECS * minishell_ssh::SOCK_KEEPALIVE_CNT;
        assert!(
            worst_case_secs <= 90,
            "dead-peer detection must be bounded under 90s, configured bound is {worst_case_secs}s",
        );
    }
}

/// Accepts at most `budget` bytes, then reports `WouldBlock` — the shape of
/// libssh2's send buffer filling up part-way through a write.
struct BudgetWriter {
    accepted: Vec<u8>,
    budget: usize,
}

impl BudgetWriter {
    fn new(budget: usize) -> Self {
        Self { accepted: Vec::new(), budget }
    }
}

impl Write for BudgetWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.budget == 0 {
            return Err(std::io::Error::new(ErrorKind::WouldBlock, "send buffer full"));
        }
        let n = buf.len().min(self.budget);
        self.accepted.extend_from_slice(&buf[..n]);
        self.budget -= n;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn flush_pending_keeps_every_byte_across_would_block() {
    let typed = b"ssh root@10.0.0.1\r\n";
    let mut pending = typed.to_vec();

    // Pass 1: the channel takes 5 bytes, then WouldBlock.
    let mut writer = BudgetWriter::new(5);
    flush_pending(&mut writer, &mut pending).unwrap();
    assert_eq!(writer.accepted, typed[..5].to_vec());
    assert_eq!(
        pending,
        typed[5..].to_vec(),
        "bytes the channel refused must be retained, not dropped",
    );

    // Pass 2: still full — no progress, and crucially still no loss.
    flush_pending(&mut writer, &mut pending).unwrap();
    assert_eq!(pending, typed[5..].to_vec());

    // Pass 3: buffer drains — everything left goes out exactly once.
    writer.budget = usize::MAX;
    flush_pending(&mut writer, &mut pending).unwrap();
    assert!(pending.is_empty());
    assert_eq!(writer.accepted, typed.to_vec());
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(ErrorKind::BrokenPipe, "peer gone"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn flush_pending_propagates_transport_errors() {
    let mut pending = b"x".to_vec();
    let err = flush_pending(&mut FailingWriter, &mut pending).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::BrokenPipe);
    assert_eq!(pending, b"x".to_vec(), "nothing was written, so nothing may be discarded");
}

struct ClosedWriter;

impl Write for ClosedWriter {
    fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
        Ok(0)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn flush_pending_treats_zero_write_as_closed_channel() {
    let mut pending = b"x".to_vec();
    let err = flush_pending(&mut ClosedWriter, &mut pending).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::WriteZero);
}
