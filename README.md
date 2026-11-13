# tpt-portus

Cross-chain intent executor for the TPT ecosystem. `tpt-portus` is a pure executor:
it acts only on signed `IntentExecutionPlan`s from `tpt-statera` and returns signed
`ExecutionReceipt`s. The full design is in [DESIGN.md](DESIGN.md).

**Status:** early development. This repository currently provides the handshake types,
the error taxonomy, configuration loading, and the generated gRPC bindings. Routing,
dispatch, proof verification, endpoint generation, and rollback are stubs.

## Layout

| Path | Contents |
|---|---|
| `src/types.rs` | `ChainId`, `AssetDescriptor`, `IntentExecutionPlan`, `ExecutionReceipt`, `ExecutionStatus` (DESIGN.md §4) |
| `src/error.rs` | `PortusError` (DESIGN.md §10.1) |
| `src/config.rs` | `portus.toml` loading and `PORTUS_` environment overrides (DESIGN.md §7) |
| `src/grpc/` | Bindings generated from `proto/portus/v1/portus.proto` (DESIGN.md §8.1) |
| `src/router`, `dispatcher`, `verifier`, `endpoint_gen`, `rollback` | Module stubs (DESIGN.md §5) |
| `portus.toml` | Default configuration |
| `deploy/` | Dockerfile and Kubernetes manifest skeletons |
| `docs/risk-register.md` | Dependency and trust risks outside this crate's own audit |

## Building

Requires a stable Rust toolchain and `protoc` (protobuf compiler) on `PATH`, because
`build.rs` runs `tonic-build` over the `.proto` file.

```sh
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Configuration

`portus.toml` documents every key and its default. Any key can be overridden with a
`PORTUS_`-prefixed environment variable built from its dotted path, for example
`PORTUS_STATERA_ENDPOINT` or `PORTUS_CHAINS_BITCOIN_RPC_URL`. Relayer fee splits must
sum to 100.

## Licensing

Dual-licensed under either of

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

at your option. Copyright © TPT Solutions.

Every source file carries the SPDX header
`// SPDX-License-Identifier: MIT OR Apache-2.0` (or `#` in YAML, Dockerfile and TOML).
