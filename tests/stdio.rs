//! End-to-end tests against a fake HerdR CLI.
#![cfg(unix)]
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Fixture {
    root: PathBuf,
    cli: PathBuf,
    args: Vec<String>,
    log: PathBuf,
}
impl Fixture {
    fn new(name: &str, release: bool, behavior: &str) -> Self {
        let root = std::env::temp_dir().join(format!("herdr-ahp-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let cli = PathBuf::from("/bin/sh");
        let script = root.join("pane");
        let log = root.join("calls.jsonl");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' 'pane' \"$@\" '__END__' >> '{}'\n{behavior}\n",
                log.to_str().unwrap()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
        let mut args: Vec<String> = ["--source", "org.example.bridge", "--agent", "example-agent"]
            .map(String::from)
            .into();
        if release {
            args.push("--release-on-eof".into());
        }
        args.extend(["--", "example-agent", "--resume", "{session_id}"].map(String::from));
        Self {
            root,
            cli,
            args,
            log,
        }
    }
    fn spawn(&self, enabled: bool) -> Child {
        let mut c = Command::new(env!("CARGO_BIN_EXE_herdr-ahp"));
        c.current_dir(&self.root)
            .args(&self.args)
            .env("HERDR_ENV", if enabled { "1" } else { "0" })
            .env("HERDR_BIN_PATH", &self.cli)
            .env("HERDR_PANE_ID", "pane-test")
            .env("HERDR_SOCKET_PATH", self.root.join("unused.sock"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        c.spawn().unwrap()
    }
    fn calls(&self) -> Vec<Vec<String>> {
        let data = fs::read_to_string(&self.log).unwrap_or_default();
        data.split("__END__\n")
            .take(data.matches("__END__\n").count())
            .map(|s| s.lines().map(str::to_owned).collect())
            .collect()
    }
    fn wait_calls(&self, count: usize) {
        let start = Instant::now();
        while self.calls().len() < count {
            assert!(
                start.elapsed() < Duration::from_secs(20),
                "calls: {:?}",
                self.calls()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn event(kind: &str, session: &str) -> Value {
    let mut e = json!({"id":format!("evt-{kind}"),"type":kind,"time":"2026-10-07T12:00:00Z","source":"urn:example:host",
        "session":{"id":session},"turn":{"id":"t1"}});
    match kind {
        "turn.start" => {
            e["items"] = json!([]);
            e["trigger"] = json!("user");
        }
        "turn.end" => {
            e["items"] = json!([]);
            e["outcome"] = json!("completed");
            e["continuationCount"] = json!(0);
        }
        "tool.permission.request" => {
            e["call"] = json!({"id":"c1"});
            e["tool"] = json!({"name":"Shell","origin":"native","input":{},"kind":"shell"});
            e["path"] = json!("shell");
            e["sandboxBypass"] = json!(false);
            e["suggestions"] = json!([]);
        }
        "session.end" => {
            e["outcome"] = json!("completed");
            e["reason"] = json!("session-switch");
        }
        _ => {}
    }
    e
}
fn send(child: &mut Child, e: Value) {
    let frame = json!({"jsonrpc":"2.0","method":"hooks/observe","params":{"protocolVersion":"draft","event":e}});
    writeln!(child.stdin.as_mut().unwrap(), "{frame}").unwrap();
    child.stdin.as_mut().unwrap().flush().unwrap();
}
fn close(mut child: Child) -> std::process::Output {
    drop(child.stdin.take());
    child.wait_with_output().unwrap()
}
fn seq(args: &[String]) -> u64 {
    let i = args.iter().position(|s| s == "--seq").unwrap();
    args[i + 1].parse().unwrap()
}
#[test]
fn maps_states_resume_and_actual_exit_opt_in() {
    let f = Fixture::new("mapping", true, "");
    let mut child = f.spawn(true);
    for (i, (kind, state)) in [
        ("turn.start", "working"),
        ("tool.permission.request", "blocked"),
        ("turn.end", "idle"),
    ]
    .iter()
    .enumerate()
    {
        send(&mut child, event(kind, "s1"));
        f.wait_calls(i + 1);
        let calls = f.calls();
        let args = &calls[i];
        assert_eq!(&args[..3], ["pane", "report-agent", "pane-test"]);
        let p = args.iter().position(|s| s == "--state").unwrap();
        assert_eq!(args[p + 1], *state);
        assert_eq!(
            &args[args.len() - 4..],
            ["--", "example-agent", "--resume", "s1"]
        );
    }
    send(&mut child, event("session.end", "s1"));
    send(&mut child, event("turn.start", "s2"));
    f.wait_calls(4);
    let output = close(child);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "notifications must not emit stdout"
    );
    let calls = f.calls();
    assert_eq!(calls.len(), 5);
    assert_eq!(calls[4][1], "release-agent");
    assert!(calls.windows(2).all(|w| seq(&w[1]) > seq(&w[0])));
    let mut child = f.spawn(true);
    send(&mut child, event("turn.start", "s3"));
    f.wait_calls(6);
    close(child);
    assert!(seq(&f.calls()[5]) > seq(&calls[4]));
}
#[test]
fn no_release_on_transport_eof_and_noop_outside_herdr() {
    let f = Fixture::new("no-release", false, "");
    let mut child = f.spawn(true);
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(1);
    assert!(close(child).status.success());
    assert_eq!(f.calls().len(), 1);
    let mut child = f.spawn(false);
    send(&mut child, event("turn.start", "s1"));
    assert!(close(child).status.success());
    assert_eq!(f.calls().len(), 1);
}
#[test]
fn latest_state_and_timeout_are_bounded() {
    let f = Fixture::new("coalesce", false, "sleep 2");
    let mut child = f.spawn(true);
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(1);
    for _ in 0..20 {
        send(&mut child, event("tool.permission.request", "s1"));
    }
    send(&mut child, event("turn.end", "s1"));
    let start = Instant::now();
    assert!(close(child).status.success());
    assert!(start.elapsed() < Duration::from_secs(2));
    let calls = f.calls();
    assert_eq!(calls.len(), 2);
    let last = &calls[1];
    assert_eq!(
        last[last.iter().position(|s| s == "--state").unwrap() + 1],
        "idle"
    );
}
#[test]
fn invalid_framing_does_not_release_and_uncertain_report_does() {
    let f = Fixture::new("failure", true, "exit 9");
    let mut child = f.spawn(true);
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(1);
    close(child);
    assert_eq!(f.calls().len(), 2);
    assert_eq!(f.calls()[1][1], "release-agent");
    let mut child = f.spawn(true);
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"{\"jsonrpc\":\"2.0\"}")
        .unwrap();
    let out = close(child);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    assert_eq!(f.calls().len(), 2);
}
#[test]
fn sdk_registered_process_observe_smoke() {
    use agenthooksprotocol::{
        capability::{self, HostCapabilities},
        generated as g,
        hooks::{Hooks, HooksOptions},
        inputs::*,
        registration::Registration,
    };
    let f = Fixture::new("sdk", false, "");
    // Registration has no env map, so set HerdR variables through sh.
    let registration: Registration = serde_json::from_value(json!({"protocolVersion":"draft","hooks":[{
        "id":"org.example.bridge","transport":{"type":"stdio","command":"/bin/sh",
        "args":["-c","export HERDR_ENV=1 HERDR_BIN_PATH=/bin/sh HERDR_PANE_ID=pane-test HERDR_SOCKET_PATH=unused; exec \"$0\" --agent example-agent", env!("CARGO_BIN_EXE_herdr-ahp")],
        "cwd":f.root,"lifecycle":"persistent"},
        "subscriptions":[{"events":["turn.start"],"mode":"observe","content":{"default":"metadata"}}]}]})).unwrap();
    let options = HooksOptions::new(
        "urn:example:host",
        HostCapabilities {
            turn_start: Some(capability::turn_start().observe()),
            ..Default::default()
        },
    );
    let hooks = Hooks::new(registration, options).unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        hooks
            .turn_start(
                TurnStartInput::new(
                    vec![],
                    g::TurnStartInputTrigger::User,
                    g::TurnStartInputTurn::new("t1"),
                )
                .with_session(Box::new(g::Session::new("s1"))),
            )
            .await
            .unwrap();
        assert_eq!(hooks.wait_until_idle().await.delivered, 1);
        // SDK observe success means flushed stdin, not callback completion.
        f.wait_calls(1);
        hooks.shutdown().await.unwrap();
    });
    assert_eq!(f.calls()[0][1], "report-agent");
}

#[test]
fn uncertain_update_cannot_deduplicate_an_old_success() {
    let f = Fixture::new(
        "uncertain",
        false,
        "case \" $* \" in *' --state working '*) sleep 2;; esac",
    );
    let mut child = f.spawn(true);
    send(&mut child, event("turn.end", "s1"));
    f.wait_calls(1);
    // A complete CLI record precedes process exit; wait for the success to settle.
    std::thread::sleep(Duration::from_millis(30));
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(2);
    send(&mut child, event("turn.end", "s1"));
    assert!(close(child).status.success());
    let calls = f.calls();
    assert_eq!(calls.len(), 3);
    assert_eq!(
        calls[2][calls[2].iter().position(|s| s == "--state").unwrap() + 1],
        "idle"
    );
}
#[test]
fn possible_first_acquisition_is_released_on_proven_exit() {
    let f = Fixture::new(
        "uncertain-release",
        true,
        "case \"$1\" in report-agent) sleep 2;; esac",
    );
    let mut child = f.spawn(true);
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(1);
    assert!(close(child).status.success());
    let calls = f.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1][1], "release-agent");
    assert!(seq(&calls[1]) > seq(&calls[0]));
}

#[test]
fn explicit_release_after_hook_stops() {
    let f = Fixture::new("explicit-release", false, "");
    let mut child = f.spawn(true);
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(1);
    assert!(close(child).status.success());
    let out = Command::new(env!("CARGO_BIN_EXE_herdr-ahp"))
        .current_dir(&f.root)
        .args(["--source", "org.example.bridge", "--release"])
        .env("HERDR_ENV", "1")
        .env("HERDR_BIN_PATH", &f.cli)
        .env("HERDR_PANE_ID", "pane-test")
        .env("HERDR_SOCKET_PATH", "unused")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let calls = f.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1][1], "release-agent");
    assert!(seq(&calls[1]) > seq(&calls[0]));
}

fn session_turn_grants() -> agenthooksprotocol::capability::HostCapabilities {
    use agenthooksprotocol::capability::{self, HostCapabilities};
    HostCapabilities {
        session_start: Some(capability::session_start().observe()),
        turn_start: Some(capability::turn_start().observe()),
        turn_end: Some(capability::turn_end().observe()),
        ..Default::default()
    }
}
#[test]
fn optional_interaction_example_is_admitted_with_exact_observe_grants() {
    use agenthooksprotocol::{
        capability::{self, HostCapabilities},
        hooks::{Hooks, HooksOptions},
        registration::Registration,
    };
    let registration: Registration = serde_json::from_str(include_str!(
        "../examples/registration-with-interactions.json"
    ))
    .unwrap();
    let grants = HostCapabilities {
        tool_permission_request: Some(capability::tool_permission_request().observe()),
        tool_permission_resolved: Some(capability::tool_permission_resolved().observe()),
        user_elicitation_request: Some(capability::user_elicitation_request().observe()),
        user_elicitation_result: Some(capability::user_elicitation_result().observe()),
        ..session_turn_grants()
    };
    // Constructor validates canonical registration/grants without launching I/O.
    Hooks::new(registration, HooksOptions::new("urn:example:agent", grants)).unwrap();
}

#[test]
fn minimal_registration_requires_only_session_turn_grants() {
    use agenthooksprotocol::{
        hooks::{Hooks, HooksOptions},
        registration::Registration,
    };
    let registration: Registration =
        serde_json::from_str(include_str!("../examples/registration.json")).unwrap();
    Hooks::new(
        registration,
        HooksOptions::new("urn:example:agent", session_turn_grants()),
    )
    .unwrap();
}

#[test]
fn rejected_notifications_never_write_stdout() {
    let f = Fixture::new("rejected", false, "");
    let mut child = f.spawn(true);
    // Schema-invalid observation: the SDK rejects it, but a notification gets no reply.
    send(
        &mut child,
        json!({"type":"turn.start","session":{"id":"s1"}}),
    );
    send(&mut child, event("turn.start", "s1"));
    f.wait_calls(1);
    let out = close(child);
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}
