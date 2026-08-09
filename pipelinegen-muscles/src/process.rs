use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
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
        let mut command = process_command(program, args);
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");
        let limit = self.output_limit;
        let stdout_thread = thread::spawn(move || read_tail(stdout, limit));
        let stderr_thread = thread::spawn(move || read_tail(stderr, limit));

        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if started.elapsed() >= self.timeout {
                kill_process_tree(&mut child);
                let _ = child.wait();
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("process timed out after {}s", self.timeout.as_secs()),
                ));
            }
            thread::sleep(Duration::from_millis(10));
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

fn read_tail<R: Read>(mut reader: R, limit: usize) -> io::Result<Vec<u8>> {
    let mut tail = Vec::with_capacity(limit.min(8192));
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Ok(tail);
        }
        if count >= limit {
            tail.clear();
            tail.extend_from_slice(&chunk[count - limit..count]);
        } else {
            tail.extend_from_slice(&chunk[..count]);
            if tail.len() > limit {
                let start = tail.len() - limit;
                tail.drain(..start);
            }
        }
    }
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

fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        // Kill descendants first, then the supervised process itself. Using
        // the process-group shorthand here can target the test/worker group
        // when `setsid` falls back to its launcher behavior.
        let pid = child.id().to_string();
        let _ = Command::new("pkill").args(["-KILL", "-P", &pid]).status();
        let _ = child.kill();
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
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
        let pid_file = std::env::temp_dir().join(format!("pipelinegen-child-{}", std::process::id()));
        let script = temp_script(
            "timeout",
            &format!("sleep 30 &\nchild=$!\nprintf '%s' \\\"$child\\\" > '{}'\nwait\n", pid_file.display()),
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
