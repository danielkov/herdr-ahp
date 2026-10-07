# Dependency audit

## Decision: APPROVE reduced selection using the supplied previously vetted baseline

The initial, independently resolved graph was **not approved**: its full transitive provenance and behavior review was incomplete. The final manifest and lockfile now reuse the exact releases from the previously vetted baseline supplied for this review. An isolated resolution seeded with that baseline introduced **zero additional registry releases**. This approval relies on that supplied baseline and the separately supplied approved SDK dependency family; it is **not a claim that this review independently completed a full source/security audit of all 122 packages**. If the baseline's prior vetting is not accepted, retain HOLD until those reviews are supplied or completed.

The secure Rust dependency changes procedure was read in full. This is a dependency addition in an existing feature worktree, not a standalone dependency-update exercise. No application source, tests, examples, or upstream project files were changed by the audit. No release/versioning policy was found at initial inspection; package version remains `0.1.0`.

## Final actual graph

- Runtime: `agenthooksprotocol =0.1.0`, defaults disabled; `serde =1.0.229`, defaults disabled, `derive,std`; `serde_json =1.0.151`, defaults disabled, `std`.
- Development: `agenthooksprotocol =0.1.0`, defaults disabled, `tokio-process`; `tokio =1.53.1`, defaults disabled, `rt,time`.
- **122 registry package/version/checksum tuples**, including development and target-specific packages, match the supplied baseline exactly. All 122 downloaded archive SHA-256 values match the final lockfile. No git/path dependency overrides were introduced.
- The earlier direct pins `serde 1.0.228`, `serde_json 1.0.145`, and `tokio 1.48.0` were replaced in the temporary snapshot before applying the reduced result. Transitives were preserved from the baseline rather than regenerated with latest compatible versions. This eliminates the extra provenance surface of the initial selection, not the intrinsic risks of the baseline.
- No selected crate declares an MSRV above this project's Rust `1.89`. This metadata check is not a compiler compatibility test.

Final unified features from `cargo metadata --locked` (including development dependencies):

| Crate | Features |
|---|---|
| SDK | `tokio-process` (no `reqwest`, `axum`, or `interop`) |
| serde | `default,derive,serde_derive,std` |
| serde_json | `arbitrary_precision,default,std` |
| Tokio | `bytes,default,io-util,libc,mio,process,rt,signal-hook-registry,sync,time,windows-sys` |

The SDK re-enables Serde/JSON defaults and exact-number support. Normal builds and tests can have different active features because the process adapter is enabled through a dev-dependency. No futures dependency was added.

## Provenance evidence and limits

Separately supplied exact-release provenance approval covers `agenthooksprotocol 0.1.0`, `fluent-uri 0.3.2`, `fraction 0.15.4`, `jsonschema 0.33.0`, and `referencing 0.33.0`. The supplied `reqwest 0.12.28` evidence is outside the final selected graph. Published archive checksums, registry owners/release history, declared VCS revisions, and packaged-source reconciliation were reused for this family. Shared workspace files and the reqwest tag manifest discrepancy were reconciled in that evidence rather than treated as unexplained source changes.

The crates.io owner endpoints now work with the audit User-Agent; the earlier HTTP 403 is no longer an outstanding endpoint-access blocker. Current family owners are `danielkov` (SDK), `yescallop` (fluent-uri), `dnsl48` (fraction), and `Stranger6667` (jsonschema/referencing). These are sole-owner packages, a residual concentration risk. The SDK release metadata returned no publisher identity; current ownership must not be conflated with proof of who published that release. GitHub reports the SDK release commit signature verified; its `v0.1.0` tag is lightweight, not a signed tag. Fluent-uri's release commit is unverified; the shared jsonschema/referencing release tag has verified annotated-tag evidence. These residuals remain visible under the supplied approval.

For the rest of the graph, **exact baseline reuse is evidence of unchanged selection, not fresh validation of owner/publisher identity or upstream source provenance**. This review does not convert that inheritance into an independent provenance claim.

The attempted broader audit downloaded and checked the remaining initial releases, queried owners/history, and compared packaged source with declared upstream revisions. That initial graph still had failed lookups (including initial wasm-family releases), a missing VCS record (`num-cmp 0.1.0`), and source/generated-file reconciliation work. It was not accepted. Evidence from rejected versions must not be cited as approval of their final replacements. A full re-audit of baseline releases would still need to resolve, among other things, `generic-array 0.14.7` source differences and generated/shared workspace files. These are limitations of the independent investigation, not new releases introduced by the reduced resolution.

## Behavior review and remaining application gates

- SDK packaged Rust/source data matches its recorded revision; no build script or proc-macro target. Generated models and schema data are checked in. Its six interop binary targets are disabled; the unsafe FFI in those targets is not selected.
- The selected SDK library validates bundled schemas. JSON Schema HTTP/file resolvers are disabled. SDK HTTP adapters and selected-body upload require its disabled `reqwest` feature; selected-body upload must not be promised by the bridge.
- The dev-only process adapter intentionally spawns a caller-supplied Tokio command with piped stdin/stdout, inherited stderr, positive framing/deadline bounds, timeouts, kill-on-drop, and interrupted-exchange retirement. Command selection, environment, working directory, and inherited diagnostic output remain application trust boundaries. No installer/download behavior was identified in this adapter.
- Initial graph build-script and proc-macro surfaces were enumerated and token-screened. Reviewed compiler probes, generated private modules, target cfg selection, schema hashing, and target-specific bundled object copying are explainable operations. Runtime unsafe code and proc macros were screened, not comprehensively proven safe. The final replacements inherit baseline review; initial-version screening is not a review of those replacements.
- Keep command/endpoint trust, response/frame limits, validation failure handling, and exact-number behavior covered by feature tests. Functional builds/tests and live integration validation are owned by the implementation, not duplicated by this audit. After the final dependency adjustment, the implementation must rerun its smallest useful build/test checks.

## Checks

- Temporary baseline-seeded resolution, followed by final `cargo metadata --locked`: passed; zero extra release/checksum tuples against the supplied baseline.
- Final archive SHA-256 verification: **122/122 match** the final lockfile.
- `cargo audit --file Cargo.lock --json`: **zero known vulnerabilities, no warnings**, RustSec commit `b8a1a33e246a0a9a3b5f377248c41a503defec74`, 1,294 advisories. This does not establish absence of unknown vulnerabilities.
- `git diff --check` for the final manifest/lockfile/report: passed.
- Final lockfile SHA-256: `796ba5667b93fedad249d2ecdfd24faf9637927e188b53d2e23628cf95e10b5d`.

Local, non-durable evidence: `/tmp/herdr-reduced-resolution/` contains the temporary and final metadata, zero-release delta, checksum inventory, and advisory report; `/tmp/herdr-final-dependency-audit/` retains the rejected initial graph's broader audit and behavior screening. Supplied family provenance was reused from the parent review. The audit stage created no commits. Final implementation builds/tests and isolated live checks subsequently passed; see `LIVE_VALIDATION.md`.

## Exact final release inventory

Every tuple below reuses the supplied baseline, not an independently completed fresh provenance audit.

| Package | Version |
|---|---|
| `agenthooksprotocol` | `0.1.0` |
| `ahash` | `0.8.12` |
| `aho-corasick` | `1.1.5` |
| `autocfg` | `1.5.1` |
| `base64` | `0.22.1` |
| `bit-set` | `0.8.0` |
| `bit-vec` | `0.8.0` |
| `bitflags` | `2.13.1` |
| `block-buffer` | `0.10.4` |
| `borrow-or-share` | `0.2.4` |
| `bumpalo` | `3.20.3` |
| `bytecount` | `0.6.9` |
| `bytes` | `1.12.1` |
| `cfg-if` | `1.0.4` |
| `cpufeatures` | `0.2.17` |
| `crypto-common` | `0.1.7` |
| `deranged` | `0.5.8` |
| `digest` | `0.10.7` |
| `displaydoc` | `0.2.7` |
| `email_address` | `0.2.9` |
| `errno` | `0.3.14` |
| `fancy-regex` | `0.16.2` |
| `fluent-uri` | `0.3.2` |
| `form_urlencoded` | `1.2.2` |
| `fraction` | `0.15.4` |
| `futures-core` | `0.3.34` |
| `futures-task` | `0.3.34` |
| `futures-util` | `0.3.34` |
| `generic-array` | `0.14.7` |
| `getrandom` | `0.3.4` |
| `icu_collections` | `2.3.0` |
| `icu_locale_core` | `2.3.0` |
| `icu_normalizer` | `2.3.0` |
| `icu_normalizer_data` | `2.3.0` |
| `icu_properties` | `2.3.0` |
| `icu_properties_data` | `2.3.0` |
| `icu_provider` | `2.3.1` |
| `idna` | `1.1.0` |
| `idna_adapter` | `1.2.2` |
| `itoa` | `1.0.18` |
| `js-sys` | `0.3.104` |
| `jsonschema` | `0.33.0` |
| `lazy_static` | `1.5.0` |
| `libc` | `0.2.189` |
| `litemap` | `0.8.3` |
| `lock_api` | `0.4.14` |
| `memchr` | `2.8.3` |
| `mio` | `1.2.2` |
| `num` | `0.4.3` |
| `num-bigint` | `0.4.8` |
| `num-cmp` | `0.1.0` |
| `num-complex` | `0.4.6` |
| `num-conv` | `0.2.2` |
| `num-integer` | `0.1.47` |
| `num-iter` | `0.1.46` |
| `num-rational` | `0.4.2` |
| `num-traits` | `0.2.19` |
| `once_cell` | `1.21.4` |
| `outref` | `0.5.2` |
| `parking_lot` | `0.12.5` |
| `parking_lot_core` | `0.9.12` |
| `percent-encoding` | `2.3.2` |
| `pin-project-lite` | `0.2.17` |
| `potential_utf` | `0.1.6` |
| `powerfmt` | `0.2.0` |
| `proc-macro2` | `1.0.107` |
| `quote` | `1.0.47` |
| `r-efi` | `5.3.0` |
| `redox_syscall` | `0.5.18` |
| `ref-cast` | `1.0.27` |
| `ref-cast-impl` | `1.0.27` |
| `referencing` | `0.33.0` |
| `regex` | `1.13.1` |
| `regex-automata` | `0.4.18` |
| `regex-syntax` | `0.8.11` |
| `rustversion` | `1.0.23` |
| `scopeguard` | `1.2.0` |
| `serde` | `1.0.229` |
| `serde_core` | `1.0.229` |
| `serde_derive` | `1.0.229` |
| `serde_json` | `1.0.151` |
| `sha2` | `0.10.9` |
| `signal-hook-registry` | `1.4.8` |
| `slab` | `0.4.12` |
| `smallvec` | `1.15.2` |
| `stable_deref_trait` | `1.2.1` |
| `syn` | `2.0.119` |
| `syn` | `3.0.4` |
| `synstructure` | `0.13.2` |
| `time` | `0.3.55` |
| `time-core` | `0.1.9` |
| `time-macros` | `0.2.32` |
| `tinystr` | `0.8.4` |
| `tokio` | `1.53.1` |
| `typenum` | `1.20.1` |
| `unicode-ident` | `1.0.24` |
| `url` | `2.5.8` |
| `utf8_iter` | `1.0.4` |
| `uuid` | `1.26.1` |
| `uuid-simd` | `0.8.0` |
| `version_check` | `0.9.5` |
| `vsimd` | `0.8.0` |
| `wasi` | `0.11.1+wasi-snapshot-preview1` |
| `wasip2` | `1.0.4+wasi-0.2.12` |
| `wasm-bindgen` | `0.2.127` |
| `wasm-bindgen-macro` | `0.2.127` |
| `wasm-bindgen-macro-support` | `0.2.127` |
| `wasm-bindgen-shared` | `0.2.127` |
| `windows-link` | `0.2.1` |
| `windows-sys` | `0.61.2` |
| `wit-bindgen` | `0.57.1` |
| `writeable` | `0.6.4` |
| `yoke` | `0.8.3` |
| `yoke-derive` | `0.8.2` |
| `zerocopy` | `0.8.56` |
| `zerocopy-derive` | `0.8.56` |
| `zerofrom` | `0.1.8` |
| `zerofrom-derive` | `0.1.7` |
| `zerotrie` | `0.2.5` |
| `zerovec` | `0.11.8` |
| `zerovec-derive` | `0.11.6` |
| `zmij` | `1.0.23` |
