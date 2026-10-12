use serde_json::Value;
use std::{
    ffi::{OsStr, OsString},
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Clone)]
pub struct Config {
    /// Stable integration source, distinct from the AHP event source.
    pub source: String,
    /// User-visible agent name. Falls back to the event's `session.agent.id`.
    pub agent: Option<String>,
    /// Command that reopens a session; `{session_id}` is substituted.
    pub resume_argv: Vec<String>,
    /// Ignore events whose AHP `source` differs.
    pub event_source: Option<String>,
    /// Ignore events from other sessions.
    pub session_id: Option<String>,
    /// Release on clean stdin EOF. Enable only if EOF means the agent quit.
    pub release_on_eof: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            source: "herdr-ahp".into(),
            agent: None,
            resume_argv: Vec::new(),
            event_source: None,
            session_id: None,
            release_on_eof: false,
        }
    }
}
const DEFAULT_AGENT: &str = "agent";
fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if !valid_name(&self.source) || self.source.starts_with("herdr:") {
            return Err("invalid --source".into());
        }
        if self.agent.as_deref().is_some_and(|a| !valid_name(a)) {
            return Err("invalid --agent".into());
        }
        if !self.resume_argv.is_empty() {
            validate_argv(&self.resume_argv)?;
        }
        Ok(())
    }
    fn agent_for(&self, event: &Value) -> String {
        self.agent
            .as_deref()
            .or(event["session"]["agent"]["id"]
                .as_str()
                .filter(|a| valid_name(a)))
            .unwrap_or(DEFAULT_AGENT)
            .into()
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
    agent: String,
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
        agent: config.agent_for(event),
        state,
        message,
        session: session.into(),
        argv,
    })
}
/// Wall-clock microseconds, bumped past the previous value if the clock stalls.
#[derive(Default)]
struct Sequence(u64);
impl Sequence {
    fn next(&mut self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as u64);
        self.0 = now.max(self.0.saturating_add(1));
        self.0
    }
}

/// HerdR CLI target. Reporting requires all of `HERDR_ENV=1`, `HERDR_BIN_PATH`,
/// `HERDR_PANE_ID` and `HERDR_SOCKET_PATH`; the CLI reads the socket itself.
struct Herdr {
    bin: OsString,
    pane: OsString,
    source: String,
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
        })
    }
    fn run(&self, command: &str, agent: &str, seq: u64, extra: Vec<OsString>) -> Option<bool> {
        let mut args: Vec<OsString> = vec![
            "pane".into(),
            command.into(),
            self.pane.clone(),
            "--source".into(),
            self.source.clone().into(),
            "--agent".into(),
            agent.into(),
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
        self.run("report-agent", &report.agent, seq, extra)
    }
    fn release(&self, agent: &str, seq: u64) -> Option<bool> {
        self.run("release-agent", agent, seq, Vec::new())
    }
}
/// Release the pane. Run only after the agent quits and the hook has stopped.
pub fn release(config: &Config) {
    if let Some(herdr) = Herdr::from_env(config) {
        let agent = config.agent.as_deref().unwrap_or(DEFAULT_AGENT);
        herdr.release(agent, Sequence::default().next());
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
    /// Outside HerdR, or if the worker cannot start, reporting is disabled but
    /// the protocol server keeps running.
    pub fn start(config: Config) -> Self {
        let disabled = |config| Self {
            config,
            tx: None,
            worker: None,
        };
        let Some(herdr) = Herdr::from_env(&config) else {
            return disabled(config);
        };
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("herdr-reporter".into())
            .spawn(move || run_worker(rx, herdr))
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
fn run_worker(rx: mpsc::Receiver<Message>, herdr: Herdr) {
    let mut sequence = Sequence::default();
    let mut last_success: Option<Report> = None;
    // Agent of the last attempted report; release only after a possible acquisition.
    let mut owner: Option<String> = None;
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
            if let Some(success) = herdr.report(sequence.next(), &report) {
                owner = Some(report.agent.clone());
                if success {
                    last_success = Some(report);
                }
            }
        }
        if let Some(release) = finish {
            if release && let Some(agent) = &owner {
                herdr.release(agent, sequence.next());
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
            agent: Some("example-agent".into()),
            resume_argv: vec![
                "example-agent".into(),
                "--resume".into(),
                "{session_id}".into(),
            ],
            ..Config::default()
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
    fn agent_name_falls_back_to_event_then_default() {
        let event =
            serde_json::json!({"type":"turn.start","session":{"id":"s1","agent":{"id":"codex"}}});
        assert_eq!(map_event(&config(), &event).unwrap().agent, "example-agent");
        let config = Config::default();
        assert_eq!(map_event(&config, &event).unwrap().agent, "codex");
        let bare = serde_json::json!({"type":"turn.start","session":{"id":"s1"}});
        assert_eq!(map_event(&config, &bare).unwrap().agent, DEFAULT_AGENT);
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
    fn sequence_strictly_increases() {
        let mut sequence = Sequence::default();
        let first = sequence.next();
        assert!(sequence.next() > first);
        sequence.0 = u64::MAX - 1;
        assert_eq!(sequence.next(), u64::MAX);
    }
}
