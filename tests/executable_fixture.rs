mod common;

use std::{
    io::Write,
    os::{fd::AsRawFd, unix::fs::PermissionsExt, unix::net::UnixStream},
    process::Command,
    time::Duration,
};

#[test]
fn executable_fixture_survives_a_child_holding_inherited_parent_descriptors() {
    let f = common::Fixture::new();
    let script = f.path("script 'with spaces'.sh");
    let (mut release, inherited) = UnixStream::pair().unwrap();
    inherited
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut child_pid = 0;
    let content = "#!/bin/sh\nprintf fixture-ok\n# 雪\n";
    common::executable::write_executable_observed(&script, content, || {
        // The child deliberately retains every descriptor open at the write seam.
        // Only async-signal-safe syscalls run in the forked child.
        let pid = unsafe { libc::fork() };
        assert!(pid >= 0, "fork failed: {}", std::io::Error::last_os_error());
        if pid == 0 {
            let mut byte = 0u8;
            unsafe {
                let received = libc::read(inherited.as_raw_fd(), (&mut byte as *mut u8).cast(), 1);
                libc::_exit(if received == 1 { 0 } else { 1 });
            }
        }
        child_pid = pid;
    });

    let executed = Command::new(&script).output();
    // Always release and reap the child before asserting execution succeeded.
    release.write_all(&[1]).unwrap();
    let mut status = 0;
    assert_eq!(
        unsafe { libc::waitpid(child_pid, &mut status, 0) },
        child_pid
    );
    assert_eq!(status, 0);
    let output = executed.expect("inherited descriptors must not keep the fixture writable");
    assert!(output.status.success());
    assert_eq!(output.stdout, b"fixture-ok");
    assert_eq!(std::fs::read_to_string(&script).unwrap(), content);
    assert_eq!(
        std::fs::metadata(script).unwrap().permissions().mode() & 0o777,
        0o755
    );
}
