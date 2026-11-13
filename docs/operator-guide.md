# tpt-portus operator guide

This guide covers the current state of the code. Several parts of the design are not built
yet, and this guide says so where it matters. Read the "Known limitations" section before
running anything outside a test environment.

## Configuration

Configuration is loaded from `portus.toml`. The file at the repository root holds the default
values. Missing keys fall back to the same defaults.

Every leaf key can be overridden with an environment variable. Build the name from the
dotted path, upper-case it, and join the parts with `_`, prefixed by `PORTUS_`:

| Key | Variable |
|---|---|
| `portus.node_id` | `PORTUS_NODE_ID` |
| `portus.statera.endpoint` | `PORTUS_STATERA_ENDPOINT` |
| `portus.relayer.rate_limit_rps` | `PORTUS_RELAYER_RATE_LIMIT_RPS` |
| `portus.chains.bitcoin.rpc_url` | `PORTUS_CHAINS_BITCOIN_RPC_URL` |

Overrides keep the type of the default. A non-numeric value for an integer key is an error.
Variables with the prefix that match no key are ignored.

### Validation

- The relayer fee split (`fee_split_relayer_pct`, `fee_split_liquidity_pct`,
  `fee_split_slash_reserve_pct`) must sum to 100.
- `relayer.min_acknowledgments` must be at least 1.

A future daemon must refuse to start if validation fails. No daemon exists yet.

### Chain identity

The `portus.toml` chain sections do not carry numeric `chain_id` values. The design identifies
chains with a tagged `ChainId`, because the old numeric ids were wrong (DESIGN.md §4.2).

### Dispute windows

`dispute_window_secs` under each chain section is the authoritative window. Statera reads it
through `GetDisputeWindow` and must embed the same value in every plan. Changing it bumps the
config version, and plans built against the old value are then rejected. Coordinate the change
with Statera before deploying it.

## Running

The crate builds a library only. There is no daemon binary yet, so the Docker image and
Kubernetes manifests in `deploy/` are skeletons and will not run as-is.

Requirements to build:

- Rust stable
- `protoc` on `PATH` (used by `build.rs`)

```sh
cargo build
cargo test
```

## Metrics

The engine updates these metrics when it is given a `Metrics` instance (`src/metrics.rs`):

| Metric | Meaning | Alert |
|---|---|---|
| `portus_intents_total` | Plans accepted | |
| `portus_rollbacks_total` | `ROLLED_BACK` markers written | Rollback rate above 5% over 10 minutes (DESIGN.md §12.2) |
| `portus_dispute_window_races_detected_total` | Valid proofs that arrived after the window closed | Any increment |
| `portus_proof_verification_failures_total` | Proofs that failed verification | Spike (not yet incremented) |
| `portus_liquidity_available` | Available liquidity (not yet set) | |

The two duration histograms in §12.2 are not implemented. Alert thresholds belong in the
deployment's monitoring configuration, not in this repository.

## Operational behaviour

**Rate limits.** Each relayer, identified by its stake reference, may submit proofs at
`rate_limit_rps` per second. Excess submissions are rejected with "rate limited". The limiter
keeps one bucket per relayer and never evicts them.

**Dispute windows.** When a plan times out, its window opens. A valid proof inside the window
still succeeds the intent. Once the window closes, a proof is rejected even if no one has
recorded the close. Rollback then waits for the `ROLLED_BACK` marker.

**Idempotency.** Resubmitting the same plan is safe. Submitting a different plan under the
same `intent_id` is rejected as a replay.

## Known limitations

These are the gaps an operator must account for before production use.

- **No durable storage.** `InMemoryEngine` loses all intents on restart. Records in flight are
  forgotten, and any rollback decision made before the restart is lost.
- **No proof verification wired in.** The EVM, Solana, and Bitcoin verifiers exist as library
  functions. The engine does not call them, so `SubmitProof` never accepts a proof.
- **No light clients.** The EVM header root, the Cosmos Tendermint validator set, and the
  Bitcoin difficulty schedule are not verified. The caller must trust them.
- **No signing.** Receipts carry an empty `portus_signature`. The HSM integration is not
  built, and no key material is handled.
- **No relayer network.** There is no NATS dispatch and no relayer claim flow. Plans are not
  delivered to relayers.
- **Rollback request not sent.** The `RollbackRequest` to Statera after the window is not
  implemented, so Statera does not unlock funds.
- **No mTLS.** The gRPC server listens without transport security.
- **Watch streams unimplemented.** Clients must poll `GetIntentStatus`.

Do not route real funds through this build.
