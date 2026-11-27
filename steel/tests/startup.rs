//! Startup errors must reach supervisors even when they keep the console open.

use std::{
    fs, io,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[test]
fn extension_startup_failure_exits_with_console_open() -> io::Result<()> {
    let root = tempfile::tempdir()?;
    fs::create_dir(root.path().join("config"))?;
    let defaults = include_str!("../../package-content/config.toml");
    fs::write(
        root.path().join("config/config.toml"),
        format!("[[extensions]]\npath = 'missing-extension.zip'\n\n{defaults}"),
    )?;

    let mut server = Command::new(env!("CARGO_BIN_EXE_steel"))
        .arg("--plain")
        .current_dir(root.path())
        .env_remove("RUST_LOG")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let deadline = Instant::now() + Duration::from_secs(5);
    while server.try_wait()?.is_none() {
        if Instant::now() >= deadline {
            server.kill()?;
            let output = server.wait_with_output()?;
            panic!(
                "Steel did not exit with stdin open:\n{}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
        thread::sleep(Duration::from_millis(10));
    }

    let output = server.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(1), "{stdout}");
    assert!(
        stdout.contains("Server startup failed:")
            && stdout.contains("cannot resolve extension missing-extension.zip"),
        "{stdout}"
    );
    Ok(())
}
