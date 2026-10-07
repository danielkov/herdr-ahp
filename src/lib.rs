use serde::Deserialize;
use serde_json::Value;
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Stable integration source, distinct from the AHP event source.
    pub source: String,
    /// User-visible name of the actual host agent.
    pub agent: String,
    /// Durable, private path. Use a different file for each source/pane pair.
    pub sequence_file: PathBuf,
    #[serde(default)]
    pub resume_argv: Vec<String>,
    /// Optional trusted AHP source filter (not authentication).
    #[serde(default)]
    pub event_source: Option<String>,
    /// Restrict to one session if a host also emits subagent events.
    #[serde(default)]
    pub session_id: Option<String>,
    /// Only enable when clean transport EOF proves the host agent actually quit.
    #[serde(default)]
    pub release_on_eof: bool,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.source.is_empty()
            || self.source.starts_with("herdr:")
            || self.source.len() > 128
            || self.agent.is_empty()
            || self.agent.len() > 128
            || self
                .source
                .chars()
                .chain(self.agent.chars())
                .any(char::is_control)
        {
            return Err("invalid source/agent identity".into());
        }
        if self.sequence_file.as_os_str().is_empty() {
            return Err("sequence_file is required".into());
        }
        if !self.resume_argv.is_empty() {
            validate_argv(&self.resume_argv)?;
        }
        Ok(())
    }
}
fn validate_argv(argv: &[String]) -> Result<(), String> {
    if argv.is_empty()
        || argv.len() > 64
        || argv.iter().map(|s| s.len() + 1).sum::<usize>() > 8192
        || argv
            .iter()
            .any(|s| s.chars().any(|c| c == '\'' || c.is_control()))
        || argv[0].is_empty()
        || argv[0] == "."
        || argv[0] == ".."
        || argv[0]
            .chars()
            .any(|c| c == '/' || c == '\\' || c == ':' || c.is_whitespace())
    {
        return Err("invalid HerdR resume argv".into());
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Report {
    state: &'static str,
    message: Option<&'static str>,
    session: String,
    argv: Vec<String>,
}
fn map_event(config: &Config, event: &Value) -> Option<Report> {
    if config
        .event_source
        .as_deref()
        .is_some_and(|s| event["source"].as_str() != Some(s))
    {
        return None;
    }
    let session = event["session"]["id"].as_str()?;
    if config.session_id.as_deref().is_some_and(|s| s != session) {
        return None;
    }
    let (state, message) = match event["type"].as_str()? {
        "session.start" | "turn.end" => ("idle", None),
        "turn.start" => ("working", None),
        "tool.permission.request" => ("blocked", Some("Tool permission required")),
        "user.elicitation.request" => ("blocked", Some("User input required")),
        "tool.permission.resolved" | "user.elicitation.result" => ("working", None),
        // session.end can be a switch, and finish.before can still be intercepted.
        // user.attention is not necessarily a decision. Neither implies state/exit.
        _ => return None,
    };
    let argv: Vec<_> = config
        .resume_argv
        .iter()
        .map(|a| a.replace("{session_id}", session))
        .collect();
    // Never attach invalid resume data: it would reject the entire state report.
    let argv = if argv.is_empty() || validate_argv(&argv).is_ok() {
        argv
    } else {
        Vec::new()
    };
    Some(Report {
        state,
        message,
        session: session.into(),
        argv,
    })
}

// Shared-state inventory: the handler writes `latest`; finish writes `closed` and
// release; the worker takes latest. Each guarded transition is a complete value
// assignment/take; no I/O, callbacks, awaits, or replaced-value drops under lock.
// Poison is NOT recovered: reporting stops; the agent/protocol continue.
#[derive(Default)]
struct Pending {
    latest: Option<Report>,
    closed: bool,
    release: bool,
}
struct Mailbox {
    pending: Mutex<Pending>,
    ready: Condvar,
}
pub struct Reporter {
    config: Config,
    mailbox: Arc<Mailbox>,
    worker: Option<thread::JoinHandle<()>>,
}
fn herdr_target() -> Option<(std::ffi::OsString, std::ffi::OsString)> {
    if std::env::var("HERDR_ENV").ok().as_deref() == Some("1") {
        let bin = std::env::var_os("HERDR_BIN_PATH").filter(|v| !v.is_empty());
        let pane = std::env::var_os("HERDR_PANE_ID").filter(|v| !v.is_empty());
        let socket = std::env::var_os("HERDR_SOCKET_PATH").filter(|v| !v.is_empty());
        match (bin, pane, socket) {
            (Some(bin), Some(pane), Some(_)) => Some((bin, pane)),
            _ => None,
        }
    } else {
        None
    }
}
/// Explicit host lifecycle signal. Call only after the real agent quits and its
/// persistent hook stops, never for session replacement or ordinary hook shutdown.
pub fn release(config: &Config) {
    let Some((bin, pane)) = herdr_target() else {
        return;
    };
    let attempt = || -> io::Result<()> {
        let mut sequence = Sequence::open(&config.sequence_file)?;
        let seq = sequence.next()?;
        let args = vec![
            "pane".into(),
            "release-agent".into(),
            pane,
            "--source".into(),
            config.source.clone().into(),
            "--agent".into(),
            config.agent.clone().into(),
            "--seq".into(),
            seq.to_string().into(),
        ];
        let _ = run_command(&bin, &args);
        Ok(())
    };
    if attempt().is_err() {
        eprintln!("herdr-ahp: release skipped (sequence storage unavailable or owned)");
    }
}
impl Reporter {
    pub fn start(config: Config) -> io::Result<Self> {
        let mailbox = Arc::new(Mailbox {
            pending: Mutex::new(Pending::default()),
            ready: Condvar::new(),
        });
        let target = herdr_target();
        let worker = if let Some((bin, pane)) = target {
            // Lock and durable sequence belong exclusively to the worker. A second
            // bridge for this same source/pane fails closed, never steals ownership.
            let sequence = match Sequence::open(&config.sequence_file) {
                Ok(sequence) => sequence,
                Err(_) => {
                    eprintln!(
                        "herdr-ahp: reporting disabled (sequence storage unavailable or owned)"
                    );
                    return Ok(Self {
                        config,
                        mailbox,
                        worker: None,
                    });
                }
            };
            let m = mailbox.clone();
            let c = config.clone();
            match thread::Builder::new()
                .name("herdr-reporter".into())
                .spawn(move || {
                    run_worker(m, c, sequence, bin, pane);
                }) {
                Ok(worker) => Some(worker),
                Err(_) => {
                    eprintln!("herdr-ahp: reporting disabled (worker unavailable)");
                    None
                }
            }
        } else {
            None
        };
        Ok(Self {
            config,
            mailbox,
            worker,
        })
    }
    pub fn observe(&self, event: &Value) {
        if self.worker.is_none() {
            return;
        }
        let Some(report) = map_event(&self.config, event) else {
            return;
        };
        if let Ok(mut guard) = self.mailbox.pending.lock() {
            if guard.closed {
                return;
            }
            let old = guard.latest.replace(report);
            drop(guard);
            drop(old);
            self.mailbox.ready.notify_one();
        }
    }
    pub fn finish(&self, clean_eof: bool) {
        if let Ok(mut guard) = self.mailbox.pending.lock() {
            guard.closed = true;
            guard.release = clean_eof && self.config.release_on_eof;
            drop(guard);
        }
        self.mailbox.ready.notify_one();
    }
}
impl Drop for Reporter {
    fn drop(&mut self) {
        // Unwind/protocol failure/drop is not evidence of host exit.
        if let Ok(mut guard) = self.mailbox.pending.lock() {
            guard.closed = true;
            drop(guard);
        }
        self.mailbox.ready.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
struct Sequence {
    file: File,
    last: u64,
}
impl Sequence {
    fn open(path: &std::path::Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path)?;
        file.try_lock().map_err(io::Error::other)?;
        let mut contents = String::new();
        (&mut file).take(64).read_to_string(&mut contents)?;
        let last = if contents.is_empty() {
            0
        } else {
            contents.trim().parse().map_err(io::Error::other)?
        };
        Ok(Self { file, last })
    }
    fn next(&mut self) -> io::Result<u64> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis();
        let next = self
            .last
            .checked_add(1)
            .ok_or_else(|| io::Error::other("sequence exhausted"))?
            .max(u64::try_from(now).map_err(io::Error::other)?);
        self.file.seek(SeekFrom::Start(0))?;
        // Persist before sending. A failed command still consumes its number.
        writeln!(self.file, "{next}")?;
        self.file.set_len(next.to_string().len() as u64 + 1)?;
        self.file.sync_data()?;
        self.last = next;
        Ok(next)
    }
}
fn run_worker(
    mailbox: Arc<Mailbox>,
    config: Config,
    mut sequence: Sequence,
    bin: std::ffi::OsString,
    pane: std::ffi::OsString,
) {
    let mut last_success: Option<Report> = None;
    let mut owned = false;
    loop {
        let (report, closed, release) = {
            let Ok(mut guard) = mailbox.pending.lock() else {
                return;
            };
            while guard.latest.is_none() && !guard.closed {
                guard = match mailbox.ready.wait(guard) {
                    Ok(g) => g,
                    Err(_) => return,
                };
            }
            (guard.latest.take(), guard.closed, guard.release)
        };
        if let Some(report) = report
            && last_success.as_ref() != Some(&report)
        {
            // A timeout/nonzero exit may follow server acceptance. Never use a
            // prior acknowledgement to deduplicate after an uncertain update.
            last_success = None;
            let Ok(seq) = sequence.next() else {
                return;
            };
            let mut args = vec![
                "pane".into(),
                "report-agent".into(),
                pane.clone(),
                "--source".into(),
                config.source.clone().into(),
                "--agent".into(),
                config.agent.clone().into(),
                "--state".into(),
                report.state.into(),
                "--seq".into(),
                seq.to_string().into(),
            ];
            if let Some(message) = report.message {
                args.extend(["--message".into(), message.into()]);
            }
            if !report.argv.is_empty() {
                args.extend([
                    "--agent-session-id".into(),
                    report.session.clone().into(),
                    "--".into(),
                ]);
                args.extend(report.argv.iter().map(Into::into));
            }
            if let Some(success) = run_command(&bin, &args) {
                owned = true; // Source-scoped release is safe after possible acquisition.
                if success {
                    last_success = Some(report);
                }
            }
        }
        if closed {
            if release
                && owned
                && let Ok(seq) = sequence.next()
            {
                let args = vec![
                    "pane".into(),
                    "release-agent".into(),
                    pane.clone(),
                    "--source".into(),
                    config.source.clone().into(),
                    "--agent".into(),
                    config.agent.clone().into(),
                    "--seq".into(),
                    seq.to_string().into(),
                ];
                let _ = run_command(&bin, &args);
            }
            return;
        }
    }
}
fn run_command(bin: &std::ffi::OsStr, args: &[std::ffi::OsString]) -> Option<bool> {
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return None;
    };
    let deadline = Instant::now() + Duration::from_millis(300);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Some(false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            source: "org.example.bridge".into(),
            agent: "example-agent".into(),
            sequence_file: PathBuf::from("unused"),
            resume_argv: vec![
                "example-agent".into(),
                "--resume".into(),
                "{session_id}".into(),
            ],
            event_source: None,
            session_id: None,
            release_on_eof: false,
        }
    }
    #[test]
    fn mapping_and_filters() {
        let mut config = config();
        for (name, state) in [
            ("session.start", "idle"),
            ("turn.start", "working"),
            ("turn.end", "idle"),
            ("tool.permission.request", "blocked"),
            ("tool.permission.resolved", "working"),
            ("user.elicitation.request", "blocked"),
            ("user.elicitation.result", "working"),
        ] {
            let report = map_event(
                &config,
                &serde_json::json!({"type":name,"session":{"id":"s1"}}),
            )
            .unwrap();
            assert_eq!(report.state, state);
            assert_eq!(report.argv[2], "s1");
        }
        for name in [
            "session.end",
            "turn.finish.before",
            "tool.after",
            "user.attention",
        ] {
            assert!(
                map_event(
                    &config,
                    &serde_json::json!({"type":name,"session":{"id":"s1"}})
                )
                .is_none()
            );
        }
        config.event_source = Some("urn:host".into());
        assert!(
            map_event(
                &config,
                &serde_json::json!({"type":"turn.start","source":"urn:other","session":{"id":"s1"}})
            )
            .is_none()
        );
        config.event_source = None;
        config.session_id = Some("s2".into());
        assert!(
            map_event(
                &config,
                &serde_json::json!({"type":"turn.start","session":{"id":"s1"}})
            )
            .is_none()
        );
    }
    #[test]
    fn resume_constraints_do_not_suppress_state() {
        let c = config();
        let report = map_event(
            &c,
            &serde_json::json!({"type":"turn.start","session":{"id":"bad'id"}}),
        )
        .unwrap();
        assert!(report.argv.is_empty());
        assert_eq!(report.state, "working");
        for argv in [
            vec!["/bin/agent".into()],
            vec!["a'b".into()],
            vec!["a\n".into()],
            vec!["..".into()],
        ] {
            assert!(validate_argv(&argv).is_err());
        }
        let mut c = c;
        c.source = "herdr:reserved".into();
        assert!(c.validate().is_err());
    }
    #[test]
    fn sequence_survives_restart_and_rejects_concurrent_owner() {
        let path = std::env::temp_dir().join(format!("herdr-sequence-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut sequence = Sequence::open(&path).unwrap();
        let first = sequence.next().unwrap();
        assert!(Sequence::open(&path).is_err());
        drop(sequence);
        let mut second = Sequence::open(&path).unwrap();
        assert!(second.next().unwrap() > first);
        drop(second);
        std::fs::write(&path, "corrupt").unwrap();
        assert!(Sequence::open(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn poisoned_waiting_worker_is_notified_and_joined() {
        let path = std::env::temp_dir().join(format!("herdr-poison-test-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let sequence = Sequence::open(&path).unwrap();
        let mailbox = Arc::new(Mailbox {
            pending: Mutex::new(Pending::default()),
            ready: Condvar::new(),
        });
        let m = mailbox.clone();
        let c = config();
        let worker =
            thread::spawn(move || run_worker(m, c, sequence, "unused".into(), "pane".into()));
        let reporter = Reporter {
            config: config(),
            mailbox: mailbox.clone(),
            worker: Some(worker),
        };
        thread::sleep(Duration::from_millis(20));
        let m = mailbox.clone();
        assert!(
            thread::spawn(move || {
                let _guard = m.pending.lock().unwrap();
                panic!("test poison");
            })
            .join()
            .is_err()
        );
        // Drop must notify even though it cannot take the poisoned lock. The
        // worker exits on poison and is joined; no recovery or release is made.
        drop(reporter);
        assert!(mailbox.pending.lock().is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn mailbox_coalesces_and_poison_disables_reporting() {
        let mailbox = Arc::new(Mailbox {
            pending: Mutex::new(Pending::default()),
            ready: Condvar::new(),
        });
        let mut c = config();
        c.release_on_eof = true;
        // Dummy worker exists only to admit production observe; it owns no state.
        let reporter = Reporter {
            config: c,
            mailbox: mailbox.clone(),
            worker: Some(thread::spawn(|| {})),
        };
        for name in ["turn.start", "tool.permission.request", "turn.end"] {
            reporter.observe(&serde_json::json!({"type":name,"session":{"id":"s1"}}));
        }
        assert_eq!(
            mailbox
                .pending
                .lock()
                .unwrap()
                .latest
                .as_ref()
                .unwrap()
                .state,
            "idle"
        );
        reporter.finish(false);
        assert!(!mailbox.pending.lock().unwrap().release);
        reporter.observe(&serde_json::json!({"type":"turn.start","session":{"id":"s1"}}));
        assert_eq!(
            mailbox
                .pending
                .lock()
                .unwrap()
                .latest
                .as_ref()
                .unwrap()
                .state,
            "idle"
        );
        let m = mailbox.clone();
        assert!(
            thread::spawn(move || {
                let _guard = m.pending.lock().unwrap();
                panic!("test poison");
            })
            .join()
            .is_err()
        );
        reporter.finish(true); // Does not recover a poisoned invariant.
        drop(reporter);
        assert!(mailbox.pending.lock().is_err());
    }
}
