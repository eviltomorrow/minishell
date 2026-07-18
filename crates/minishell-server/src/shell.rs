use nix::sys::signal::{self, Signal};
use nix::sys::wait::{waitpid, WaitPidFlag};
use std::ffi::CString;
use std::os::unix::io::RawFd;

pub struct PtySession {
    pub master_fd: RawFd,
    pub child_pid: nix::unistd::Pid,
}

impl PtySession {
    pub fn spawn(username: &str, term: &str, cols: u16, rows: u16) -> anyhow::Result<Self> {
        let shell = get_shell();
        let shell_cstr = CString::new(shell.clone())?;
        let arg_i = CString::new("-i")?;

        let winsize = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };

        let mut master: libc::c_int = 0;

        let pid = unsafe {
            libc::forkpty(
                &mut master as *mut libc::c_int,
                std::ptr::null_mut(),
                std::ptr::null(),
                &winsize as *const libc::winsize,
            )
        };

        if pid < 0 {
            anyhow::bail!("forkpty failed: {}", std::io::Error::last_os_error());
        }

        if pid == 0 {
            // Child
            let home = get_home(username);
            std::env::set_var("HOME", &home);
            std::env::set_var("USER", username);
            std::env::set_var("TERM", term);
            std::env::set_var("SHELL", &shell);
            std::env::set_var("PATH", "/usr/local/bin:/usr/bin:/bin");
            let _ = std::env::set_current_dir(&home);

            let argv = [shell_cstr.as_ptr(), arg_i.as_ptr(), std::ptr::null()];
            unsafe {
                libc::execvp(shell_cstr.as_ptr(), argv.as_ptr());
                // If execvp returns, it failed
                std::process::exit(1);
            }
        }

        // Parent
        Ok(PtySession {
            master_fd: master,
            child_pid: nix::unistd::Pid::from_raw(pid),
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
