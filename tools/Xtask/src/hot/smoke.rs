//! `xtask hot --smoke`: an end-to-end proof that Rust hot-patching works.
//!
//! 1. Start the game under `dx serve --hot-patch` (non-interactive) and wait
//!    for `hot probe: alpha`, logged by reloadable setup at startup.
//! 2. Rewrite the probe string in `app/src/startup/hot_probe.rs`.
//! 3. Wait for the patched value in the log. It can only appear if dx rebuilt
//!    the patch, the running game loaded it, Bevy reported `HotPatched`, and
//!    World_Reload re-ran setup with the new code.
//! 4. Restore the file (also on failure or Ctrl-C-free early exit) and stop
//!    the game. Everything dx printed is kept in `target/hot-smoke.log`.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::dx_serve;

const PROBE_FILE: &str = "app/src/startup/hot_probe.rs";
const PROBE_LINE: &str = "    \"alpha\"";
/// The first build compiles Bevy without dynamic linking: slow the first time.
const FIRST_BUILD: Duration = Duration::from_secs(40 * 60);
const PATCH: Duration = Duration::from_secs(5 * 60);

pub fn run(root: &Path, args: &[String]) -> Result<(), String> {
    let probe = Restore::capture(root.join(PROBE_FILE))?;
    if !probe.original.contains(PROBE_LINE) {
        return Err(format!(
            "{PROBE_FILE} no longer contains {PROBE_LINE:?}; update the smoke test"
        ));
    }

    let log_path = root.join("target/hot-smoke.log");
    let mut log =
        std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let mut command = dx_serve(root, args, false);
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    println!("$ {:?}", command);
    let mut child = Running(
        command
            .spawn()
            .map_err(|e| format!("could not start dx: {e}"))?,
    );
    let lines = pipe_lines(&mut child.0);

    let started = Instant::now();
    println!("waiting for the game to start under dx (the first build is slow)...");
    wait_for(&lines, &mut log, FIRST_BUILD, |l| {
        l.contains("hot probe: alpha")
    })
    .map_err(|e| failure(&e, &log_path))?;
    let startup = started.elapsed();
    println!("game running after {:.0?}; patching the probe", startup);

    let token = format!(
        "beta{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
            % 1_000_000
    );
    let patched = probe
        .original
        .replace(PROBE_LINE, &format!("    \"{token}\""));
    std::fs::write(&probe.path, patched).map_err(|e| format!("could not edit the probe: {e}"))?;
    let edited = Instant::now();

    let want = format!("hot probe: {token}");
    let mut saw_reload = false;
    wait_for(&lines, &mut log, PATCH, |l| {
        saw_reload |= l.contains("code reload #");
        l.contains(&want)
    })
    .map_err(|e| failure(&e, &log_path))?;
    let latency = edited.elapsed();

    drop(child);
    drop(probe);
    if !saw_reload {
        return Err(failure(
            "the probe changed but no `code reload #` line was logged",
            &log_path,
        ));
    }
    println!(
        "hot-patch smoke test passed: startup {startup:.1?}, edit to patched code {latency:.1?}\nlog: {}",
        log_path.display()
    );
    Ok(())
}

fn failure(reason: &str, log: &Path) -> String {
    let tail = std::fs::read_to_string(log)
        .map(|t| {
            let lines: Vec<_> = t.lines().collect();
            lines[lines.len().saturating_sub(25)..].join("\n")
        })
        .unwrap_or_default();
    format!(
        "hot-patch smoke test failed: {reason}\n--- last lines of {} ---\n{tail}",
        log.display()
    )
}

/// Every line dx and the game print, from both pipes.
fn pipe_lines(child: &mut Child) -> Receiver<String> {
    let (sender, receiver) = channel();
    let out = child.stdout.take().expect("stdout is piped");
    let err = child.stderr.take().expect("stderr is piped");
    for pipe in [
        Box::new(out) as Box<dyn std::io::Read + Send>,
        Box::new(err),
    ] {
        let sender = sender.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
    }
    receiver
}

fn wait_for(
    lines: &Receiver<String>,
    log: &mut std::fs::File,
    timeout: Duration,
    mut done: impl FnMut(&str) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(line) => {
                let _ = writeln!(log, "{line}");
                if done(&line) {
                    return Ok(());
                }
                // dx keeps serving after a failed build; nothing more will come.
                if line.contains("Build failed") {
                    return Err(format!("dx reported a failed build: {}", line.trim()));
                }
            }
            Err(RecvTimeoutError::Timeout) => return Err(format!("timed out after {timeout:?}")),
            Err(RecvTimeoutError::Disconnected) => return Err("dx exited".into()),
        }
    }
}

/// Puts the probe file back exactly as it was, however the test ends.
struct Restore {
    path: PathBuf,
    original: String,
}

impl Restore {
    fn capture(path: PathBuf) -> Result<Self, String> {
        let original =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { path, original })
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        if let Err(e) = std::fs::write(&self.path, &self.original) {
            eprintln!("could not restore {}: {e}", self.path.display());
        }
    }
}

/// Stops dx and the game it launched when dropped.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        if cfg!(windows) {
            // dx starts the game as a child process; take the whole tree down.
            let _ = std::process::Command::new("taskkill")
                .args(["/PID", &self.0.id().to_string(), "/T", "/F"])
                .output();
        }
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
