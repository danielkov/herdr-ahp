# Dependency audit

## Decision: HOLD — partial audit, not security approval

This is an addition to an empty dependency baseline, not an update. The secure Rust dependency changes procedure was read in full. Work took place in the existing feature worktree (`main`, initially unborn), without changing its manifest or lockfile. No repository instructions, contribution guidance, or release policy existed at initial inspection. Only this report is written in the repository.

The requested versions have matching published archive checksums and the direct source provenance checks below passed. **Registry ownership/publisher verification and full transitive source/provenance review remain incomplete. Under the procedure, all four final direct crate additions remain HOLD.** A clean advisory scan does not override that decision. No dependency was approved merely because its checksum matched.

## Scope and resolution

Final direct dependencies, reconciled against the current `Cargo.toml` and `Cargo.lock`:

- Runtime: `agenthooksprotocol =0.1.0`, defaults disabled; `serde =1.0.228`, defaults disabled, `derive,std`; `serde_json =1.0.145`, defaults disabled, `std`.
- Development only: another declaration of `agenthooksprotocol =0.1.0`, defaults disabled, enabling `tokio-process`; `tokio =1.48.0`, defaults disabled, `rt,time`.
- `futures` was **not added**. Tokio is a non-optional dev-dependency, not an optional runtime integration.

The **final lockfile contains 122 registry package/version pairs**, including development and target-specific packages; all are new against the empty baseline. Every final pair and checksum matches the previously screened temporary resolution. Final `cargo metadata --locked` confirms the unified features below. Normal builds and tests can have different active features because the process adapter is enabled by the dev-dependency.

### Final unified features (including development dependencies)

- SDK: `tokio-process`; no `reqwest`, `axum`, or `interop`.
- `serde`: `default,derive,serde_derive,std`; SDK dependencies re-enable defaults despite the direct declaration.
- `serde_json`: `arbitrary_precision,default,std`; SDK requires arbitrary precision and re-enables defaults. Preserve exact-number behavior.
- Tokio: `bytes,default,io-util,libc,mio,process,rt,signal-hook-registry,sync,time,windows-sys`. SDK broadens the direct dev feature list; Tokio's default feature set is empty.

### Earlier hypothetical audit superset (not the final manifest)

The isolated temporary project additionally proposed `futures =0.3.31` with `executor` and optional Tokio with `rt,time,process`. It resolved 127 registry pairs. Exactly five packages from that hypothetical snapshot are absent from the final lockfile: `futures 0.3.31`, `futures-channel 0.3.34`, `futures-executor 0.3.34`, `futures-io 0.3.34`, and `futures-sink 0.3.34`. Their earlier checksum/source screening is extra evidence only; they are not final additions. There are no final packages outside that screened superset.

## Registry and checksum evidence

The crates.io REST endpoints for both the SDK crate and its owners returned HTTP 403. The sparse registry at `https://index.crates.io/` was accessible. Its metadata, Cargo's downloaded registry archives, and the lockfile were compared for every selected registry package: **122/122 final archive SHA-256 values were reverified against the final lockfile and the previously verified registry metadata checksums; none were yanked in the consulted metadata**. The earlier hypothetical superset also passed 127/127 checksum checks. Existing Cargo cache metadata was also used. Registry metadata is not an ownership/publisher identity service and cannot establish those facts.

| Final direct release | SHA-256 | Declared MSRV |
|---|---|---|
| `agenthooksprotocol 0.1.0` | `3cbbee5580a4a6dfbb2e6db3c97fb779ce987c89b4b8ac4dd2e34c7bb9b04af1` | 1.88 |
| `serde 1.0.228` | `9a8e94ea7f378bd32cbbd37198a4a91436180c5bb472411e48b5ec2e2124ae9e` | 1.56 |
| `serde_json 1.0.145` | `402a6f66d8c709116cf22f558eab210f5a50187f702eb4d7e5ef38d9a7f1c79c` | 1.61 |
| `tokio 1.48.0` | `ff360e02eab121e0bc37a2d3b4d4dc622e6eda3a8e5253d5435ecf5bd4c68408` | 1.71 |

The highest declared MSRV among the selected packages is `uuid 1.27.0` at Rust 1.89.0; the observed real manifest declares Rust 1.89. The SDK itself declares 1.88. This is metadata review, not an MSRV build verification.

## Direct provenance and behavior review

Published `.cargo_vcs_info.json` commits were fetched from each declared GitHub repository and compared against packaged files, including `Cargo.toml.orig` against upstream `Cargo.toml`. Cargo-normalized manifests and Cargo's bookkeeping files were excluded from byte comparison.

| Crate | Declared upstream commit | Source comparison |
|---|---|---|
| agenthooksprotocol | `e1b4b2f620d0da1c0a0cfbd29880f1ce1a66be5c` | All 94 compared files identical; no unexplained omissions or mismatches. |
| serde | `a866b336f14aa57a07f0d0be9f8762746e64ecb4` | 11 files identical; all 19 copied `src/core` files match upstream `serde_core/src` at the same commit. Packaged Cargo.lock was not reconciled. |
| serde_json | `efa66e3a1d61459ab2d325f92ebe3acbd6ca18b1` | 85 files identical; packaged Cargo.lock was not reconciled. |
| tokio | `556820ff84030b37e74e11b86b7733f5795770ea` | 522 files identical; packaged Cargo.lock was not reconciled. |

SDK upstream tag `v0.1.0` points to its packaged commit, but is a **lightweight tag**, not a signed tag. The commit author is `agent-hooks-protocol-bot[bot]`; that is Git metadata, not proof of the crates.io publisher identity. Commit signature verification could not complete because GPG was unavailable. Upstream history starts with a bootstrap commit on 2026-08-27, the release commit is dated 2026-10-07, and the cloned default branch includes later fixes; this is a young SDK with post-release activity, not evidence that those fixes are in 0.1.0. Tags/signatures, maintenance histories and ownership for the other direct crates were not fully verified.

SDK inspection: no build script or proc-macro target. Its six binary targets require `interop`, disabled here. Unsafe FFI found in the lifecycle interop binary is therefore outside the selected SDK library configuration; this is not a safety audit of that binary. Packaged files marked executable are two codegen-lock JSON files, generated Rust source, and schemas JSON, not identified native executable payloads. All packaged SDK files match upstream, including these. Generated models and schemas are checked-in source/data; generation is not invoked by a build script. No archive symlinks were found in any selected package.

The library performs schema validation with bundled `schemas.json`, localizes schema references, and caches validators; `jsonschema` has defaults disabled, avoiding its default HTTP/file resolvers. HTTP adapters and selected-body uploads require the SDK `reqwest` feature; it is not enabled. Source review does not establish that caller-supplied transports are safe or remove application responsibility for SSRF/resource limits. Disabling `reqwest` also makes selected-body upload unavailable, including for stdio hooks; implementations must not silently promise that functionality.

Optional SDK process adapter deliberately spawns a caller-supplied Tokio command. It pipes stdin/stdout, inherits stderr, sets kill-on-drop, requires positive frame/deadline limits, bounds request/response framing, and uses timeouts and retirement after interrupted exchanges. Command selection, environment, working directory and inherited diagnostic output are application trust boundaries. No shell/download installation behavior was identified in this reviewed adapter. This is a focused review, not exhaustive reasoning about all SDK paths.

Serde's build script writes a private module in OUT_DIR and runs the configured compiler with `--version`; serde_json's build script selects arithmetic cfgs from target metadata. These observed build-time operations are explained. Derive introduces the `serde_derive` proc macro transitively. Tokio has no build script; Tokio includes extensive unsafe/runtime/process code that was screened, not comprehensively audited. No claim is made that every token scan hit is executable behavior: docs/tests and inactive targets also match.

## Transitive audit: screened, incomplete

All 122 final releases, as part of the earlier 127-release superset, had their normalized manifest metadata (repository, MSRV, targets/build scripts), VCS metadata when present, archive members, and Rust-source behavior patterns inspected programmatically. This enumerates risks but **does not satisfy the required per-release manual provenance/owner/history/source review**. In particular transitive upstream source reconciliation, executable-bit interpretation for every package, opaque payload review, and unsafe/proc-macro reasoning are unfinished. No blanket transitive approval is given.

Build scripts selected in the full lockfile:

`ahash`, `getrandom`, `icu_normalizer_data`, `icu_properties_data`, `libc`, `num-traits`, `parking_lot_core`, `proc-macro2`, `quote`, `ref-cast`, `rustversion`, `serde`, `serde_core`, `serde_json`, `wasm-bindgen`, `wasm-bindgen-shared`, `wit-bindgen`, `zerocopy`.

Proc-macro targets:

`displaydoc`, `ref-cast-impl`, `rustversion`, `serde_derive`, `time-macros`, `wasm-bindgen-macro`, `yoke-derive`, `zerocopy-derive`, `zerofrom-derive`, `zerovec-derive`.

These lists include optional/target-only code, not assertions that every item runs on the current host. The exact complete **final** new release set is below; the five hypothetical-only packages are excluded.

| Package | Version |
|---|---|
| agenthooksprotocol | 0.1.0 |
| ahash | 0.8.12 |
| aho-corasick | 1.1.5 |
| autocfg | 1.5.1 |
| base64 | 0.22.1 |
| bit-set | 0.8.0 |
| bit-vec | 0.8.0 |
| bitflags | 2.13.2 |
| block-buffer | 0.10.4 |
| borrow-or-share | 0.2.4 |
| bumpalo | 3.20.3 |
| bytecount | 0.6.9 |
| bytes | 1.12.1 |
| cfg-if | 1.0.5 |
| cpufeatures | 0.2.17 |
| crypto-common | 0.1.7 |
| deranged | 0.5.8 |
| digest | 0.10.7 |
| displaydoc | 0.2.7 |
| email_address | 0.2.9 |
| errno | 0.3.14 |
| fancy-regex | 0.16.2 |
| fluent-uri | 0.3.2 |
| form_urlencoded | 1.2.2 |
| fraction | 0.15.4 |
| futures-core | 0.3.34 |
| futures-task | 0.3.34 |
| futures-util | 0.3.34 |
| generic-array | 0.14.7 |
| getrandom | 0.3.4 |
| icu_collections | 2.3.0 |
| icu_locale_core | 2.3.0 |
| icu_normalizer | 2.3.0 |
| icu_normalizer_data | 2.3.0 |
| icu_properties | 2.3.0 |
| icu_properties_data | 2.3.0 |
| icu_provider | 2.3.1 |
| idna | 1.1.0 |
| idna_adapter | 1.2.2 |
| itoa | 1.0.18 |
| js-sys | 0.3.106 |
| jsonschema | 0.33.0 |
| lazy_static | 1.5.1 |
| libc | 0.2.190 |
| litemap | 0.8.3 |
| lock_api | 0.4.14 |
| memchr | 2.8.3 |
| mio | 1.2.4 |
| num | 0.4.3 |
| num-bigint | 0.4.8 |
| num-cmp | 0.1.0 |
| num-complex | 0.4.6 |
| num-conv | 0.2.2 |
| num-integer | 0.1.47 |
| num-iter | 0.1.46 |
| num-rational | 0.4.2 |
| num-traits | 0.2.19 |
| once_cell | 1.21.4 |
| outref | 0.5.2 |
| parking_lot | 0.12.5 |
| parking_lot_core | 0.9.12 |
| percent-encoding | 2.3.2 |
| pin-project-lite | 0.2.17 |
| potential_utf | 0.1.6 |
| powerfmt | 0.2.1 |
| proc-macro2 | 1.0.107 |
| quote | 1.0.47 |
| r-efi | 5.3.0 |
| redox_syscall | 0.5.18 |
| ref-cast | 1.0.27 |
| ref-cast-impl | 1.0.27 |
| referencing | 0.33.0 |
| regex | 1.13.1 |
| regex-automata | 0.4.18 |
| regex-syntax | 0.8.11 |
| rustversion | 1.0.23 |
| ryu | 1.0.23 |
| scopeguard | 1.2.0 |
| serde | 1.0.228 |
| serde_core | 1.0.228 |
| serde_derive | 1.0.228 |
| serde_json | 1.0.145 |
| sha2 | 0.10.9 |
| signal-hook-registry | 1.4.8 |
| slab | 0.4.12 |
| smallvec | 1.16.2 |
| stable_deref_trait | 1.2.1 |
| syn | 2.0.119 |
| syn | 3.0.6 |
| synstructure | 0.14.0 |
| time | 0.3.55 |
| time-core | 0.1.9 |
| time-macros | 0.2.32 |
| tinystr | 0.8.4 |
| tokio | 1.48.0 |
| typenum | 1.20.1 |
| unicode-ident | 1.0.26 |
| url | 2.5.8 |
| utf8_iter | 1.0.4 |
| uuid | 1.27.0 |
| uuid-simd | 0.8.0 |
| version_check | 0.9.5 |
| vsimd | 0.8.0 |
| wasi | 0.11.1+wasi-snapshot-preview1 |
| wasip2 | 1.0.4+wasi-0.2.12 |
| wasm-bindgen | 0.2.129 |
| wasm-bindgen-macro | 0.2.129 |
| wasm-bindgen-macro-support | 0.2.129 |
| wasm-bindgen-shared | 0.2.129 |
| windows-link | 0.2.1 |
| windows-sys | 0.61.2 |
| wit-bindgen | 0.57.1 |
| writeable | 0.6.4 |
| yoke | 0.8.3 |
| yoke-derive | 0.8.4 |
| zerocopy | 0.8.61 |
| zerocopy-derive | 0.8.61 |
| zerofrom | 0.1.8 |
| zerofrom-derive | 0.1.8 |
| zerotrie | 0.2.5 |
| zerovec | 0.11.8 |
| zerovec-derive | 0.11.6 |

## Checks and remaining gates

- Temporary `cargo generate-lockfile`, `cargo fetch`, and `cargo metadata --features tokio-process` completed for the hypothetical superset. Final `cargo metadata --locked` completed for the actual manifest; all 122 final archive checksums were reverified.
- `cargo audit --file <temporary Cargo.lock> --json`: zero known vulnerabilities, no warnings, no ignored advisories. RustSec database commit `b8a1a33e246a0a9a3b5f377248c41a503defec74`, last updated 2026-10-07, 1,294 advisories.
- `cargo audit --file Cargo.lock --json` on the final current real lockfile reported zero vulnerabilities and no warnings, with the same database commit and no ignored advisories. Neither scan proves absence of unknown vulnerabilities.
- No feature build/test is duplicated here; those checks belong to the implementation. No dependency or version-bump edits were made by this audit.
- Before accepting: verify registry owners/publishers and release history (including sole-owner risk), direct tags/signatures, packaged Cargo.lock generation where applicable, and complete the same provenance/behavior audit for every newly selected transitive release. If any source/provenance inconsistency is found, retain HOLD.
- Final real manifest/features and lockfile were reconciled with the screened release set. The implementation must inspect its final manifest/lockfile diff and run the feature's smallest useful tests plus `git diff --check`. Added versions outside this set require a new audit. Release/versioning policy was absent at initial inspection; the observed initial package version is 0.1.0, not an audited release bump.

Temporary evidence files are under `/tmp/herdr-dependency-audit/` (inspection/provenance JSON, temporary manifest/lockfile, advisory JSON); they are local working evidence, not durable repository artifacts. Checksums and release enumeration are retained in this report. **Overall result remains HOLD because the mandatory full transitive and publisher audit is incomplete.**
