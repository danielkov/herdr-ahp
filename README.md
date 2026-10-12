# herdr-ahp

Observe-only [Agent Hooks Protocol](https://github.com/agenthooksprotocol/rust-sdk) hook
that reports an agent's state to [HerdR](https://herdr.dev/docs/add-herdr-support/).
Runs as a persistent stdio hook built on the `agenthooksprotocol` server dispatcher.

## Build

Requires Rust 1.89+.

```sh
cargo build --release --locked
cargo test --locked
```

## Configure

Copy `examples/adapter-context.json` and replace the example values:

| Field | Meaning |
| --- | --- |
| `source` | Stable ID of your integration. Must not start with `herdr:`. |
| `agent` | User-visible agent name. |
| `sequence_file` | Absolute path to a private file, unique per (HerdR server, pane, source). Keep it across restarts. Its parent directory must exist. |
| `resume_argv` | Command that reopens a session. `{session_id}` is substituted; no shell is involved. The first argument must be a bare command on `PATH`. Max 64 args / 8 KiB, no apostrophes or control characters. Empty disables resume. Needs HerdR 0.9.2+. |
| `event_source` | Optional. Ignore events whose AHP `source` differs. |
| `session_id` | Optional. Ignore events from other sessions. |
| `release_on_eof` | Release the pane on clean stdin EOF. Default `false`; see [Exit](#exit). |

Paths are used verbatim: no `~` or environment variable expansion.

## Register

Use `examples/registration.json` (session and turn events) or
`examples/registration-with-interactions.json` (adds permission and elicitation
events) with the SDK's persistent stdio transport. Grant observe on every
subscribed event:

```rust
use agenthooksprotocol::{
    capability::{self, HostCapabilities, Observe},
    hooks::{Hooks, HooksOptions},
    registration::Registration,
};
let registration: Registration = serde_json::from_str(registration_json)?;
let options = HooksOptions::new("urn:example:agent", HostCapabilities {
    session_start: Some(capability::session_start().observe()),
    turn_start: Some(capability::turn_start().observe()),
    turn_end: Some(Observe),
    ..Default::default()
});
let hooks = Hooks::new(registration, options)?;
```

`examples/sdk_driver.rs` shows the full interaction grant set. The SDK process
transport does not apply an `env` map, so the hook inherits the host's environment.

## Event mapping

| AHP event | HerdR state |
| --- | --- |
| `session.start` | `idle` |
| `turn.start` | `working` |
| `turn.end` | `idle` |
| `tool.permission.request` | `blocked` — "Tool permission required" |
| `user.elicitation.request` | `blocked` — "User input required" |
| `tool.permission.resolved`, `user.elicitation.result` | `working` |
| anything else | ignored |

Each change runs `herdr pane report-agent` with `--source`, `--agent`, `--state`
and `--seq`. When `resume_argv` is set, the same call carries
`--agent-session-id <session> -- <argv...>`. Prompt and tool content is never sent
to HerdR.

Events must carry a session ID and arrive in order. If the host emits subagent
events, set `session_id` or route only foreground events.

## Reporting

Reporting is enabled only when `HERDR_ENV=1` and `HERDR_BIN_PATH`, `HERDR_PANE_ID`
and `HERDR_SOCKET_PATH` are all set. Otherwise the hook accepts events and does
nothing.

- One worker thread runs the CLI with a 300 ms timeout. State changes that arrive
  while a call is in flight are coalesced to the newest.
- A repeat of the last successfully reported state is skipped. A failed or
  timed-out call clears that, so the next state is always sent.
- `--seq` is `max(previous + 1, unix_time_ms)`, written to `sequence_file`
  before each call. An exclusive file lock stops a second hook from using the
  same file; a locked or corrupt file disables reporting.
- Diagnostics go to stderr. The hook never writes to stdout for observations.

## Exit

Stopping the hook does not release the pane. Pick one:

- **Clean EOF means the agent quit:** set `release_on_eof: true`. On clean stdin
  EOF the hook sends the latest state, then `pane release-agent`.
- **Otherwise:** after the agent quits and the hook stops, run
  `herdr-ahp --config <adapter-context.json> --release`.

The SDK's `shutdown()` kills the child, so it never counts as a clean EOF.

## Testing

```sh
cargo test --locked
```

The tests in `tests/stdio.rs` (Unix only) use a shell script in place of the HerdR CLI.

Manual smoke test outside HerdR (expect no output, exit 0):

```sh
printf '%s\n' '{"jsonrpc":"2.0","method":"hooks/observe","params":{"protocolVersion":"draft","event":{"id":"evt-smoke","source":"urn:example:agent","type":"turn.start","time":"2026-10-07T12:00:00Z","session":{"id":"session-smoke"},"turn":{"id":"turn-smoke"},"trigger":"user","items":[]}}}' \
  | HERDR_ENV=0 target/release/herdr-ahp --config /absolute/path/to/adapter-context.json
```

`examples/sdk_driver.rs` is a demo host. It reads event JSON lines from stdin,
sends them to the hook through the SDK and prints one ack per event.
`{"shutdown":true}` stops it.

```sh
cargo build --locked --example sdk_driver
HERDR_ENV=0 target/debug/examples/sdk_driver /absolute/path/to/registration.json
```

`scripts/live_smoke.py` runs the driver against a real HerdR in a throwaway named
session, then deletes it:

```sh
cargo build --release --locked && cargo build --example sdk_driver --locked
python3 scripts/live_smoke.py                 # session/turn events
python3 scripts/live_smoke.py --interactions  # plus permission events
```
