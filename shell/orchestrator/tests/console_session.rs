use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use prisma_orchestrator::console_session::{ConsoleSession, ConsoleSessionBuilder};

#[cfg(windows)]
fn shell(script: &str) -> ConsoleSessionBuilder {
    let mut command = ConsoleSession::builder("powershell.exe");
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        script,
    ]);
    command
}

#[cfg(unix)]
fn shell(script: &str) -> ConsoleSessionBuilder {
    let mut command = ConsoleSession::builder("/bin/sh");
    command.args(["-c", script]);
    command
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\r', "")
}

#[test]
fn captures_stdout_stderr_and_exit_status() {
    #[cfg(windows)]
    let script =
        "[Console]::Out.WriteLine('stdout-line'); [Console]::Error.WriteLine('stderr-line'); exit 7";
    #[cfg(unix)]
    let script = "printf 'stdout-line\\n'; printf 'stderr-line\\n' >&2; exit 7";

    let mut session = shell(script).spawn().expect("spawn console child");
    let status = session.wait().expect("wait for console child");

    assert_eq!(status.code(), Some(7));
    assert_eq!(text(&session.stdout_snapshot()), "stdout-line\n");
    assert_eq!(text(&session.stderr_snapshot()), "stderr-line\n");
    assert_eq!(session.status(), Some(status));
}

#[test]
fn writes_stdin_and_closes_it_at_end_of_input() {
    #[cfg(windows)]
    let script = "$prisma_line = [Console]::In.ReadLine(); [Console]::Out.WriteLine(\"received:$prisma_line\")";
    #[cfg(unix)]
    let script = "IFS= read -r prisma_line; printf 'received:%s\\n' \"$prisma_line\"";

    let mut session = shell(script).spawn().expect("spawn console child");
    session
        .write_stdin(b"hello console\n")
        .expect("write console stdin");
    session.close_stdin();
    let status = session.wait().expect("wait for console child");

    assert!(status.success());
    assert_eq!(text(&session.stdout_snapshot()), "received:hello console\n");
    assert!(session.write_stdin(b"late input").is_err());
}

#[test]
fn applies_environment_and_current_directory() {
    let directory = tempfile::tempdir().expect("create working directory");
    #[cfg(windows)]
    let script =
        "[Console]::Out.WriteLine($env:PRISMA_CONSOLE_ENV); [Console]::Out.WriteLine($PWD.Path)";
    #[cfg(unix)]
    let script = "printf '%s\\n' \"$PRISMA_CONSOLE_ENV\"; pwd";

    let mut command = shell(script);
    command
        .env("PRISMA_CONSOLE_ENV", "environment-ok")
        .current_dir(directory.path());
    let mut session = command.spawn().expect("spawn configured child");
    assert!(session.wait().expect("wait for configured child").success());

    let output = text(&session.stdout_snapshot());
    let mut lines = output.lines();
    assert_eq!(lines.next(), Some("environment-ok"));
    let child_directory = lines.next().expect("child prints current directory");
    assert!(same_path(Path::new(child_directory), directory.path()));
}

#[test]
fn stop_reaps_child_and_allows_clean_restart() {
    #[cfg(windows)]
    let script = "$null = [Console]::In.ReadLine(); [Console]::Out.WriteLine('unexpected')";
    #[cfg(unix)]
    let script = "IFS= read -r prisma_line; printf 'unexpected\\n'";

    let mut first = shell(script).spawn().expect("spawn blocking child");
    assert!(first.id().is_some());
    assert_eq!(first.try_wait().expect("poll blocking child"), None);
    let status = first.stop().expect("stop blocking child");
    assert!(!status.success());
    assert_eq!(first.status(), Some(status));
    assert!(first.id().is_none());

    #[cfg(windows)]
    let restart_script = "[Console]::Out.WriteLine('restarted')";
    #[cfg(unix)]
    let restart_script = "printf 'restarted\\n'";
    let mut restarted = shell(restart_script).spawn().expect("restart child");
    assert!(restarted
        .wait()
        .expect("wait for restarted child")
        .success());
    assert_eq!(text(&restarted.stdout_snapshot()), "restarted\n");
}

#[test]
fn drop_terminates_child_before_later_side_effect() {
    let directory = tempfile::tempdir().expect("create marker directory");
    let marker = directory.path().join("child-survived.txt");

    #[cfg(windows)]
    let script = "Start-Sleep -Milliseconds 1000; [IO.File]::WriteAllText($env:PRISMA_DROP_MARKER, 'survived')";
    #[cfg(unix)]
    let script = "sleep 1; printf survived >\"$PRISMA_DROP_MARKER\"";

    {
        let mut command = shell(script);
        command.env("PRISMA_DROP_MARKER", marker.as_os_str());
        let session = command.spawn().expect("spawn child for drop cleanup");
        assert!(session.id().is_some());
    }

    thread::sleep(Duration::from_millis(1_300));
    assert!(!marker.exists(), "child survived ConsoleSession::drop");
}

#[test]
fn drains_output_larger_than_an_os_pipe_buffer() {
    let expected = 512 * 1024;
    #[cfg(windows)]
    let script = "[Console]::Out.Write(('0123456789abcdef' * 32768))";
    #[cfg(unix)]
    let script = "i=0; while [ $i -lt 32768 ]; do printf 0123456789abcdef; i=$((i+1)); done";

    let mut session = shell(script).spawn().expect("spawn verbose child");
    let started = Instant::now();
    assert!(session.wait().expect("wait for verbose child").success());
    assert!(started.elapsed() < Duration::from_secs(20));
    assert_eq!(session.stdout_snapshot().len(), expected);
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).expect("canonicalize child directory");
    let right = fs::canonicalize(right).expect("canonicalize expected directory");
    left == right
}
