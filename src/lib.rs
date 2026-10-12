use serde::Deserialize;
use serde_json::Value;
use std::{
    ffi::{OsStr, OsString},
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc,
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
    /// Release on clean stdin EOF. Enable only if EOF means the agent quit.
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
        // session.end may be a session switch; turn.finish.before and
        // user.attention don't imply a state change.
        _ => return None,
    };
    let argv: Vec<_> = config
        .resume_argv
        .iter()
        .map(|a| a.replace("{session_id}", session))
        .collect();
    // Never attach invalid resume data: it would reject the entire state report.
    let argv = if validate_argv(&argv).is_ok() {
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

/// HerdR CLI target. Reporting requires all of `HERDR_ENV=1`, `HERDR_BIN_PATH`,
/// `HERDR_PANE_ID` and `HERDR_SOCKET_PATH`; the CLI reads the socket itself.
struct Herdr {
    bin: OsString,
    pane: OsString,
    source: String,
    agent: String,
}
impl Herdr {
    fn from_env(config: &Config) -> Option<Self> {
        let var = |key| std::env::var_os(key).filter(|v| !v.is_empty());
        if std::env::var("HERDR_ENV").ok().as_deref() != Some("1") {
            return None;
        }
        var("HERDR_SOCKET_PATH")?;
        Some(Self {
            bin: var("HERDR_BIN_PATH")?,
            pane: var("HERDR_PANE_ID")?,
            source: config.source.clone(),
            agent: config.agent.clone(),
        })
    }
    fn run(&self, command: &str, seq: u64, extra: Vec<OsString>) -> Option<bool> {
        let mut args: Vec<OsString> = vec![
            "pane".into(),
            command.into(),
            self.pane.clone(),
            "--source".into(),
            self.source.clone().into(),
            "--agent".into(),
            self.agent.clone().into(),
            "--seq".into(),
            seq.to_string().into(),
        ];
        args.extend(extra);
        run_command(&self.bin, &args)
    }
    fn report(&self, seq: u64, report: &Report) -> Option<bool> {
        let mut extra: Vec<OsString> = vec!["--state".into(), report.state.into()];
        if let Some(message) = report.message {
            extra.extend(["--message".into(), message.into()]);
        }
        // Resume data rides on the same report: ownership before resume.
        if !report.argv.is_empty() {
            extra.extend([
                "--agent-session-id".into(),
                report.session.clone().into(),
                "--".into(),
            ]);
            extra.extend(report.argv.iter().map(Into::into));
        }
        self.run("report-agent", seq, extra)
    }
    fn release(&self, seq: u64) -> Option<bool> {
        self.run("release-agent", seq, Vec::new())
    }
}
/// Release the pane. Run only after the agent quits and the hook has stopped.
pub fn release(config: &Config) {
    let Some(herdr) = Herdr::from_env(config) else {
        return;
    };
    match Sequence::open(&config.sequence_file).and_then(|mut s| s.next()) {
        Ok(seq) => {
            herdr.release(seq);
        }
        Err(_) => eprintln!("herdr-ahp: release skipped (sequence storage unavailable or owned)"),
    }
}
enum Message {
    Report(Report),
    Finish { release: bool },
}
pub struct Reporter {
    config: Config,
    tx: Option<mpsc::Sender<Message>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Reporter {
    /// Outside HerdR, or if sequence storage/worker startup fails, reporting is
    /// disabled but the protocol server keeps running.
    pub fn start(config: Config) -> Self {
        let disabled = |config| Self {
            config,
            tx: None,
            worker: None,
        };
        let Some(herdr) = Herdr::from_env(&config) else {
            return disabled(config);
        };
        // The file lock stops a second hook for this pane from reporting.
        let Ok(sequence) = Sequence::open(&config.sequence_file) else {
            eprintln!("herdr-ahp: reporting disabled (sequence storage unavailable or owned)");
            return disabled(config);
        };
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("herdr-reporter".into())
            .spawn(move || run_worker(rx, herdr, sequence))
        {
            Ok(worker) => Self {
                config,
                tx: Some(tx),
                worker: Some(worker),
            },
            Err(_) => {
                eprintln!("herdr-ahp: reporting disabled (worker unavailable)");
                disabled(config)
            }
        }
    }
    pub fn observe(&self, event: &Value) {
        if let Some(tx) = &self.tx
            && let Some(report) = map_event(&self.config, event)
        {
            let _ = tx.send(Message::Report(report));
        }
    }
    /// Drain the latest state; release only if clean EOF proves actual agent exit.
    pub fn finish(&self, clean_eof: bool) {
        if let Some(tx) = &self.tx {
            let release = clean_eof && self.config.release_on_eof;
            let _ = tx.send(Message::Finish { release });
        }
    }
}
impl Drop for Reporter {
    fn drop(&mut self) {
        // Closing the channel without `Finish` (unwind, protocol failure) never releases.
        self.tx.take();
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
    fn open(path: &Path) -> io::Result<Self> {
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
    /// `max(previous + 1, unix_time_ms)`, persisted before the command is sent.
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
        let line = format!("{next}\n");
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(line.as_bytes())?;
        self.file.set_len(line.len() as u64)?;
        self.file.sync_data()?;
        self.last = next;
        Ok(next)
    }
}
fn run_worker(rx: mpsc::Receiver<Message>, herdr: Herdr, mut sequence: Sequence) {
    let mut last_success: Option<Report> = None;
    let mut owned = false;
    while let Ok(first) = rx.recv() {
        // Coalesce whatever queued while the previous command ran: only the
        // newest state is worth sending.
        let (mut latest, mut finish) = (None, None);
        for message in std::iter::once(first).chain(rx.try_iter()) {
            match message {
                Message::Report(report) => latest = Some(report),
                Message::Finish { release } => finish = Some(release),
            }
        }
        if let Some(report) = latest
            && last_success.as_ref() != Some(&report)
        {
            // A timeout/nonzero exit may follow server acceptance. Never use a
            // prior acknowledgement to deduplicate after an uncertain update.
            last_success = None;
            let Ok(seq) = sequence.next() else {
                return;
            };
            if let Some(success) = herdr.report(seq, &report) {
                owned = true; // Source-scoped release is safe after possible acquisition.
                if success {
                    last_success = Some(report);
                }
            }
        }
        if let Some(release) = finish {
            if release
                && owned
                && let Ok(seq) = sequence.next()
            {
                herdr.release(seq);
            }
            return;
        }
    }
}
fn run_command(bin: &OsStr, args: &[OsString]) -> Option<bool> {
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
}
