//! Subprocess execution without a shell. Every argument is passed verbatim.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use super::error::{BridgeError, Result};
use super::platform::Env;
use super::text::tail;

pub const TIMEOUT_MESSAGE: &str = "操作超时；发送结果可能未知，请查看原终端，不要直接重发。";

#[derive(Clone, Debug)]
pub struct RunRequest {
    pub argv: Vec<String>,
    pub input: Option<String>,
    pub timeout: Duration,
    pub cwd: Option<PathBuf>,
}

impl RunRequest {
    pub fn new(argv: Vec<String>, timeout_secs: u64) -> Self {
        RunRequest { argv, input: None, timeout: Duration::from_secs(timeout_secs), cwd: None }
    }
}

/// Runs a request and returns stdout. Swappable so tests never start real CLIs.
pub type Runner = Arc<dyn Fn(&RunRequest) -> Result<String> + Send + Sync>;

pub fn system_runner() -> Runner {
    Arc::new(run)
}

/// A command with the GUI-safe PATH and, on Windows, no console window.
pub fn command(argv: &[String]) -> Command {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    if let Some(path) = Env::process().child_path() {
        command.env("PATH", path);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

struct Capture {
    buffer: Arc<Mutex<Vec<u8>>>,
    done: mpsc::Receiver<()>,
}

fn collect(pipe: Option<impl Read + Send + 'static>) -> Capture {
    let buffer = Arc::new(Mutex::new(Vec::new()));
    let (sender, done) = mpsc::channel();
    if let Some(mut pipe) = pipe {
        let buffer = buffer.clone();
        thread::spawn(move || {
            let mut chunk = [0u8; 8192];
            while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                buffer.lock().unwrap().extend_from_slice(&chunk[..n]);
            }
            let _ = sender.send(());
        });
    }
    Capture { buffer, done }
}

pub fn run(request: &RunRequest) -> Result<String> {
    let program = request.argv.first().ok_or_else(|| BridgeError::new("命令为空。"))?;
    let mut command = command(&request.argv);
    command
        .stdin(if request.input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|e| BridgeError::new(format!("无法启动 {program}：{e}")))?;
    if let (Some(input), Some(mut stdin)) = (request.input.clone(), child.stdin.take()) {
        thread::spawn(move || {
            let _ = stdin.write_all(input.as_bytes());
        });
    }
    let stdout = collect(child.stdout.take());
    let stderr = collect(child.stderr.take());
    let deadline = Instant::now() + request.timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(BridgeError::new(TIMEOUT_MESSAGE));
            }
            Ok(None) => thread::sleep(Duration::from_millis(15)),
            Err(e) => return Err(BridgeError::new(format!("{program}：{e}"))),
        }
    };
    // A helper that inherited the pipes may keep them open after exit: keep
    // what was written and stop waiting shortly after the process itself ends.
    let read = |capture: Capture| {
        let left = deadline.saturating_duration_since(Instant::now());
        let _ = capture.done.recv_timeout(left.min(Duration::from_secs(2)).max(Duration::from_millis(200)));
        String::from_utf8_lossy(&capture.buffer.lock().unwrap()).into_owned()
    };
    let (stdout, stderr) = (read(stdout), read(stderr));
    if !status.success() {
        let message =
            [&stderr, &stdout].into_iter().find(|text| !text.is_empty()).map_or("命令失败", |text| text.as_str());
        return Err(BridgeError::new(tail(message, 3000)));
    }
    Ok(stdout)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh(script: &str) -> RunRequest {
        RunRequest::new(vec!["/bin/sh".into(), "-c".into(), script.into()], 5)
    }

    #[test]
    fn captures_output_input_and_failures() {
        assert_eq!(run(&sh("printf 'hello\\n世界'")).unwrap(), "hello\n世界");
        let mut echo = sh("cat");
        echo.input = Some("$(whoami) `cmd`".into());
        assert_eq!(run(&echo).unwrap(), "$(whoami) `cmd`");
        assert_eq!(run(&sh("echo bad >&2; exit 3")).unwrap_err().message(), "bad\n");
        assert_eq!(run(&sh("exit 4")).unwrap_err().message(), "命令失败");
    }

    #[test]
    fn timeout_kills_and_reports_unknown_result() {
        let mut slow = sh("sleep 5");
        slow.timeout = Duration::from_millis(100);
        let started = Instant::now();
        assert_eq!(run(&slow).unwrap_err().message(), TIMEOUT_MESSAGE);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn inherited_pipes_do_not_block_after_exit() {
        let mut request = sh("(sleep 5 &) ; echo done");
        request.timeout = Duration::from_secs(1);
        let started = Instant::now();
        assert_eq!(run(&request).unwrap(), "done\n");
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn missing_program_is_a_bridge_error() {
        let error = run(&RunRequest::new(vec!["/nonexistent/hc".into()], 1)).unwrap_err();
        assert!(error.message().contains("/nonexistent/hc"));
    }
}
