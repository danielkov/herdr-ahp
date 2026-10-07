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
struct Bridge(Reporter);
impl Handler for Bridge {
    fn handle(&self, _: Principal, message: Incoming) -> HandlerFuture<'_> {
        Box::pin(async move {
            match message {
                Incoming::Observe(notification) => {
                    let event = serde_json::to_value(notification.params.event)
                        .map_err(|e| e.to_string())?;
                    self.0.observe(&event);
                    Ok(Outgoing::Observed)
                }
                _ => Err("This hook supports registered observations only".into()),
            }
        })
    }
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
    let server = Server {
        handler: Bridge(Reporter::start(config)?),
        authenticator: LocalProcess,
        max_body_bytes: 1024 * 1024,
    };
    let mut stdin = io::stdin().lock();
    // Do not retain stdout lock while waiting on stdin: process spawning may
    // need standard-I/O locks on some platforms. Only this thread writes frames.
    let mut stdout = io::stdout();
    let result = (|| -> io::Result<()> {
        while let Some(frame) = stdio::read_frame(&mut stdin, server.max_body_bytes)? {
            let response = block_on(stdio::dispatch(&server, frame, None));
            stdio::write_frame(&mut stdout, &response, server.max_body_bytes)?;
        }
        Ok(())
    })();
    // EOF is NOT agent exit by default. Opt-in belongs to a lifetime-bound adapter.
    server.handler.0.finish(result.is_ok());
    result?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("herdr-ahp: {error}");
        std::process::exit(1);
    }
}
