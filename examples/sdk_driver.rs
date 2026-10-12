//! Demo host: reads canonical event JSON lines from stdin and delivers them to the
//! registered hook through the SDK.
use agenthooksprotocol::{
    capability::{self, HostCapabilities, Observe},
    hooks::{Hooks, HooksOptions},
    registration::Registration,
};
use std::io::{BufRead, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: sdk_driver <registration.json>")?;
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let registration: Registration = serde_json::from_slice(&std::fs::read(path)?)?;
    // Grant only the events the host actually emits.
    let options = HooksOptions::new(
        "urn:example:agent",
        HostCapabilities {
            session_start: Some(capability::session_start().observe()),
            turn_start: Some(capability::turn_start().observe()),
            turn_end: Some(Observe),
            tool_permission_request: Some(capability::tool_permission_request().observe()),
            tool_permission_resolved: Some(Observe),
            user_elicitation_request: Some(capability::user_elicitation_request().observe()),
            user_elicitation_result: Some(capability::user_elicitation_result().observe()),
            ..Default::default()
        },
    );
    let hooks = Hooks::new(registration, options)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let result = runtime.block_on(async {
        for line in stdin.lock().lines() {
            let event: serde_json::Value = serde_json::from_str(&line?)?;
            if event.get("shutdown").and_then(serde_json::Value::as_bool) == Some(true) {
                break;
            }
            let name = event["type"]
                .as_str()
                .ok_or("event type required")?
                .to_owned();
            hooks.event(event).await?;
            let report = hooks.wait_until_idle().await;
            if !report.failures.is_empty() {
                return Err("observation delivery failed".into());
            }
            writeln!(
                stdout,
                "{}",
                serde_json::json!({"event":name,"delivered":report.delivered})
            )?;
            stdout.flush()?;
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    });
    // Kills the hook; does not release the pane.
    runtime.block_on(hooks.shutdown())?;
    result
}
