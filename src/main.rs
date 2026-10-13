use agenthooksprotocol::{adapters::stdio, server::*};
use herdr_ahp::{Config, Reporter};
use std::{
    future::Future,
    io,
    pin::pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

// The SDK dispatcher is runtime-neutral. Wake/park also handles pending futures.
struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::thread::park(),
        }
    }
}
struct LocalProcess;
impl Authenticator for LocalProcess {
    fn authenticate(&self, _: Option<&str>) -> Result<Principal, String> {
        // Authority is the host-owned child stdin, not identity fields in events.
        Ok(Principal {
            subject: "local-process".into(),
        })
    }
}
type Observer<E> = Arc<dyn Fn(ObserveContext<E>, E) -> ServerFuture<()> + Send + Sync>;
/// Mapping is keyed on the canonical event JSON, so every mapped event shares one
/// callback. Unmapped observations are accepted by the SDK without a handler;
/// discovery/interception have no handler and are rejected.
fn forward<E: 'static>(reporter: &Arc<Reporter>) -> Option<Observer<E>> {
    let reporter = reporter.clone();
    Some(Arc::new(move |ctx, _| {
        reporter.observe(&ctx.request()["params"]["event"]);
        Box::pin(async { Ok(()) })
    }))
}
const USAGE: &str = "usage: herdr-ahp [--agent NAME] [--source ID] [--event-source URN] \
[--session-id ID] [--release-on-eof] [--release] [-- RESUME_COMMAND... ({session_id})]";
/// Returns the config and whether to run the explicit `--release` command.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<(Config, bool), String> {
    let mut config = Config::default();
    let mut release = false;
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--agent" => config.agent = Some(value()?),
            "--source" => config.source = value()?,
            "--event-source" => config.event_source = Some(value()?),
            "--session-id" => config.session_id = Some(value()?),
            "--release-on-eof" => config.release_on_eof = true,
            "--release" => release = true,
            "--" => {
                config.resume_argv = args.collect();
                break;
            }
            _ => return Err(format!("unexpected argument {arg:?}\n{USAGE}")),
        }
    }
    config.validate()?;
    Ok((config, release))
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("herdr-ahp {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (config, release) = parse_args(std::env::args().skip(1))?;
    if release {
        herdr_ahp::release(&config);
        return Ok(());
    }
    let reporter = Arc::new(Reporter::start(config));
    let mut handlers = HandlerSet::default();
    let observe = &mut handlers.observe;
    observe.session_start = forward(&reporter);
    observe.turn_start = forward(&reporter);
    observe.turn_end = forward(&reporter);
    observe.tool_permission_request = forward(&reporter);
    observe.tool_permission_resolved = forward(&reporter);
    observe.user_elicitation_request = forward(&reporter);
    observe.user_elicitation_result = forward(&reporter);
    let server = Server {
        handlers,
        authenticator: LocalProcess,
        max_body_bytes: 4 * 1024 * 1024,
    };
    // Do not retain the stdout lock while waiting on stdin: process spawning may
    // need standard-I/O locks on some platforms.
    let result = block_on(stdio::serve(
        &server,
        io::stdin().lock(),
        io::stdout(),
        None,
        server.max_body_bytes,
    ));
    // Only clean EOF with `release_on_eof` releases the pane.
    reporter.finish(result.is_ok());
    // Dropping the server and our handle joins the worker after it drains.
    drop(server);
    drop(reporter);
    result?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("herdr-ahp: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<(Config, bool), String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }
    #[test]
    fn flags_and_resume_command() {
        let (config, release) = parse(&[
            "--agent",
            "codex",
            "--release-on-eof",
            "--",
            "codex",
            "resume",
            "{session_id}",
        ])
        .unwrap();
        assert_eq!(config.agent.as_deref(), Some("codex"));
        assert_eq!(config.source, "herdr-ahp");
        assert!(config.release_on_eof && !release);
        assert_eq!(config.resume_argv, ["codex", "resume", "{session_id}"]);
        assert!(parse(&[]).is_ok());
        assert!(parse(&["--agent"]).is_err());
        assert!(parse(&["--source", "herdr:x"]).is_err());
        assert!(parse(&["--", "/bin/codex"]).is_err());
    }
}
