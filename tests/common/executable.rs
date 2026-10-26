use std::{
    io::Write,
    path::Path,
    process::{Child, Command, Stdio},
};

struct ReapedWriter(Child);

impl Drop for ReapedWriter {
    fn drop(&mut self) {
        // On panic or a short write, head may still be waiting for missing bytes.
        // Kill before waiting; an unrelated fork may retain the stdin pipe writer.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn write_executable(path: &Path, content: &str) {
    write_executable_observed(path, content, || {});
}

pub fn write_executable_observed(path: &Path, content: &str, writer_started: impl FnOnce()) {
    // Only the child opens the executable, so concurrent forks cannot inherit it.
    // Read bytes, not EOF: those forks can still retain the stdin pipe writer.
    let mut writer = ReapedWriter(
        Command::new("sh")
            .args(["-c", "exec head -c \"$2\" > \"$1\"", "fixture-writer"])
            .arg(path)
            .arg(content.len().to_string())
            .stdin(Stdio::piped())
            .spawn()
            .expect("start fixture executable writer"),
    );
    writer_started();
    writer
        .0
        .stdin
        .take()
        .expect("fixture writer stdin")
        .write_all(content.as_bytes())
        .expect("send fixture executable contents");
    let status = writer.0.wait().expect("reap fixture executable writer");
    assert!(status.success(), "fixture executable writer failed");
    // The child has closed the file; chmod never opens it for writing.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("make fixture executable");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_is_reaped_when_aborted_before_all_bytes_arrive() {
        let child = Command::new("head")
            .args(["-c", "100"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id() as libc::pid_t;
        drop(ReapedWriter(child));
        let waited = unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) };
        let error = std::io::Error::last_os_error().raw_os_error();
        if waited == 0 {
            // Reap even when the Drop implementation under test is broken.
            unsafe {
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
            }
        }
        assert_eq!(waited, -1);
        assert_eq!(error, Some(libc::ECHILD));
    }

    #[test]
    fn callback_panic_and_failed_write_do_not_strand_the_writer() {
        let dir = tempfile::tempdir().unwrap();
        let callback = std::panic::catch_unwind(|| {
            write_executable_observed(&dir.path().join("callback"), "bytes", || panic!("observer"));
        });
        assert!(callback.is_err());
        let failed_write = std::panic::catch_unwind(|| {
            write_executable(&dir.path().join("missing/script"), &"x".repeat(1_000_000));
        });
        assert!(failed_write.is_err());
    }
}
