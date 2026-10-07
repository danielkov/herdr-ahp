//! Demo host: SDK client -> registered process hook. Not the hook's protocol loop.
//! Supply newline-delimited canonical event objects on stdin; SDK binds identity,
//! validates events, projects content and delivers observe notifications.
use agenthooksprotocol::{
    generated as g,
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
    // Demo host authority is explicit and independent of registration. Production
    // hosts must grant only the boundaries they actually implement.
    let options = HooksOptions::from_declarations(
        "urn:example:agent",
        [
            (g::EventType::SessionStart, g::capability::observe()),
            (g::EventType::TurnStart, g::capability::observe()),
            (g::EventType::TurnEnd, g::capability::observe()),
            (
                g::EventType::ToolPermissionRequest,
                g::capability::observe(),
            ),
            (
                g::EventType::ToolPermissionResolved,
                g::capability::observe(),
            ),
            (
                g::EventType::UserElicitationRequest,
                g::capability::observe(),
            ),
            (
                g::EventType::UserElicitationResult,
                g::capability::observe(),
            ),
        ],
    )?;
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
    // SDK shutdown is transport cleanup, NOT evidence of actual agent exit.
    runtime.block_on(hooks.shutdown())?;
    result
}
