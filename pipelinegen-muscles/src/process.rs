use std::io::{self, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const DEFAULT_OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct RustProcessRunner {
    timeout: Duration,
    output_limit: usize,
}

#[derive(Debug)]
pub(crate) struct ProcessOutput {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl RustProcessRunner {
    pub(crate) fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            output_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            output_limit: DEFAULT_OUTPUT_LIMIT,
        }
    }

    pub(crate) fn command(&self, program: &str, args: Vec<String>) -> ProcessCommand {
        ProcessCommand {
            runner: *self,
            program: program.to_string(),
            args,
        }
    }

    fn run(&self, program: &str, args: &[String]) -> io::Result<ProcessOutput> {
        self.run_with_handler(program, args, |_| {})
    }

    /// run_with_handler is `run` plus an incremental line callback on the
    /// child's stderr. The handler is invoked for every COMPLETE line as it
    /// streams (before the tail cap truncates anything), so callers can
    /// accumulate facts from verbose output that would otherwise exceed the
    /// retained tail — e.g. ffmpeg's per-frame `-benchmark_all` lines on a
    /// long render. The tail buffer and its 64KiB cap are unchanged.
    fn run_with_handler(
        &self,
        program: &str,
        args: &[String],
        on_line: impl FnMut(&str) + Send + 'static,
    ) -> io::Result<ProcessOutput> {
        let mut command = process_command(program, args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");
        let limit = self.output_limit;
        let stdout_thread = thread::spawn(move || read_tail(stdout, limit, |_| {}));
        let stderr_thread = thread::spawn(move || read_tail(stderr, limit, on_line));

        // Wait on the child through a watcher thread + channel instead of a
        // try_wait/sleep poll loop: the waiter blocks in waitpid with zero
        // wakeups (a 10-minute timeout previously woke ~60k times per render)
        // and the timeout fires on the deadline, not on the next poll tick.
        let child_pid = child.id();
        let (status_tx, status_rx) = mpsc::channel::<io::Result<ExitStatus>>();
        let wait_thread = {
            let mut child = child;
            thread::spawn(move || {
                let result = child.wait();
                let _ = status_tx.send(result);
            })
        };
        let started = Instant::now();
        let status = match status_rx.recv_timeout(self.timeout) {
            Ok(status) => status?,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                kill_process_tree(child_pid);
                // Reap the killed child so no zombie outlives the runner.
                let _ = status_rx.recv();
                let _ = wait_thread.join();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("process timed out after {}s", started.elapsed().as_secs().max(self.timeout.as_secs())),
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(io::Error::other("child wait thread panicked"));
            }
        };

        let stdout = stdout_thread
            .join()
            .map_err(|_| io::Error::other("stdout reader thread panicked"))??;
        let stderr = stderr_thread
            .join()
            .map_err(|_| io::Error::other("stderr reader thread panicked"))??;
        Ok(ProcessOutput {
            status,
            stdout,
            stderr,
        })
    }
}

pub(crate) struct ProcessCommand {
    runner: RustProcessRunner,
    program: String,
    args: Vec<String>,
}

impl ProcessCommand {
    pub(crate) fn arg(&mut self, arg: impl Into<String>) -> &mut Self {
        self.args.push(arg.into());
        self
    }

    pub(crate) fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_string()));
        self
    }

    pub(crate) fn output(self) -> io::Result<ProcessOutput> {
        self.runner.run(&self.program, &self.args)
    }

    /// output_with_line_handler streams the child's stderr to `on_line`
    /// (complete lines only) while still returning the capped tail buffer.
    pub(crate) fn output_with_line_handler(
        self,
        on_line: impl FnMut(&str) + Send + 'static,
    ) -> io::Result<ProcessOutput> {
        self.runner.run_with_handler(&self.program, &self.args, on_line)
    }
}

pub(crate) struct FFmpegRunner {
    process: RustProcessRunner,
    ffmpeg: String,
    ffprobe: String,
}

impl FFmpegRunner {
    pub(crate) fn from_ffmpeg_path(ffmpeg: &str) -> Self {
        let ffprobe = super::probe::ffprobe_path(ffmpeg);
        Self::new(ffmpeg, &ffprobe)
    }

    pub(crate) fn from_ffprobe_path(ffprobe: &str) -> Self {
        Self::new("ffmpeg", ffprobe)
    }

    pub(crate) fn new(ffmpeg: &str, ffprobe: &str) -> Self {
        Self {
            process: RustProcessRunner::new(),
            ffmpeg: ffmpeg.to_string(),
            ffprobe: ffprobe.to_string(),
        }
    }

    pub(crate) fn ffmpeg(&self) -> ProcessCommand {
        self.process.command(&self.ffmpeg, Vec::new())
    }

    pub(crate) fn ffprobe(&self) -> ProcessCommand {
        self.process.command(&self.ffprobe, Vec::new())
    }
}

fn read_tail<R: Read>(
    mut reader: R,
    limit: usize,
    mut on_line: impl FnMut(&str),
) -> io::Result<Vec<u8>> {
    // Tail retention: the buffer grows in chunk steps and is trimmed in bulk
    // only when it exceeds limit by more than one chunk — amortized O(1) per
    // byte instead of a full-limit memmove per chunk (~8x traffic reduction
    // for the per-frame `-benchmark_all` stderr stream). The final trim pins
    // the retained tail to exactly `limit` bytes before returning.
    let mut tail: Vec<u8> = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0_u8; 8192];
    let mut line_buf: Vec<u8> = Vec::new();
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            // Final unterminated line (no trailing newline) still reaches the
            // handler.
            if !line_buf.is_empty() {
                if let Ok(line) = std::str::from_utf8(&line_buf) {
                    on_line(line);
                }
            }
            break;
        }
        tail.extend_from_slice(&chunk[..count]);
        if tail.len() > limit.saturating_add(count) {
            let start = tail.len() - limit;
            tail.drain(..start);
        }
        // Feed complete lines to the handler as they stream. Newlines are
        // located with a slice scan (auto-vectorizable) and complete lines are
        // passed to the handler zero-copy straight from the chunk — only a
        // partial line spanning chunks is buffered byte-wise. A pathological
        // line longer than the cap is dropped rather than buffered forever.
        let mut line_start = 0;
        while let Some(relative) = chunk[line_start..count].iter().position(|&byte| byte == b'\n') {
            let newline = line_start + relative;
            if line_buf.is_empty() {
                if let Ok(line) = std::str::from_utf8(&chunk[line_start..newline]) {
                    on_line(line);
                }
            } else {
                line_buf.extend_from_slice(&chunk[line_start..newline]);
                if let Ok(line) = std::str::from_utf8(&line_buf) {
                    on_line(line);
                }
                line_buf.clear();
            }
            line_start = newline + 1;
        }
        if line_start < count && line_buf.len() < limit {
            line_buf.extend_from_slice(&chunk[line_start..count]);
        }
    }
    if tail.len() > limit {
        let start = tail.len() - limit;
        tail.drain(..start);
    }
    Ok(tail)
}

fn process_command(program: &str, args: &[String]) -> Command {
    #[cfg(unix)]
    {
        // `setsid` execs the requested program as a new process group leader.
        // Using the system utility keeps this crate safe-code-only while still
        // allowing cancellation of the complete FFmpeg descendant tree.
        let mut command = Command::new("setsid");
        command.arg(program).args(args);
        command
    }
    #[cfg(not(unix))]
    {
        let mut command = Command::new(program);
        command.args(args);
        command
    }
}

fn kill_process_tree(pid: u32) {
    #[cfg(unix)]
    {
        // `setsid` makes the supervised program the leader of a new process
        // group. Signal the negative PGID so FFmpeg and every other descendant
        // are terminated together; killing only the direct child leaves the
        // actual worker alive behind the Rust executor.
        let group = format!("-{pid}");
        let group_killed = Command::new("kill")
            .args(["-KILL", "--", &group])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !group_killed {
            // Fallback: signal the direct child by pid (equivalent of
            // Child::kill) when the group signal could not be delivered.
            let _ = Command::new("kill")
                .args(["-KILL", &pid.to_string()])
                .status();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::RustProcessRunner;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn temp_script(name: &str, body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("pipelinegen-{name}-{}", std::process::id()));
        fs::write(&path, body).expect("write test script");
        path
    }

    #[test]
    fn stderr_is_capped_at_64_kibibytes() {
        let script = temp_script("stderr", "printf '%01048576d' 0 >&2\n");
        let output = RustProcessRunner::new()
            .command("sh", vec![script.to_string_lossy().into_owned()])
            .output()
            .expect("run stderr producer");
        let _ = fs::remove_file(script);
        assert!(output.status.success());
        assert!(output.stderr.len() <= 64 * 1024);
    }

    #[test]
    fn timeout_kills_process_group_and_returns_promptly() {
        let pid_file =
            std::env::temp_dir().join(format!("pipelinegen-child-{}", std::process::id()));
        let script = temp_script(
            "timeout",
            &format!(
                "sleep 30 &\nchild=$!\nprintf '%s' \"$child\" > '{}'\nwait\n",
                pid_file.display()
            ),
        );
        let started = Instant::now();
        let result = RustProcessRunner::with_timeout(Duration::from_millis(100))
            .command("sh", vec![script.to_string_lossy().into_owned()])
            .output();
        let _ = fs::remove_file(&script);
        let error = result.expect_err("long process must time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut child_gone = false;
        while Instant::now() < deadline {
            if let Ok(pid) = fs::read_to_string(&pid_file) {
                if let Ok(pid) = pid.trim().parse::<i32>() {
                    let status = std::process::Command::new("kill")
                        .args(["-0", &pid.to_string()])
                        .status()
                        .expect("probe child process");
                    if !status.success() {
                        child_gone = true;
                        break;
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let _ = fs::remove_file(pid_file);
        assert!(child_gone, "descendant process survived timeout");
    }
}
