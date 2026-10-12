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
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--config") {
        return Err("usage: herdr-ahp --config <adapter-context.json> [--release]".into());
    }
    let path = args.next().ok_or("missing config path")?;
    let release = match args.next().as_deref() {
        None => false,
        Some("--release") => true,
        _ => return Err("unexpected arguments".into()),
    };
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let config: Config = serde_json::from_slice(&std::fs::read(path)?)?;
    config.validate()?;
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
    let mut stdin = io::stdin().lock();
    // Do not retain stdout lock while waiting on stdin: process spawning may
    // need standard-I/O locks on some platforms. Only this thread writes frames.
    let mut stdout = io::stdout();
    let result = (|| -> io::Result<()> {
        while let Some(frame) = stdio::read_frame(&mut stdin, server.max_body_bytes)? {
            // JSON-RPC notifications get no reply. The SDK process transport never
            // reads stdout after `notify`, so an error frame here would sit in the
            // pipe and desynchronize (or eventually block) the stream.
            let is_request = serde_json::from_slice::<serde_json::Value>(&frame)
                .is_ok_and(|message| message.get("id").is_some());
            let response = block_on(stdio::dispatch(&server, frame, None));
            if is_request {
                stdio::write_frame(&mut stdout, &response, server.max_body_bytes)?;
            } else if response.status >= 400 {
                eprintln!(
                    "herdr-ahp: rejected notification: {}",
                    String::from_utf8_lossy(&response.body)
                );
            }
        }
        Ok(())
    })();
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
