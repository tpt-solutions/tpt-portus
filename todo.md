# tpt-portus — Project TODO

Tracks implementation of `spec.txt` (Design Document v0.3) end to end, organized into
phases. Each item cites the spec section it comes from so the checklist stays
traceable back to the source document.

**License:** Dual-licensed under **MIT OR Apache-2.0**, © TPT Solutions.

---

## Phase 0 — Repository & Tooling Setup

- [x] Initialize Cargo workspace (`Cargo.toml`)
- [x] Add `LICENSE-MIT` and `LICENSE-APACHE` files (copyright TPT Solutions)
- [x] Set `license = "MIT OR Apache-2.0"` in `Cargo.toml` manifest
- [x] Decide and apply SPDX-License-Identifier header convention across source files
- [x] Scaffold directory structure per §15.C (`src/`, `proto/`, `tests/`, `deploy/`, `docs/`)
- [x] Set up `proto/portus/v1/portus.proto` build pipeline (prost/tonic codegen)
- [x] Set up CI: build, test, `clippy`, `fmt` checks
- [x] Write `README.md`
- [x] Promote `spec.txt` into `DESIGN.md` per §15.C, with an SPDX identifier stated (closing the spec's own license TODO)
- [x] Add `deploy/docker/Dockerfile` skeleton
- [x] Add `deploy/kubernetes/portus-deployment.yaml` + `portus-service.yaml` skeletons
- [x] Implement `portus.toml` defaults per §7.1 (`[portus]`, `[portus.statera]`, `[portus.relayer]`, `[portus.chains.*]`, `[portus.rollback]`, `[portus.security]`)
- [x] Implement `PORTUS_`-prefixed environment variable overrides per §7.2
- [x] Open a tracked dependency-risk register entry for `tpt-telos` compiler-backend soundness as a trust assumption, separate from Portus's own audit (§6.2, §14)

---

## Phase 1 — Foundation (spec §13, Months 1–3)

- [x] Scaffold core module structure per §15.C: `types`, `error`, `engine`, `router/`, `dispatcher/`, `verifier/`, `endpoint_gen/`, `rollback/`, `grpc/`, `config`
- [x] Implement `ChainId` tagged enum: `Evm`, `Cosmos`, `Solana`, `Bitcoin` variants (§4.2)
- [x] Implement `AssetDescriptor` + cross-chain decimal conversion rule (§4.2.1)
  - [x] Flag the dust-threshold `ensures` contract as blocked pending `tpt-telos` fixed-point reasoning support (§4.2.1, §14)
- [ ] Implement `IntentExecutionPlan` struct with `#[telos::verify]` contract annotations, incl. `source_finality_proof` field (§4.2)
- [x] Implement `ExecutionReceipt` + `ExecutionStatus` enum incl. `DisputeWindowOpen` variant (§4.3)
- [x] Implement `PortusError` taxonomy (§10.1)
- [x] Write full `.proto` service definition: `PortusService` RPCs + all messages — `ChainId`, `AssetDescriptor`, `IntentExecutionPlan`, `ExecutionReceipt`, `AcceptanceAck`, `IntentIdQuery`, `ChainQuery`, `DisputeWindowConfig`, `ProofSubmission`, `ProofAck`, `HealthRequest`/`HealthResponse` (§8.1)
- [x] Reserve field numbers 20–29 on `IntentExecutionPlan` and `ExecutionReceipt` for future additions (§8.1)
- [x] Generate gRPC codegen and implement the `PortusEngine` async trait (§8.2)
- [x] Implement address format validators for EVM, Solana, Cosmos
- [x] Implement Router module: single-hop routing + `(gas*0.4 + latency*0.4 + success_rate*0.2)` scoring (§5.1)
- [x] Implement `GetDisputeWindow` RPC; make Portus the sole source of truth for `dispute_window_secs`, with plan-vs-config drift rejected rather than silently overridden (§4.5)
- [ ] Implement source-chain finality check: verify `source_finality_proof` independently in the `ExecuteIntent` handler rather than trusting Statera's say-so (§4.6)
- [ ] Set up mTLS 1.3 transport between Statera and Portus daemons (§4.4, §6.3)
- [ ] Build a mock `tpt-statera` harness for integration testing (§13 Phase 1)
- [ ] Decide the concrete threshold-signature scheme (threshold-Ed25519 vs. BLS) for Statera plan signing (§6.3, §14 open item)

---

## Phase 2 — Execution & Relaying (spec §13, Months 4–6)

- [ ] Implement NATS-based Dispatcher module: publish plans to topic-based pub/sub (§5.2)
- [x] Implement relayer independence/Sybil-resistance check: dedupe acknowledgments by distinct operator identity + distinct ASN before counting toward the 3-of-3 threshold (§5.2)
- [ ] Implement backup relayer pool escalation on claim timeout (`timeout_block_height / 2`) (§5.2)
- [x] Implement relayer fee-split economics: 70% relayer / 20% liquidity pool / 10% slash-reserve top-up, with pro-rata gas-cost refund instead of relayer pocketing the difference (§5.2.1) Pure split and gas-refund functions; the refund is the budgeted minus actual gas cost, not a pro-rata share of it.
- [x] Implement stake-to-claimable-amount cap (`amount <= staked_bond * collateralization_ratio`, default 3x), falling through to backup pool when exceeded (§5.2.1)
- [ ] Integrate `tpt-eve` relayer agents: claim → execute → submit-proof flow
- [ ] Implement Proof Verifier: EVM (Merkle Patricia Trie + light client header, optimistic tier) (§5.3)
- [ ] Implement Proof Verifier: Cosmos (IBC Tendermint light client, fast-finality tier) (§5.3)
- [x] Implement the Rollback Engine dispute-window state machine per Appendix D (adds `RollbackPending` between window close and the marker write; see `src/rollback/dispute_window.rs`) (`Pending → InFlight → DisputeWindowOpen → Success/Failed/RolledBack`)
- [ ] Implement two-stage timeout: timeout fires into `DisputeWindowOpen` (no funds move), window elapses with no late `Success` before the on-chain `ROLLED_BACK` marker is written and `RollbackRequest` is emitted to Statera (§5.5.1)
- [x] Write the exhaustive `IntentState × IntentEvent` transition test asserting every pair either matches an explicit arm or returns `InvalidTransition` — never panics, never silently no-ops (§11.1, Appendix D test obligation)
- [ ] Implement `SubmitProof` protections: stake-reference check before verification, per-relayer rate limit (default 10 req/sec), queued/load-shed verification under overload (§6.4)
- [ ] Generate the first `tpt-telos` endpoint contract: CosmWasm for Cosmos (§13 Phase 2)
- [x] Implement replay protection / idempotency: duplicate `intent_id` submissions return the existing receipt rather than re-executing (§4.4, §10.2). Duplicates of an identical plan return an accepted `AcceptanceAck`, and the receipt is read via `GetIntentStatus`, matching the RPC signatures. Different plans under the same id are rejected as `ReplayAttack`.

---

## Phase 3 — Multi-Chain & Hardening (spec §13, Months 7–9)

- [ ] Generate Solana endpoint program (Anchor/Rust) with account-model formal contract (§5.4)
- [ ] Generate EVM endpoint contract (Solidity via `tpt-telos` FFI) with account-model formal contract (§5.4)
- [ ] Generate Bitcoin endpoint (Script/Miniscript) with the UTXO-native invariant: `OP_RETURN` nonce replay protection, escrow-input full-spend guarantee (§5.4, corrected from v0.1)
- [ ] Implement Proof Verifier: Solana (Ed25519 validator signature set, fast-finality tier) (§5.3)
- [ ] Implement Proof Verifier: Bitcoin (SPV Merkle block proof + header-chain PoW, slow-finality tier) (§5.3)
- [ ] Implement 2-hop routing (e.g., Ethereum → Cosmos → Solana) when no direct route exists (§5.1)
- [ ] Run formal verification via `tpt-telos` on all endpoint contracts before deployment — no unverified code on-chain (§11.3)
- [ ] Run chaos testing: Ethereum reorg (3 blocks) during in-flight transfer, Byzantine relayer submitting fraudulent-but-valid-looking proofs, Statera↔Portus network partition (§11.4)
- [ ] Run Sybil relayer attack simulation (multiple acknowledgments from one operator behind different addresses) confirming the diversity check rejects them (§5.2, §11.4)
- [ ] Integrate HSM for key management: HD key derivation, private keys never leave HSM (§6.3)
- [ ] Add dispute-window race integration test: inject a proof 1s before and 1s after `dispute_window_secs` closes, confirm outcome differs correctly (§11.2)
- [ ] Add concurrent intent execution load test: 100 simultaneous cross-chain transfers (§11.2)
- [ ] Add relayer failure simulation test: kill 2 of 3 relayers mid-execution (§11.2)

---

## Phase 4 — Production & Ecosystem (spec §13, Months 10–12)

- [ ] Deploy to mainnet: Ethereum, Cosmos Hub, Solana, Osmosis, Arbitrum
- [ ] Integrate with `tpt-payments` for chain-abstracted merchant/user flows
- [ ] Instrument Prometheus metrics: `portus_intents_total`, `portus_execution_duration_seconds`, `portus_relayer_claim_duration_seconds`, `portus_proof_verification_failures_total`, `portus_rollbacks_total`, `portus_dispute_window_races_detected_total`, `portus_liquidity_available` (§12.2)
- [ ] Instrument OpenTelemetry tracing spans for every intent, including the dispute-window wait (§12.2)
- [ ] Configure alerting: rollback rate >5%/10min, no relayer claims >60s, proof verification failure spike, any dispute-window-race increment (§12.2)
- [ ] Configure Kubernetes rolling upgrades (`maxSurge: 1`) with `protocol_version` mismatch rejected outright (§12.3)
- [ ] Publish a public SDK for third-party dApps to consume `tpt-portus`
- [ ] Commission an external security audit, explicitly prioritizing the §5.5.1 dispute-window/rollback mechanism and the §5.2 relayer independence check as the highest-priority review items (per the spec's own instruction)
- [ ] Economically model and stress-test the §5.2.1 fee-split percentages against relayer profitability under realistic gas-price volatility before mainnet (§14)
- [ ] Write a manual-reconciliation runbook for destination-chain outages that outlast the dispute window (§10.2, §14 residual risk)
- [ ] Write an incident-response plan for third-party liquidity pool exploits affecting in-flight transfers, given Portus doesn't control the pools it routes through (§2.2, §14)
- [ ] Publish documentation site + operator guide + developer onboarding guides

---

## Reference

- Source of truth: `spec.txt` (Design Document v0.3)
- Dependencies (external repos, not built here): `tpt-telos`, `tpt-statera`, `tpt-eve`, `tpt-anima`
