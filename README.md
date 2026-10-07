# herdr-ahp

A generic, observe-only Agent Hooks Protocol hook
server that reports an agent's state to [HerdR](https://herdr.dev/docs/add-herdr-support/).
It uses the published `agenthooksprotocol` 0.1.0 **server dispatcher and stdio
adapter**, not a host-specific hook format. It has no host application state or
agent-specific integration code. This repository has no GitHub remote.

## Build and configure

Rust 1.89+ is required (standard-library file locking). Direct dependencies are
exactly pinned, SDK default features are disabled, and `Cargo.lock` is committed.
Tokio/process SDK support is test-only; the release bridge uses standard threads.

```sh
cargo build --release --locked
# Binary: target/release/herdr-ahp
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Copy `examples/adapter-context.json` and `examples/registration.json`, replacing
all example paths, identities, and resume options. `source` identifies **your
integration**, remains stable across restarts, and must not use HerdR's reserved
`herdr:` prefix. `agent` is the actual agent's user-visible name, not the bridge
name or another vendor's name. AHP event `source` is separate; optional
`event_source` restricts it but is not authentication.

The host owns adapter context. Set `resume_argv` to the actual command that reopens
the current session, including model and permission options. `{session_id}` is
substituted in each argument, never evaluated by a shell. An empty array disables
resume reporting. The first argument must be a plain command on PATH; arguments
cannot contain apostrophes/control characters; maximum 64 arguments/8 KiB.
Invalid substituted argv is omitted **without discarding state**. Resume needs
HerdR 0.9.2+. Restart the hook with new context if host options change; a session
switch alone does not require restarting it.

Create the parent directory of `sequence_file` first. Use a private local,
non-symlink path unique to the `(HerdR server, pane, source)` tuple; do not share it
with another pane or reporter. Keep this file across sessions, upgrades and
restarts. Do not delete it to fix ownership conflicts. The host must materialize
pane-dependent paths; config does not expand environment variables or `~`.

## Canonical SDK registration and grants

Load `examples/registration.json` as the SDK `Registration`. Use the built-in
**persistent stdio registered process transport**, not `per_event`. The hook
inherits its environment from the host. SDK 0.1.0's process backend supports
`command`, `args`, `cwd`, and `lifecycle`; it does **not** apply an `env` map.
Only a trusted local host should have access to the child stdin. No bearer token
or network listener is involved.

Registration selects routes; it does not grant authority. The host separately
declares exact **observe** grants for every subscribed event:

```rust
use agenthooksprotocol::{generated as g, hooks::{Hooks, HooksOptions}, registration::Registration};
let registration: Registration = serde_json::from_str(registration_json)?;
let options = HooksOptions::from_declarations("urn:example:agent", [
    (g::EventType::SessionStart, g::capability::observe()),
    (g::EventType::TurnStart, g::capability::observe()),
    (g::EventType::TurnEnd, g::capability::observe()),
    (g::EventType::ToolPermissionRequest, g::capability::observe()),
    (g::EventType::ToolPermissionResolved, g::capability::observe()),
    (g::EventType::UserElicitationRequest, g::capability::observe()),
    (g::EventType::UserElicitationResult, g::capability::observe()),
])?;
let hooks = Hooks::new(registration, options)?;
```

The host needs SDK feature `tokio-process` and its own Tokio runtime to deliver
registered process calls. Use its typed event boundaries and await observation
delivery. `tests/stdio.rs::sdk_registered_process_observe_smoke` demonstrates this
with a real SDK client/process backend. Do not add intercept capabilities,
wildcards, inline content grants, or `failurePolicy` to these observe-only routes.
The bridge supports observations only; discovery/intercept requests are rejected
by the SDK dispatcher. JSON is framed as bounded **newline-delimited** objects
(not Content-Length framing). Successful observations produce no stdout frame.
All diagnostics go to stderr, and CLI stdout/stderr are discarded.

## Event → state contract

| AHP event | HerdR state |
| --- | --- |
| `session.start` | `idle`, with the new session's resume argv |
| `turn.start` | `working` |
| `turn.end` | `idle` (including failed/cancelled turns) |
| `tool.permission.request` | `blocked`, message `Tool permission required` |
| `user.elicitation.request` | `blocked`, message `User input required` |
| `tool.permission.resolved`, `user.elicitation.result` | `working` |
| all others, including `session.end`, `turn.finish.before`, `user.attention` | no state change, **no release** |

Each state report uses `pane report-agent <pane> --source ... --agent ... --state
... --seq ...`. Blocked reports add `--message`. Valid configured resume data adds
`--agent-session-id <session> -- <argv...>` to **the same report**, satisfying
HerdR's ownership-before-resume rule. Messages are fixed labels: prompt/tool
content is never copied into HerdR.

The host must provide a session ID on mapped events and deliver an ordered stream
for the foreground agent. If it multiplexes child agents, route only foreground
events or set `session_id` to restrict a fixed session. This is a state projection,
not a task scheduler: it does not count overlapping approval requests, infer
whether an elicitation cancellation ends a turn, or infer work from tool progress.
The host should emit `turn.end` after a stopped turn. Informational attention,
pre-finish interceptions, and intermediate tool completion do not imply readiness.

## Reporting, ownership and actual exit

Reporting activates only with **all** of `HERDR_ENV=1`, `HERDR_BIN_PATH`,
`HERDR_PANE_ID`, and `HERDR_SOCKET_PATH` set/nonempty. Outside HerdR, no CLI,
sequence file, or reporter thread is created; the AHP server still accepts events.
The portable CLI boundary is used, not platform-specific socket code.

A worker serializes reports with a 300 ms subprocess timeout and one replaceable
pending-state slot. While a command is in flight, newer changes replace older
pending changes. Successful identical reports are deduplicated. An uncertain
failure invalidates that baseline, so a later old state is sent again. CLI
failures are ignored. Sequence-storage/worker startup failures disable reporting
with a stderr warning, without breaking protocol service. There is no retry queue.

An exclusive standard-library file lock prevents concurrent writers using the
same sequence file. Every attempted report/release consumes a persisted sequence
number `max(previous + 1, unix_time_ms)` **before** spawning the CLI. This survives
clock rollback and process restarts. Corrupt storage fails closed, not reset to
zero. Use durable local storage; unexpected power loss/storage failure may require
operator repair. Different files for the same source/pane defeat this guarantee.
Source ownership is claimed by state reports, not merely by starting the bridge.

**Transport shutdown is not agent exit.** Default `release_on_eof: false` never
releases on `session.end`, session switches, SDK shutdown, hook restarts, framing
errors, or drop. HerdR's idle-shell detection is a fallback, not a substitute for
an adapter exit contract. The host must arrange a real quit signal:

* If clean stdin EOF is guaranteed to occur **only when the actual agent quits**,
  set `release_on_eof: true` and close stdin on that lifecycle boundary. The hook
  drains the latest state, then sends a higher-sequence source-scoped
  `pane release-agent`, including after uncertain state acceptance.
* Otherwise leave it false. A host-side lifecycle supervisor must call the explicit exit command
  **after the actual agent quits and this hook stops**:
  `herdr-ahp --config /absolute/path/to/adapter-context.json --release`.
  It uses the same locked durable sequence store and source-scoped release;
  reporting failures remain non-fatal and no stdout is emitted. Do not enable the flag for ordinary SDK process shutdown:
  SDK 0.1.0 shutdown can kill its child, so it cannot guarantee graceful release.

SIGKILL/crashes cannot execute cleanup; no claim of unconditional release is made.
The CLI timeout bounds external commands, not OS process creation, filesystem
sync, scheduler latency, or kernel kill/reap latency. Avoid slow/network sequence
storage. Reporter shutdown joins its owned thread; it does not detach children.

## Smoke tests

```sh
# SDK registered process transport and actual observe-only grants:
cargo test --locked --test stdio sdk_registered_process_observe_smoke
# Fake external CLI boundary: working → blocked → idle, resume argv, seq/release:
cargo test --locked --test stdio maps_states_resume_and_actual_exit_opt_in
# Coalescing, timeout, uncertain acceptance, storage and ownership failures:
cargo test --locked
```

Unix boundary tests use a shell script as a fake external CLI; they do not need a
real pane. Library tests are portable. For a manual no-HerdR stdio smoke, write a
valid adapter context with real paths, then (stdin terminates cleanly):

```sh
printf '%s\n' '{"jsonrpc":"2.0","method":"hooks/observe","params":{"protocolVersion":"draft","event":{"id":"evt-smoke","source":"urn:example:agent","type":"turn.start","time":"2026-10-07T12:00:00Z","session":{"id":"session-smoke"},"turn":{"id":"turn-smoke"},"trigger":"user","items":[]}}}' \
  | HERDR_ENV=0 target/release/herdr-ahp --config /absolute/path/to/adapter-context.json
```

Expected stdout: empty, exit status 0. In a **real HerdR pane**, register the hook
with your agent, then check `herdr agent list` while starting a turn, blocking on a
permission, finishing, switching sessions, and quitting. Test restore by restarting
the HerdR server and verify the configured command resumes the session. No live
pane/restore testing has been performed here.

See `DEPENDENCY_AUDIT.md` for checksum/advisory checks and the explicitly
**incomplete dependency provenance review**. This implementation is locally
validated, not a dependency-security approval or live HerdR certification.
