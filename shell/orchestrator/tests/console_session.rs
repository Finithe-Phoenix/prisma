use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use prisma_orchestrator::console_session::{
    ConsoleResizeError, ConsoleSession, ConsoleSessionBuilder,
};

static TEST_LOCK: Mutex<()> = Mutex::new(());

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
    let _guard = serial_guard();
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
    let _guard = serial_guard();
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
    let _guard = serial_guard();
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
    let _guard = serial_guard();
    #[cfg(windows)]
    let script = "$null = [Console]::In.ReadLine(); [Console]::Out.WriteLine('unexpected')";
    #[cfg(unix)]
    let script = "IFS= read -r prisma_line; printf 'unexpected\\n'";

    let mut first = shell(script).spawn().expect("spawn blocking child");
    let first_pid = first.id().expect("blocking child has a process id");
    assert_eq!(first.try_wait().expect("poll blocking child"), None);
    let status = first.stop().expect("stop blocking child");
    assert!(!status.success());
    assert_eq!(first.status(), Some(status));
    assert!(first.id().is_none());
    assert_process_stopped(first_pid);

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
    let _guard = serial_guard();
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
    let _guard = serial_guard();
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

#[test]
fn preserves_utf8_and_ansi_bytes_across_stdin_and_stdout() {
    let _guard = serial_guard();
    #[cfg(windows)]
    let script = "[Console]::InputEncoding = [Text.UTF8Encoding]::new($false); $text = [Console]::In.ReadToEnd(); $bytes = [Text.Encoding]::UTF8.GetBytes($text); $output = [Console]::OpenStandardOutput(); $output.Write($bytes, 0, $bytes.Length); $output.Flush()";
    #[cfg(unix)]
    let script = "cat";
    let expected = "\u{1b}[38;5;45mPrisma\u{1b}[0m · café · 日本語\n".as_bytes();

    let mut session = shell(script).spawn().expect("spawn raw byte echo child");
    session
        .write_stdin(expected)
        .expect("write UTF-8 and ANSI bytes");
    session.close_stdin();
    assert!(session.wait().expect("wait for raw byte echo").success());

    assert_eq!(session.stdout_snapshot(), expected);
    assert!(session.stderr_snapshot().is_empty());
}

#[test]
fn pipe_transport_reports_resize_as_unsupported() {
    let _guard = serial_guard();
    #[cfg(windows)]
    let script = "$null = [Console]::In.ReadLine()";
    #[cfg(unix)]
    let script = "IFS= read -r prisma_line";

    let mut session = shell(script).spawn().expect("spawn pipe-backed child");
    assert_eq!(
        session.resize(120, 40),
        Err(ConsoleResizeError::Unsupported)
    );
    assert!(session
        .resize(u16::MAX, u16::MAX)
        .expect_err("ordinary pipes never claim resize support")
        .to_string()
        .contains("PTY"));
    session.stop().expect("stop resize test child");
}

#[cfg(windows)]
#[test]
fn repeated_restart_isolates_output_and_leaks_no_process_or_handle() {
    let _guard = serial_guard();
    run_short_session("warm-up");
    let handles_before = windows::current_process_handle_count();

    for generation in 0..16 {
        let expected = format!("generation-{generation}");
        run_short_session(&expected);
    }

    let handles_after = windows::current_process_handle_count();
    assert!(
        handles_after <= handles_before,
        "console restart leaked handles: before={handles_before}, after={handles_after}"
    );
}

#[cfg(windows)]
fn run_short_session(expected: &str) {
    let script = "[Console]::Out.Write($env:PRISMA_RESTART_TOKEN)";
    let mut command = shell(script);
    command.env("PRISMA_RESTART_TOKEN", expected);
    let mut session = command.spawn().expect("spawn isolated restart child");
    let pid = session.id().expect("restart child has a process id");
    assert!(session.wait().expect("wait for restart child").success());
    assert_eq!(session.stdout_snapshot(), expected.as_bytes());
    assert!(session.stderr_snapshot().is_empty());
    assert_process_stopped(pid);
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = fs::canonicalize(left).expect("canonicalize child directory");
    let right = fs::canonicalize(right).expect("canonicalize expected directory");
    left == right
}

fn serial_guard() -> MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn assert_process_stopped(pid: u32) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while process_is_running(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !process_is_running(pid),
        "child process {pid} is still live"
    );
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    windows::process_is_running(pid)
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
mod windows {
    use std::ffi::c_void;

    type Handle = *mut c_void;

    const SYNCHRONIZE: u32 = 0x0010_0000;
    const WAIT_TIMEOUT: u32 = 0x0000_0102;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        #[link_name = "CloseHandle"]
        fn close_handle(handle: Handle) -> i32;
        #[link_name = "GetCurrentProcess"]
        fn get_current_process() -> Handle;
        #[link_name = "GetProcessHandleCount"]
        fn get_process_handle_count(process: Handle, count: *mut u32) -> i32;
        #[link_name = "OpenProcess"]
        fn open_process(access: u32, inherit_handle: i32, process_id: u32) -> Handle;
        #[link_name = "WaitForSingleObject"]
        fn wait_for_single_object(handle: Handle, milliseconds: u32) -> u32;
    }

    pub fn current_process_handle_count() -> u32 {
        let mut count = 0;
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle and `count` is writable.
        let succeeded = unsafe { get_process_handle_count(get_current_process(), &raw mut count) };
        assert_ne!(succeeded, 0, "GetProcessHandleCount failed");
        count
    }

    pub fn process_is_running(pid: u32) -> bool {
        // SAFETY: no pointer is passed in and the returned handle is checked before use.
        let process = unsafe { open_process(SYNCHRONIZE, 0, pid) };
        if process.is_null() {
            return false;
        }
        // SAFETY: `process` is a live handle returned by OpenProcess.
        let wait_result = unsafe { wait_for_single_object(process, 0) };
        // SAFETY: this function owns the handle returned by OpenProcess.
        let _ = unsafe { close_handle(process) };
        wait_result == WAIT_TIMEOUT
    }
}
