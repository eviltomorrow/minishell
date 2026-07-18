use nix::pty::openpty;
use nix::unistd::{fork, ForkResult, setsid, execvp};
use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag};
use std::ffi::CString;
use std::os::unix::io::{RawFd, AsRawFd};

pub struct PtySession {
    pub master_fd: RawFd,
    pub child_pid: nix::unistd::Pid,
}

impl PtySession {
    pub fn spawn(username: &str, term: &str, cols: u16, rows: u16) -> anyhow::Result<Self> {
        let pty = openpty(None, None)?;

        let master_fd = pty.master.as_raw_fd();
        let slave_fd = pty.slave.as_raw_fd();

        // Set initial window size
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master_fd, libc::TIOCSWINSZ, &winsize);
        }

        let child_pid = match unsafe { fork()? } {
            ForkResult::Child => {
                // Create new session
                setsid()?;

                // Set controlling terminal
                unsafe {
                    libc::ioctl(slave_fd, libc::TIOCSCTTY, 0);
                }

                // Redirect stdin/stdout/stderr to slave
                unsafe {
                    libc::dup2(slave_fd, libc::STDIN_FILENO);
                    libc::dup2(slave_fd, libc::STDOUT_FILENO);
                    libc::dup2(slave_fd, libc::STDERR_FILENO);
                }

                // Close original slave fd
                if slave_fd > 2 {
                    unsafe { libc::close(slave_fd); }
                }
                unsafe { libc::close(master_fd); }

                // Set environment
                let home = get_home(username);
                std::env::set_var("HOME", &home);
                std::env::set_var("USER", username);
                std::env::set_var("TERM", term);
                std::env::set_var("SHELL", get_shell());
                std::env::set_var("PATH", "/usr/local/bin:/usr/bin:/bin");

                // Change to home directory
                let _ = std::env::set_current_dir(&home);

                // Exec shell
                let shell = get_shell();
                let shell_cstr = CString::new(shell.clone())?;
                let arg = CString::new("-i")?;
                execvp(&shell_cstr, &[shell_cstr.clone(), arg])?;

                // execvp doesn't return on success
                std::process::exit(1);
            }
            ForkResult::Parent { child } => {
                // Close slave fd in parent
                drop(pty.slave);
                child
            }
        };

        Ok(PtySession {
            master_fd,
            child_pid,
        })
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(self.master_fd, libc::TIOCSWINSZ, &winsize);
        }
    }

    pub fn is_alive(&self) -> bool {
        match waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG)) {
            Ok(_) => false,
            Err(nix::errno::Errno::ECHILD) => false,
            Err(_) => true,
        }
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = signal::kill(self.child_pid, Signal::SIGHUP);
        let _ = waitpid(self.child_pid, Some(WaitPidFlag::WNOHANG));
        unsafe {
            libc::close(self.master_fd);
        }
    }
}

fn get_shell() -> String {
    if let Ok(shell) = std::env::var("SHELL") {
        if std::path::Path::new(&shell).exists() {
            return shell;
        }
    }
    if std::path::Path::new("/bin/bash").exists() {
        return "/bin/bash".to_string();
    }
    "/bin/sh".to_string()
}

fn get_home(username: &str) -> String {
    if username == "root" {
        return "/root".to_string();
    }
    format!("/home/{}", username)
}
