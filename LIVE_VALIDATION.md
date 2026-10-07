# Isolated live validation

Validated 2026-10-07 with HerdR **0.9.3**, using the rebuilt release binary
and the accepted reduced dependency lockfile. The actual path was:

`SDK demonstration host → canonical registered persistent process transport →
herdr-ahp release binary → HerdR CLI → isolated named server/pane`.

## Isolation and cleanup

Two final named sessions were created solely for validation:

- Minimal session/turn subset: `herdr-ahp-e2e-12081fec4a`, pane `w1:p1`.
- Optional interaction subset: `herdr-ahp-e2e-5814814552`, pane `w1:p1`.

Identical pane IDs are scoped to different named sockets, not the user's pane.
Every control command supplied `--session <new-name>`, with inherited HerdR caller
variables removed. The hook inherited only the environment of the created test
pane. No production pane was stopped, released, inspected, or given resume data.
No production server was stopped or restarted.

Both final tests stopped and deleted only their created named sessions; all four
cleanup operations returned exit status **0**, and the owned server processes were
reaped. Earlier exploratory test sessions were also stopped/deleted successfully.
No test sessions were deliberately left running.

## Observed pane state

`herdr --session <test-name> pane get <created-pane>` returned:

| Event | Minimal test | Optional test |
|---|---|---|
| `session.start` | `sdk-smoke-agent`, `idle` | `sdk-smoke-agent`, `idle` |
| `turn.start` | `working` | `working` |
| `tool.permission.request` | Not subscribed | `blocked` |
| `tool.permission.resolved` | Not subscribed | `working` |
| `turn.end` | `idle` | `idle` |
| New-session `session.start` | Still owned and `idle`, new sequence consumed | Still owned and `idle`, new sequence consumed |

The SDK reported sequential delivery totals **1–4** for the minimal test and
**1–6** for the optional test, with no delivery failures. These acknowledgements
were not used alone as evidence of HerdR state: each state was checked via `pane get`.

After actual demonstration-host exit, the separate `--release` invocation ran.
Both host and release command returned **0**; `pane get` no longer contained an
agent and reported `unknown`. `pane process-info --pane <created-pane>` confirmed
foreground **sleep** was still running, excluding idle-shell fallback as the
reason ownership had cleared.

## Scope and reproducibility

Resume argv was attached to reports only in the isolated test pane, using a harmless
`sh`/`printf` command. Private resume argv/session identity are not exposed by these
`pane get` responses; this test does **not** independently verify stored argv or
prove restart/restore behavior. Session-switch evidence is limited to continued
ownership/readiness and sequence advancement. Live elicitation events were not
exercised; fake-boundary/unit tests cover their mapping. Windows and the declared
minimum compiler were not exercised by this live test.

```sh
cargo build --release --locked
cargo build --example sdk_driver --locked
python3 scripts/live_smoke.py
python3 scripts/live_smoke.py --interactions
```

The opt-in helper performs the isolation/cleanup and retains detailed evidence in
private temporary directories. Final evidence from this run:

- `/var/folders/t9/w5z6dqrs68nfb_spxn62h32w0000gn/T/herdr-ahp-live-qgdjwvxm/evidence.json`
- `/var/folders/t9/w5z6dqrs68nfb_spxn62h32w0000gn/T/herdr-ahp-live-t0r2pwui/evidence.json`

Release binary SHA-256: `274d44348ed3deec61ffd70c6a8eed095613f4c87145245952fde702e74d3f9f`.

Accepted lockfile SHA-256: `796ba5667b93fedad249d2ecdfd24faf9637927e188b53d2e23628cf95e10b5d`.

Functional checks against these final pins also passed: **16 Rust tests**,
`cargo fmt --all --check`, and `cargo clippy --all-targets --locked -- -D warnings`.
Dependency acceptance relies on the supplied vetted exact-release baseline and
approved SDK-family provenance, as distinguished in `DEPENDENCY_AUDIT.md`.
