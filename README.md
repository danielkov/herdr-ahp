# herdr-ahp

Observe-only [Agent Hooks Protocol](https://github.com/agenthooksprotocol/rust-sdk) hook
that reports an agent's state to [HerdR](https://herdr.dev/docs/add-herdr-support/).
Runs as a persistent stdio hook built on the `agenthooksprotocol` server dispatcher.

## Install

macOS and Linux (x86_64 and arm64):

```sh
curl -LsSf https://github.com/danielkov/herdr-ahp/releases/latest/download/herdr-ahp-installer.sh | sh
```

Windows (x86_64):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/danielkov/herdr-ahp/releases/latest/download/herdr-ahp-installer.ps1 | iex"
```

Or from source (Rust 1.88+):

```sh
cargo install --locked herdr-ahp
```

Prebuilt archives are attached to each
[GitHub release](https://github.com/danielkov/herdr-ahp/releases).

## Setup

Add the hook to your harness's AHP config (`examples/registration.json`):

```json
{
  "protocolVersion": "draft",
  "hooks": [{
    "id": "com.github.danielkov.herdr-ahp",
    "transport": { "type": "stdio", "command": "herdr-ahp", "lifecycle": "persistent" },
    "subscriptions": [{
      "events": ["session.start", "turn.start", "turn.end"],
      "mode": "observe",
      "content": { "default": "metadata" }
    }]
  }]
}
```

`examples/registration-with-interactions.json` also subscribes to permission and
elicitation events, for harnesses that emit them.

### Options

Pass these in the transport's `args`. All are optional.

| Flag | Meaning |
| --- | --- |
| `--agent NAME` | Name shown in HerdR. Defaults to the event's `session.agent.id`, then `agent`. |
| `--source ID` | HerdR integration source. Default `herdr-ahp`. Must not start with `herdr:`. |
| `--event-source URN` | Ignore events whose AHP `source` differs. |
| `--session-id ID` | Ignore events from other sessions. |
| `--release-on-eof` | Release the pane on clean stdin EOF; see [Exit](#exit). |
| `-- COMMAND...` | Resume command, e.g. `-- codex resume {session_id}`. `{session_id}` is substituted; no shell is involved. The first word must be a bare command on `PATH`. Needs HerdR 0.9.2+. |

### Harness grants

A harness built on the Rust SDK grants observe on every subscribed event:

```rust
use agenthooksprotocol::{
    capability::{self, HostCapabilities},
    hooks::{Harness, Hooks, HooksOptions},
};
let options = HooksOptions::new("urn:example:agent", Harness::new("example-host", "0.1.0"), HostCapabilities {
    session_start: Some(capability::session_start().observe()),
    turn_start: Some(capability::turn_start().observe()),
    turn_end: Some(capability::turn_end().observe()),
    ..Default::default()
});
let hooks = Hooks::new(registration, options)?;
```

`examples/sdk_driver.rs` shows the full interaction grant set. The hook inherits
the harness's environment, which includes HerdR's pane variables.

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
and `--seq`. With a resume command, the same call carries
`--agent-session-id <session> -- <argv...>`. Prompt and tool content is never sent
to HerdR.

Events must carry a session ID and arrive in order. If the host emits subagent
events, pass `--session-id` or route only foreground events.

## Reporting

Reporting is enabled only when `HERDR_ENV=1` and `HERDR_BIN_PATH`, `HERDR_PANE_ID`
and `HERDR_SOCKET_PATH` are all set. Otherwise the hook accepts events and does
nothing.

- One worker thread runs the CLI with a 300 ms timeout. State changes that arrive
  while a call is in flight are coalesced to the newest.
- A repeat of the last successfully reported state is skipped. A failed or
  timed-out call clears that, so the next state is always sent.
- `--seq` is the wall-clock time in microseconds, bumped past the previous value
  if the clock stalls.
- Diagnostics go to stderr. The hook never writes to stdout for observations.

## Exit

Stopping the hook does not release the pane. Pick one:

- **Clean EOF means the agent quit:** pass `--release-on-eof`. On clean stdin
  EOF the hook sends the latest state, then `pane release-agent`.
- **Otherwise:** after the agent quits and the hook stops, run
  `herdr-ahp --release` (with the same `--source`, if set).

The SDK's `shutdown()` kills the child, so it never counts as a clean EOF.

## Development

```sh
cargo build --release --locked
cargo test --locked
```

The tests in `tests/stdio.rs` (Unix only) use a shell script in place of the HerdR CLI.

Manual smoke test outside HerdR (expect no output, exit 0):

```sh
printf '%s\n' '{"jsonrpc":"2.0","method":"hooks/observe","params":{"protocolVersion":"draft","event":{"id":"evt-smoke","source":"urn:example:agent","harness":{"name":"example-host","version":"0.1.0"},"type":"turn.start","time":"2026-10-07T12:00:00Z","session":{"id":"session-smoke"},"turn":{"id":"turn-smoke"},"trigger":"user","items":[]}}}' \
  | HERDR_ENV=0 target/release/herdr-ahp
```

`examples/sdk_driver.rs` is a demo host. It reads event JSON lines from stdin,
sends them to the hook through the SDK and prints one ack per event.
`{"shutdown":true}` stops it.

```sh
cargo build --locked --example sdk_driver
HERDR_ENV=0 target/debug/examples/sdk_driver examples/registration.json  # herdr-ahp on PATH
```

`scripts/live_smoke.py` runs the driver against a real HerdR in a throwaway named
session, then deletes it:

```sh
cargo build --release --locked && cargo build --example sdk_driver --locked
python3 scripts/live_smoke.py                 # session/turn events
python3 scripts/live_smoke.py --interactions  # plus permission events
```

## Releasing

Bump `version` in `Cargo.toml`, then:

```sh
cargo publish --locked
git tag v0.1.0 && git push origin v0.1.0
```

The tag runs `.github/workflows/release.yml` (generated by
[dist](https://opensource.axo.dev/cargo-dist/); regenerate with `dist generate`
after editing `dist-workspace.toml`). It builds the archives and creates the GitHub
release with the shell and PowerShell installers.
