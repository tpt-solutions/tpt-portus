# tpt-portus — Design Document

**Project:** tpt-portus
**Status:** Draft v0.3 (revised — see Changelog)
**Author:** TPT Architecture Team
**Date:** August 20, 2026
**License:** MIT OR Apache-2.0 (SPDX: `MIT OR Apache-2.0`); see `LICENSE-MIT` and `LICENSE-APACHE`.
**Dependencies:** `tpt-telos`, `tpt-statera`, `tpt-eve`, `tpt-anima`

---

## Changelog from v0.1

- Closed the rollback / late-execution race condition (new §5.5.1).
- Corrected the finality model in the end-to-end walkthrough; split G6 into per-chain latency tiers (new §3.3).
- Defined relayer independence/Sybil-resistance criteria for the 3-acknowledgment threshold (§5.2).
- Clarified custody model: Portus never holds reserves itself; liquidity pools are third-party, and endpoint contracts hold only what's explicitly escrowed per-intent (§2.2, §5.4).
- Fixed section numbering (13 → 14 → 15, no gap).
- Replaced the "Fourier-Motzkin SMT solver" phrasing with an accurate description (§11.3).
- Added relayer fee-split economics (§5.2.1).
- Added rate-limiting/backpressure spec for the public proof-submission surface (§6.4).
- Reframed the Bitcoin invariant in UTXO terms instead of account-balance terms (§5.4).

## Changelog from v0.2 → v0.3

- Replaced the flat `uint32 chain_id` with a tagged `ChainId` type that can represent Cosmos zones (string-identified) and Bitcoin networks, not just EIP-155 EVM IDs (§4.2, §8.1) — the old scheme couldn't distinguish Cosmos Hub from Osmosis, which the roadmap requires.
- Defined `AssetDescriptor` explicitly, including decimals and a cross-chain conversion rule (§4.2.1 — new).
- Completed the `.proto` definitions that were referenced but missing (`AcceptanceAck`, `ProofSubmission`, `IntentIdQuery`, `HealthRequest/Response`) (§8.1).
- Added a stake-to-claimable-amount cap for relayers so bonding scales with transfer size, not just a flat slash percentage (§5.2.1).
- Closed the Statera/Portus config-drift gap on `dispute_window_secs` by making Portus the source of truth, queried per-plan rather than hard-coded by Statera (§4.2, new §4.5).
- Added an explicit source-chain finality requirement and attestation field, since a plan issued against an unconfirmed source-chain lock defeats the whole rollback design (§4.2, new §4.6).
- Reclassified the "<500ms verification" line from a formal `ensures` contract to an SLO, since wall-clock behavior under real network I/O isn't the kind of thing an SMT/FM-based verifier discharges (§5.3).
- Added the actual Rust state machine for the dispute-window flow (new Appendix D).

---

## 1. Executive Summary

`tpt-portus` is the interoperability, routing, and cross-chain execution layer of the TPT ecosystem. Named after the Latin word for "port" or "harbor," it serves as the secure gateway where assets, intents, and state transitions are safely transferred between isolated blockchain networks.

While `tpt-statera` acts as the "brain" (state reconciliation, intent resolution, and global ledger), `tpt-portus` acts as the "limbs and nerves" — physically moving messages, verifying cryptographic proofs, coordinating relayer agents, and executing on-chain transactions across external blockchains.

### The Primary Problem Solved

In the current crypto ecosystem, users routinely lose funds by sending assets to the wrong network, and bridges routinely lose funds to double-spends between source-chain unlocks and destination-chain executions. `tpt-portus` addresses both failure modes through:

- Formal verification of address formats before any transaction is dispatched.
- Timeout-enforced atomic rollbacks with an explicit **dispute window** that closes the classic "rollback fires, then a late execution also succeeds" race (see §5.5.1) — this is the mechanism, not just a slogan.
- Cryptographically verified execution receipts preventing fraudulent relayers.
- Chain-abstracted routing so the user never manually selects a network.

We deliberately avoid over-claiming: rollback safety is a function of the dispute-window design, not an inherent property of "atomicity." Section 5.5.1 states the assumption this rests on explicitly.

---

## 2. Goals & Non-Goals

### 2.1 Goals

| # | Goal | Rationale |
|---|------|-----------|
| G1 | Provably safe cross-chain state transitions | Eliminate bridge hacks via `tpt-telos` formal verification |
| G2 | Full chain abstraction for end users | Remove "wrong network" UX failure entirely |
| G3 | Atomic execution with guaranteed rollback, closed against late-execution races | No stuck funds, no double payouts |
| G4 | Agentic, decentralized relaying with defined Sybil resistance | No single point of failure, and no single *operator*, in message delivery |
| G5 | Polyglot endpoint generation | Support EVM, Cosmos, Solana, and future chains |
| G6 | Best-effort low latency, tiered by destination chain's real finality | Competitive UX without misrepresenting settlement guarantees |
| G7 | Auditable, minimal attack surface | Clean separation from `tpt-statera` logic |

### 2.2 Non-Goals

- **Not a liquidity provider, and never a custodian of pooled reserves.** `tpt-portus` routes through third-party liquidity (external AMMs / LP pools it does not own or control). Endpoint contracts hold funds only transiently, per-intent, under escrow the user's own source-chain lock funded — Portus's code never has standing custody of a shared reserve. (This clarifies a point that was ambiguous in v0.1: §5.4's "available_here" tracking is an *accounting* invariant enforced on third-party pools via the endpoint contract's view function, not a claim that Portus owns those funds.)
- **Not a token issuer.** It does not mint wrapped/synthetic assets.
- **Not a user-facing wallet or UI.** It is a backend library consumed by `tpt-statera`, `tpt-payments`, and third-party dApps.
- **Not a consensus mechanism.** It does not validate blocks on external chains; it verifies proofs of finalized (or, for the optimistic tier, provisionally finalized) state.

---

## 3. Architecture Overview

### 3.1 System Context

```
┌─────────────────────────────────────────────────────────────────────┐
│                        USER / dAPP / WALLET                         │
└─────────────────────────────┬───────────────────────────────────────┘
                              │ "Send 100 USDC to Bob"
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                       tpt-statera (Brain)                            │
│    • Intent Resolution                                               │
│    • State Reconciliation                                            │
│    • Formal Verification of Constraints                              │
│    • Generates: IntentExecutionPlan                                  │
└─────────────────────────────┬───────────────────────────────────────┘
                              │ IntentExecutionPlan (signed)
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│                    ★ tpt-portus (This Library) ★                     │
│  ┌───────────┐ ┌───────────────┐ ┌────────────┐ ┌────────────┐      │
│  │  Router   │ │  Dispatcher   │ │   Proof    │ │  Endpoint  │      │
│  │  Module   │ │ (Relayer      │ │  Verifier  │ │  Generator │      │
│  │           │ │  Coord.)      │ │            │ │            │      │
│  └───────────┘ └───────────────┘ └────────────┘ └────────────┘      │
└───────┬──────────────┬──────────────────┬───────────────┬───────────┘
        ▼               ▼                  ▼               ▼
  ┌───────────┐   ┌───────────┐      ┌───────────┐   ┌───────────┐
  │ Ethereum  │   │  Solana   │      │  Cosmos   │   │  Bitcoin  │
  │  (EVM)    │   │  (SVM)    │      │  (IBC)    │   │  (UTXO)   │
  │ Endpoint  │   │ Endpoint  │      │ Endpoint  │   │ Endpoint  │
  │ Contract  │   │ Program   │      │ Module    │   │ Script    │
  └───────────┘   └───────────┘      └───────────┘   └───────────┘
        ▲               ▲                  ▲               ▲
        └───────────────┴──────────────────┴───────────────┘
                tpt-eve / tpt-anima (Agentic Relayer Network)
```

### 3.2 Internal Module Breakdown

`tpt-portus` is composed of five core modules:

1. **Router** — Determines the optimal path (chain, liquidity source, relayer pool).
2. **Dispatcher** — Coordinates the `tpt-eve`/`tpt-anima` relayer agents to deliver messages.
3. **Proof Verifier** — Validates Merkle proofs, light client headers, and ZK proofs from destination chains.
4. **Endpoint Generator** — Uses `tpt-telos` to produce formally verified smart contracts for each target chain.
5. **Rollback Engine** — Monitors timeouts and triggers reversals if execution fails, subject to the dispute-window rule in §5.5.1.

### 3.3 Finality & Latency Tiers (new)

G6 in v0.1 promised a single "sub-30-second" number across all chains, which is not achievable while also honoring each chain's real finality. Instead, we define latency tiers per destination chain and make the trade-off explicit:

| Tier | Chains (initial) | Finality basis | Typical settlement latency |
|---|---|---|---|
| Fast-finality | Solana, Cosmos/IBC (Tendermint) | Single-slot / single-block BFT finality | ~2–8 seconds |
| Optimistic | Ethereum, other EVM L1s | Soft confirmation + fraud-proof / dispute window, *not* full 12-block finality | ~15–30 seconds to provisional execution; dispute window (default 10 min, configurable) before the receipt is considered irreversible |
| Slow-finality | Bitcoin | SPV + N confirmations | Minutes (~1 hour for high-value transfers, per configurable confirmation depth) |

The v0.1 walkthrough's "12 seconds Ethereum finality" was incorrect (12 *blocks* at ~12s/block ≈ 144s minimum for the basic finality checkpoint, and full economic finality is on the order of minutes). This is corrected in §9.

---

## 4. The Handshake: `tpt-statera` ↔ `tpt-portus`

### 4.1 Design Principle

The boundary between `tpt-statera` and `tpt-portus` is a **strict, formally verified API contract**. `tpt-portus` is a pure executor: it may only act on a cryptographically signed `IntentExecutionPlan` and may only return a cryptographically signed `ExecutionReceipt`. It holds no independent decision-making authority over state.

### 4.2 Outbound: `IntentExecutionPlan`

Generated by `tpt-statera`. Consumed by `tpt-portus`. Immutable once signed.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[telos::verify(
    requires = "amount > 0 && timeout_block_height > current_block_height",
    requires = "source_finality_proof.confirms(source_chain, at_least = required_confirmations(source_chain))",
    ensures  = "cryptographic_proof.is_valid_for(hash(self))",
    ensures  = "recipient_address.matches_format(destination_chain)"
)]
pub struct IntentExecutionPlan {
    pub intent_id: Uuid,
    pub source_chain: ChainId,
    pub destination_chain: ChainId,
    pub asset: AssetDescriptor,
    pub amount: u64,                        // denominated in `asset.decimals` units on source_chain
    pub recipient_address: Vec<u8>,
    pub timeout_block_height: u64,
    pub dispute_window_secs: u64,           // fetched from Portus, not hard-coded by Statera — see §4.5
    pub fee_budget: u64,
    pub cryptographic_proof: Vec<u8>,
    pub created_at: u64,
    pub protocol_version: u16,
    pub source_finality_proof: Vec<u8>,     // NEW — see §4.6; proves the source-chain lock is past reorg risk
}
```

**`ChainId` — redesigned (was a flat `uint32`, which cannot distinguish Cosmos zones or represent Bitcoin):**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChainId {
    Evm { eip155_id: u64 },              // e.g. 1 = Ethereum mainnet, 42161 = Arbitrum
    Cosmos { chain_id: String },         // e.g. "cosmoshub-4", "osmosis-1" — IBC chain-ids are strings, not numbers
    Solana { cluster: SolanaCluster },   // Mainnet / Testnet / Devnet
    Bitcoin { network: BitcoinNetwork }, // Mainnet / Testnet / Signet
}
```

The v0.2 config (`ethereum chain_id = 1`, `cosmos chain_id = 118`, `bitcoin chain_id = 3`) mixed a real standard (EIP-155) with a borrowed-but-wrong one (118 is ATOM's *SLIP-44 coin type*, not a chain identifier) and an arbitrary number for Bitcoin. Since the roadmap targets both Cosmos Hub and Osmosis as separate mainnet deployments, a single numeric Cosmos chain ID is a genuine bug, not just an inconsistency — fixed by the tagged enum above.

### 4.2.1 `AssetDescriptor` (new — was referenced throughout but never defined)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetDescriptor {
    pub symbol: String,               // e.g. "USDC" — display only, not used for equality checks
    pub source_contract: Vec<u8>,     // token contract/mint address on source_chain
    pub destination_contract: Vec<u8>,// token contract/mint address on destination_chain
    pub source_decimals: u8,
    pub destination_decimals: u8,
}
```

**Conversion rule:** `amount` in `IntentExecutionPlan` is always denominated in `source_decimals` units. The Endpoint Contract on the destination chain converts to `destination_decimals` at execution time:

```
destination_amount = amount * 10^(destination_decimals - source_decimals)   // may be negative exponent
```

Any conversion that would lose more than `dust_threshold` (config, default 1 unit at `destination_decimals`) due to rounding is rejected rather than silently truncated — this needs its own formal contract (`ensures: abs(value(destination_amount) - value(amount)) <= dust_threshold`) once `tpt-telos` supports fixed-point reasoning; tracked as an open item in §14.

### 4.3 Inbound: `ExecutionReceipt`

Generated by `tpt-portus`. Consumed by `tpt-statera` to finalize global state.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[telos::verify(
    requires = "intent_id exists in statera.pending_ledger",
    ensures  = "if status == Success => chain_proof.verifies_against(destination_chain)",
    ensures  = "if status == Timeout => statera MUST NOT rollback until dispute_window_secs has elapsed with no late Success proof"
)]
pub struct ExecutionReceipt {
    pub intent_id: Uuid,
    pub destination_chain: ChainId,
    pub status: ExecutionStatus,
    pub chain_proof: Vec<u8>,
    pub executed_at_block: u64,
    pub portus_signature: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutionStatus {
    Success { tx_hash: String, block_number: u64 },
    Failed { error_code: u32, revert_reason: String },
    Timeout { current_block: u64, timeout_block: u64 },
    DisputeWindowOpen { closes_at_secs: u64 },   // NEW
}
```

### 4.4 Communication Protocol

- **Transport:** gRPC with mTLS between `tpt-statera` and `tpt-portus` daemons.
- **Serialization:** Protocol Buffers (`.proto`) for wire format; Rust structs for internal logic.
- **Idempotency:** Every `IntentExecutionPlan` carries a unique `intent_id`. Duplicate submissions are rejected and the existing receipt is returned instead of re-executing.
- **Ordering:** Plans are processed in FIFO order per `(source_chain, destination_chain)` pair.
- **Versioning:** `protocol_version` is checked on every plan; a version mismatch is rejected outright rather than best-effort-interpreted.

### 4.5 Dispute-Window Config Sync (new — closes a Statera/Portus drift risk)

v0.2 had Statera embed `dispute_window_secs` into the signed plan while the *authoritative* per-chain default lived only in `portus.toml`, read by Portus and never seen by Statera. If an operator changed Portus's config (e.g., tightening Ethereum's window after an incident) without Statera's copy changing in lockstep, the two systems would disagree about when it's safe to roll back — silently reopening the exact double-payout race §5.5.1 exists to close.

Fix: Portus is the sole source of truth for `dispute_window_secs`. Statera does not hard-code a value; it calls a new read-only RPC before generating each plan:

```protobuf
rpc GetDisputeWindow(ChainQuery) returns (DisputeWindowConfig);

message DisputeWindowConfig {
  ChainId chain_id = 1;
  uint64 dispute_window_secs = 2;
  uint64 config_version = 3;   // bumped on every operator change, so Statera can detect and log config changes
}
```

`IntentExecutionPlan.dispute_window_secs` is still embedded in the signed plan (so Portus can verify at execution time that it's enforcing the same window Statera assumed), but it's now always sourced from Portus, not chosen independently by Statera. If Portus ever receives a plan whose embedded value doesn't match its current config, it rejects the plan rather than silently using its own value — surfacing the drift instead of masking it.

### 4.6 Source-Chain Finality Requirement (new)

Nothing in v0.1 or v0.2 specified how deep a confirmation Statera must wait for on the *source* chain before locking funds and issuing a signed plan. If Statera treats an unconfirmed source-chain lock as final and a reorg later removes it, the destination chain can pay out against source funds that never actually existed — a failure mode at least as serious as the rollback race, just on the other side of the flow.

This is formally Statera's responsibility (per the non-goals in §2.2, Portus doesn't validate source-chain state), but Portus should not be purely trusting here — it's the component left holding the bag if this goes wrong. Accordingly:

- `IntentExecutionPlan.source_finality_proof` (added in §4.2) carries a light-client-verifiable proof that the source-chain lock has reached `required_confirmations(source_chain)` (same per-chain confirmation depths already defined for destination-side verification in §5.3).
- Portus's `ExecuteIntent` handler independently checks this proof before dispatching to relayers, rather than trusting Statera's say-so. This is a small amount of duplicated verification work in exchange for not being fully dependent on a system outside Portus's control for a safety-critical check.
- Residual risk (a source-chain reorg deeper than `required_confirmations`, e.g., a >50%-style attack) is out of scope for both systems and is called out as a residual risk in §14, not silently assumed away.

---

## 5. Core Module Specifications

### 5.1 Router Module

**Responsibility:** Given an `IntentExecutionPlan`, determine the optimal execution path across third-party liquidity — Portus never becomes the liquidity provider of record (see §2.2).

**Inputs:**
- Source and destination chain IDs
- Asset type and amount
- Current liquidity pool depths (fetched from on-chain oracles, read-only)
- Relayer network health metrics (from `tpt-eve`)

**Outputs:**
- `RouteDescriptor` containing the chosen relayer pool, liquidity provider, estimated gas, and expected latency (per the tier from §3.3).

**Formal Contract:**
```
requires: liquidity_available(route) >= plan.amount
ensures:  estimated_gas_fee <= plan.fee_budget
ensures:  route does not traverse more than 2 intermediate hops
```

**Algorithm:**
1. Query all registered liquidity pools for the `(asset, source_chain, destination_chain)` tuple.
2. Filter pools where available liquidity ≥ `plan.amount`.
3. Score remaining pools by: `(gas_cost * 0.4) + (latency * 0.4) + (historical_success_rate * 0.2)`.
4. Select the highest-scoring route.
5. If no direct route exists, compute a 2-hop path (e.g., Ethereum → Cosmos → Solana).

### 5.2 Dispatcher Module (Relayer Coordination)

**Responsibility:** Deliver the `IntentExecutionPlan` to the appropriate `tpt-eve`/`tpt-anima` relayer agents.

**Design:**
- Relayers are permissionless agents that monitor `tpt-portus` for pending plans.
- The Dispatcher publishes plans to a topic-based message queue (e.g., NATS or libp2p pub/sub).
- Relayers compete to claim and execute the plan. The first to submit a valid `chain_proof` wins the fee.
- A minimum of 3 **independent** relayers must acknowledge receipt before the plan is considered "in-flight."

**Independence criteria (new — closes the Sybil gap from v0.1):** an acknowledgment only counts toward the 3-of-3 threshold if it comes from a relayer that is registered under a distinct operator identity (staked address with a distinct owner key) **and** whose network egress falls under a distinct ASN, as reported by the `tpt-eve` reputation registry. Acknowledgments from operators sharing a registered identity or ASN are deduplicated to one.

**Fault Tolerance:**
- If no relayer claims the plan within `timeout_block_height / 2`, the Dispatcher escalates to a backup relayer pool.
- If the timeout is fully breached, the Rollback Engine is triggered — subject to the dispute window (§5.5.1).

#### 5.2.1 Relayer Fee Economics (new)

`fee_budget` on the `IntentExecutionPlan` is split as follows, published in `portus.toml` and adjustable per deployment:

- 70% to the executing relayer (paid on valid `SubmitProof`).
- 20% to the underlying liquidity pool as a routing fee.
- 10% reserved as a slashing bond top-up pool, used to cover cases where a slashed relayer's bond is insufficient to make the protocol whole.

A relayer may under-bid by claiming a plan and executing at lower actual gas cost than budgeted; the difference is refunded pro-rata to the fee-payer, not pocketed by the relayer, to avoid incentivizing gas-price manipulation. (The fee is charged against `fee_budget`, which is separate from the transferred `amount` — the recipient always receives the full converted amount per §4.2.1; the fee does not come out of the transfer itself.)

**Stake-to-claimable-amount cap (new — closes a bonding gap from v0.2):** a flat slash percentage alone doesn't deter fraud on large transfers if the fraudulent upside exceeds the relayer's bond. A relayer may only claim a plan if:

```
plan.amount (in a common USD-equivalent reference) <= relayer.staked_bond * collateralization_ratio
```

`collateralization_ratio` defaults to 3x and is configurable per asset risk tier. Claims exceeding a relayer's capacity fall through to the backup relayer pool (§5.2 Fault Tolerance) rather than being accepted anyway. This means large transfers may need higher-bonded relayers or splitting across multiple smaller intents — a UX trade-off worth surfacing to `tpt-payments`, not hiding.

### 5.3 Proof Verifier Module

**Responsibility:** Cryptographically verify that a transaction actually occurred on the destination chain, at the finality level appropriate to its tier (§3.3).

**Supported Proof Types:**

| Chain Family | Proof Mechanism | Verification Method |
|---|---|---|
| EVM (Ethereum, BSC, Arbitrum) | Merkle Patricia Trie + Block Header | Light client header verification, optimistic tier |
| Cosmos (ATOM, Osmosis) | IBC Tendermint Light Client | Consensus state + Merkle proof, fast-finality tier |
| Solana | Account state proof + Slot hash | Ed25519 validator signature set, fast-finality tier |
| Bitcoin | SPV Merkle Block Proof | Header-chain PoW verification, slow-finality tier |
| ZK Chains (zkSync, StarkNet) | Zero-Knowledge Proof | On-chain verifier contract |

**Formal Contract:**
```
requires: proof is well-formed for the given chain_id
ensures:  if verify(proof) == true => transaction is finalized-or-provisional
          (per tier) on destination_chain
```

**Operational SLO (not a formal contract):** target p99 verification latency < 500ms for all supported chains. This is deliberately kept out of the `#[telos::verify]`-style contracts above — `tpt-telos`'s SMT/FM-based verification reasons about logical invariants over program state, not wall-clock behavior under real network I/O, so "completes in < 500ms" isn't the kind of property it can actually discharge. Track it as a Prometheus SLO (`portus_proof_verification_duration_seconds`) with alerting instead of a proof obligation.

### 5.4 Endpoint Generator Module

**Responsibility:** Use `tpt-telos` to automatically generate, verify, and deploy the lightweight smart contracts that sit on each external chain. These contracts hold funds only transiently, per-intent — never as a standing Portus-owned reserve (§2.2).

**Generated Artifacts:**

| Target Chain | Language | Framework |
|---|---|---|
| Ethereum / EVM L2s | Solidity | Foundry |
| Cosmos / IBC chains | Rust | CosmWasm |
| Solana | Rust | Anchor |
| Bitcoin (limited) | Script / Miniscript | Custom |
| Go-based chains | Go | Cosmos SDK module |

**Endpoint Contract Responsibilities (on-chain):**
1. Listen for incoming cross-chain messages (submitted by relayers).
2. Verify the message proof using the embedded light client.
3. **Check the intent has not been marked `ROLLED_BACK` in the local replay-protection store** (this is the check that closes the race in §5.5.1).
4. Execute the token release/mint to the recipient.
5. Emit a confirmation event for the relayer to capture.

**Formal Contract for Account-Model Chains (EVM, Solana, Cosmos):**
```
requires: msg.proof.verifies_against(stored_consensus_state)
requires: msg.intent_id has not been previously executed (replay protection)
requires: msg.intent_id is not marked ROLLED_BACK
ensures:  recipient.balance_after == recipient.balance_before + msg.amount
ensures:  total_locked_on_source + total_available_here == invariant_supply
```

**Formal Contract for UTXO Chains (Bitcoin) — corrected from v0.1:**
Bitcoin has no account balances, so the invariant is expressed over UTXO set membership rather than balances:
```
requires: msg.proof.verifies_against(stored_header_chain)
requires: msg.intent_id has not been previously spent (tracked via OP_RETURN nonce)
requires: msg.intent_id is not marked ROLLED_BACK
ensures:  a new UTXO of value msg.amount, spendable only by recipient's script, exists post-execution
ensures:  the UTXO consumed from the escrow output is fully spent (no double-spend of the escrow input)
```

### 5.5 Rollback Engine

**Responsibility:** Guarantee that no funds are ever permanently stuck, **without** creating a double-payout race against late destination-chain execution.

**Trigger Conditions:**
- `current_block >= plan.timeout_block_height` **and** the dispute window (§5.5.1) has elapsed with no valid `ExecutionReceipt::Success`.
- Relayer submits a `Failed` status with an unrecoverable error.
- Proof Verifier rejects the `chain_proof` (potential fraud attempt).

#### 5.5.1 Closing the Rollback/Late-Execution Race (new)

This is the mechanism the v0.1 draft was missing. The problem: if rollback unlocks funds on the source chain purely on a block-height timeout, a relayer can still get a legitimate execution finalized on the destination chain moments later, and the recipient gets paid twice from the protocol's perspective.

The fix is a two-stage timeout:

1. **Timeout fires** at `plan.timeout_block_height` → status moves to `DisputeWindowOpen`, *not* directly to `Timeout`. No funds move yet.
2. **Dispute window** (`dispute_window_secs`, default derived from the destination chain's tier — near-zero for fast-finality chains, several minutes for optimistic/EVM, up to an hour for Bitcoin) elapses. During this window:
   - The endpoint contract on the destination chain is queried (or, for chains where this isn't cheap, a cached light-client head is checked) for whether the intent already executed.
   - If a valid `Success` proof surfaces during the window, the plan resolves as `Success`, not `Timeout` — no rollback occurs, and the endpoint contract's own `ROLLED_BACK` check (added in §5.4) is never triggered because Portus never sent that marker.
   - If nothing surfaces by the window's end, Portus writes an on-chain (or light-client-attested) `ROLLED_BACK` marker for `intent_id`, canonically closing off any future execution attempt for that intent, *then* emits `RollbackRequest` to Statera.
3. Only after that marker exists does Statera unlock the source-chain funds.

This adds latency to the unhappy path (bounded by `dispute_window_secs`) in exchange for eliminating the double-payout race. This trade-off should be called out explicitly to stakeholders, not hidden behind "guaranteed rollback" language.

**Rollback Procedure (revised):**
1. Dispute window elapses with no success proof.
2. `tpt-portus` writes the `ROLLED_BACK` marker (see above) and emits `RollbackRequest` to `tpt-statera`.
3. `tpt-statera` verifies the request against its pending ledger.
4. `tpt-statera` unlocks the user's funds on the source chain.
5. `tpt-portus` marks the `intent_id` as `ROLLED_BACK` in its own internal ledger (already done in step 1 on-chain; this is the internal mirror).
6. The relayer that failed to deliver is slashed (if staked).

**Formal Contract (revised):**
```
requires: current_block >= timeout_block_height AND dispute_window has elapsed
          with no ExecutionStatus::Success
       OR status == Failed
ensures:  ROLLED_BACK marker for intent_id is written before source funds unlock
ensures:  user.source_balance_after_rollback == user.source_balance_before_transfer
ensures:  intent_id is permanently marked and cannot be re-executed on any chain
```

---

## 6. Security Model

### 6.1 Threat Model

| Threat | Mitigation |
|---|---|
| Malicious relayer submits fake proof | Proof Verifier rejects; relayer is slashed |
| Replay attack (re-submitting old plan) | `intent_id` uniqueness + on-chain nonce tracking |
| Address format confusion (wrong network) | `tpt-statera` validates format BEFORE generating plan; `tpt-portus` re-validates as defense-in-depth |
| Timeout manipulation | Timeout is set by `tpt-statera` based on destination chain's finality; cannot be altered by relayer |
| **Rollback / late-execution double payout** | **Dispute window + on-chain `ROLLED_BACK` marker (§5.5.1) — new** |
| **Sybil'd relayer acknowledgment threshold** | **Operator-identity + ASN diversity check on the 3-of-3 threshold (§5.2) — new** |
| Endpoint contract exploit | Generated by `tpt-telos` with formal verification; no manual code |
| Denial of service on relayer network | Minimum 3-acknowledgment threshold; backup relayer pools |
| Denial of service on public `SubmitProof` gRPC | Rate limiting + proof-of-stake bond required to submit (§6.4 — new) |
| Statera key compromise | Threshold signature scheme (t-of-n) for `cryptographic_proof`, not a single Ed25519/ECDSA key; HSM recommended (clarified — see §6.3) |

### 6.2 Trust Assumptions

- `tpt-statera` is the sole authority for generating `IntentExecutionPlan`. `tpt-portus` trusts the signature.
- External chain consensus mechanisms (Ethereum PoS, Tendermint BFT, Solana PoH) are assumed to be secure at the confirmation depth configured per chain.
- The `tpt-telos` formal verifier's compiler backend is assumed sound; this assumption is out of scope for `tpt-portus`'s own security review and should be tracked as a dependency risk, not treated as a given.
- The dispute-window mechanism (§5.5.1) assumes destination-chain state (or its light client) remains queryable during the window; an extended destination-chain outage that also outlasts the window is a residual risk, called out explicitly in §10.2.

### 6.3 Cryptographic Primitives

- **Signatures:** Ed25519 for Cosmos/Solana chains; Secp256k1 (ECDSA) for EVM chains.
- **Statera plan signing:** t-of-n threshold Ed25519 (or BLS) signature scheme across Statera's signing nodes — not a single key — so that no single compromised Statera node can forge a valid `IntentExecutionPlan`. (This replaces the vaguer "multi-sig requirement" language from v0.1 with a concrete scheme choice; final selection between threshold-Ed25519 and BLS is an open item for Phase 1.)
- **Hashing:** SHA-256 (Cosmos/Bitcoin), Keccak-256 (EVM), BLAKE3 (internal TPT hashing).
- **Transport Security:** mTLS 1.3 between all TPT daemons.
- **Key Management:** Hierarchical Deterministic (HD) key derivation; private keys never leave HSM.

### 6.4 Rate Limiting & DoS Protection on Public Surfaces (new)

`SubmitProof` is the one gRPC method reachable by permissionless relayers rather than only by `tpt-statera`, and is therefore the primary external attack surface:

- Each submission must include a valid relayer stake reference; unstaked submitters are rejected before proof verification runs (cheap check first).
- Per-relayer submission rate is capped (default 10 req/sec, configurable via `portus.relayer.rate_limit_rps`).
- Proof verification work is queued and load-shed under sustained overload rather than allowed to block the FIFO plan-processing path for legitimate intents.

---

## 7. Configuration

### 7.1 Configuration File (`portus.toml`)

```toml
[portus]
node_id = "portus-node-01"
listen_addr = "0.0.0.0:9400"
log_level = "info"

[portus.statera]
endpoint = "grpc://statera.internal:9300"
tls_cert = "/etc/tpt/certs/statera-ca.pem"
timeout_ms = 5000

[portus.relayer]
message_bus = "nats://nats.internal:4222"
min_acknowledgments = 3
claim_timeout_secs = 30
backup_pool_id = "relayer-backup-01"
rate_limit_rps = 10
fee_split_relayer_pct = 70
fee_split_liquidity_pct = 20
fee_split_slash_reserve_pct = 10

[portus.chains.ethereum]
chain_id = 1
rpc_url = "https://eth-mainnet.internal:8545"
finality_blocks = 12
endpoint_contract = "0x..."
light_client_sync_interval_secs = 12
dispute_window_secs = 600

[portus.chains.solana]
chain_id = 2
rpc_url = "https://solana-mainnet.internal:8899"
finality_slots = 32
endpoint_program = "PortusEndpoint1111111111111111111111"
dispute_window_secs = 5

[portus.chains.cosmos]
chain_id = 118
rpc_url = "https://cosmos-mainnet.internal:26657"
finality_blocks = 1
ibc_client_id = "07-tendermint-42"
dispute_window_secs = 5

[portus.chains.bitcoin]
chain_id = 3
rpc_url = "https://btc-mainnet.internal:8332"
confirmation_depth = 6
dispute_window_secs = 3600

[portus.rollback]
check_interval_secs = 5
max_retry_attempts = 3
slash_percentage = 10

[portus.security]
hsm_endpoint = "pkcs11:///dev/hsm0"
key_id = "portus-signing-key-01"
statera_signature_scheme = "threshold-ed25519"
```

### 7.2 Environment Overrides

All configuration values can be overridden via environment variables prefixed with `PORTUS_`:
- `PORTUS_NODE_ID`
- `PORTUS_STATERA_ENDPOINT`
- `PORTUS_RELAYER_MESSAGE_BUS`

---

## 8. API Surface

### 8.1 gRPC Service Definition

```protobuf
syntax = "proto3";
package tpt.portus.v1;

service PortusService {
  rpc ExecuteIntent(IntentExecutionPlan) returns (AcceptanceAck);
  rpc GetIntentStatus(IntentIdQuery) returns (ExecutionReceipt);
  // Server-streaming variant recommended over polling for long-lived intents
  // (an intent can sit in DISPUTE_WINDOW_OPEN for up to an hour on Bitcoin).
  rpc WatchIntentStatus(IntentIdQuery) returns (stream ExecutionReceipt);
  rpc SubmitProof(ProofSubmission) returns (ProofAck);
  rpc GetDisputeWindow(ChainQuery) returns (DisputeWindowConfig);   // see §4.5
  rpc HealthCheck(HealthRequest) returns (HealthResponse);
}

// --- Chain identity: tagged, not a flat uint32 (see §4.2) ---
message ChainId {
  oneof chain {
    EvmChain evm = 1;
    CosmosChain cosmos = 2;
    SolanaChain solana = 3;
    BitcoinChain bitcoin = 4;
  }
}
message EvmChain     { uint64 eip155_id = 1; }
message CosmosChain  { string chain_id = 1; }               // e.g. "osmosis-1"
message SolanaChain  { SolanaCluster cluster = 1; }
message BitcoinChain { BitcoinNetwork network = 1; }
enum SolanaCluster   { SOLANA_MAINNET = 0; SOLANA_TESTNET = 1; SOLANA_DEVNET = 2; }
enum BitcoinNetwork  { BITCOIN_MAINNET = 0; BITCOIN_TESTNET = 1; BITCOIN_SIGNET = 2; }

message AssetDescriptor {
  string symbol = 1;
  bytes source_contract = 2;
  bytes destination_contract = 3;
  uint32 source_decimals = 4;
  uint32 destination_decimals = 5;
}

message IntentExecutionPlan {
  string intent_id = 1;
  ChainId source_chain = 2;
  ChainId destination_chain = 3;
  AssetDescriptor asset = 4;
  uint64 amount = 5;
  bytes recipient_address = 6;
  uint64 timeout_block_height = 7;
  uint64 dispute_window_secs = 8;
  uint64 fee_budget = 9;
  bytes cryptographic_proof = 10;
  uint64 created_at = 11;
  uint32 protocol_version = 12;
  bytes source_finality_proof = 13;
}

message ExecutionReceipt {
  string intent_id = 1;
  ChainId destination_chain = 2;
  ExecutionStatus status = 3;
  bytes chain_proof = 4;
  uint64 executed_at_block = 5;
  bytes portus_signature = 6;
}

enum ExecutionStatus {
  PENDING = 0;
  IN_FLIGHT = 1;
  SUCCESS = 2;
  FAILED = 3;
  DISPUTE_WINDOW_OPEN = 4;
  TIMEOUT = 5;
  ROLLED_BACK = 6;
}

// --- Previously referenced but undefined in v0.1/v0.2 — filled in ---
message AcceptanceAck {
  string intent_id = 1;
  bool accepted = 2;
  string rejection_reason = 3;   // set only if accepted == false
}

message IntentIdQuery {
  string intent_id = 1;
}

message ChainQuery {
  ChainId chain_id = 1;
}

message DisputeWindowConfig {
  ChainId chain_id = 1;
  uint64 dispute_window_secs = 2;
  uint64 config_version = 3;
}

message ProofSubmission {
  string intent_id = 1;
  bytes relayer_stake_ref = 2;   // checked before proof verification runs (§6.4)
  bytes chain_proof = 3;
  ChainId destination_chain = 4;
}

message ProofAck {
  bool accepted = 1;
  string rejection_reason = 2;
}

message HealthRequest {}

message HealthResponse {
  bool healthy = 1;
  string version = 2;
  map<string, string> component_status = 3;   // e.g. "nats" -> "ok", "hsm" -> "ok"
}
```

Reserve field numbers 20–29 on `IntentExecutionPlan` and `ExecutionReceipt` for future additions (e.g., the fixed-point dust-threshold field flagged in §4.2.1) so schema evolution doesn't force a breaking renumber.

### 8.2 Internal Rust Trait API

```rust
#[async_trait]
pub trait PortusEngine: Send + Sync {
    async fn execute_intent(&self, plan: IntentExecutionPlan) -> Result<AcceptanceAck, PortusError>;
    async fn get_status(&self, intent_id: Uuid) -> Result<ExecutionReceipt, PortusError>;
    async fn submit_proof(&self, submission: ProofSubmission) -> Result<ProofAck, PortusError>;
    async fn shutdown(&self) -> Result<(), PortusError>;
}
```

---

## 9. Data Flow: End-to-End Walkthrough (timing corrected)

**Scenario:** Alice sends 100 USDC from Ethereum to Bob on Solana.

```
Step 1: Alice clicks "Send 100 USDC to Bob" in tpt-payments UI.
        (Alice never selects a network.)

Step 2: tpt-statera receives the intent.
        - Validates Alice's balance on Ethereum.
        - Validates Bob's Solana address format (32 bytes, base58).
        - Locks 100 USDC in the Ethereum escrow contract.
        - Generates IntentExecutionPlan with timeout = current_block + 100,
          dispute_window_secs = 600 (per §7.1 EVM default).
        - Signs the plan with Statera's threshold key.

Step 3: tpt-portus receives the IntentExecutionPlan via gRPC.
        - Re-validates Bob's address format (defense-in-depth).
        - Router selects a third-party Solana USDC pool (0.02% fee) — Portus
          does not own this pool (§2.2).
        - Dispatcher publishes plan to NATS topic "portus.plans.eth-sol".

Step 4: Three tpt-eve relayer agents, verified as independent operators
        under distinct stakes/ASNs (§5.2), acknowledge the plan.
        - Relayer Alpha claims execution.
        - Alpha calls the Solana endpoint program with the plan + Merkle proof.

Step 5: Solana endpoint program (generated by tpt-telos):
        - Verifies the Merkle proof against the stored Ethereum light client state.
        - Confirms intent_id has not been seen before and is not ROLLED_BACK.
        - Releases 100 USDC from the third-party Solana liquidity pool to Bob.
        - Emits ExecutionConfirmed event.

Step 6: Relayer Alpha captures the event, packages the chain_proof,
        and submits it to tpt-portus via SubmitProof gRPC call.
        Solana's fast-finality tier means this typically happens within
        ~2-8 seconds of Step 4 (§3.3) — well inside Ethereum's own
        dispute window, so no race condition arises here.

Step 7: tpt-portus Proof Verifier validates the Solana slot hash and
        validator signatures, confirms the proof matches the intent,
        and signs the ExecutionReceipt (status: Success).

Step 8: tpt-portus returns ExecutionReceipt (status: Success) to tpt-statera.

Step 9: tpt-statera marks the intent COMPLETED, finalizes global state
        (Alice -100 on ETH, Bob +100 on SOL), and releases the Ethereum
        escrow lock to the liquidity provider that fronted the Solana-side
        funds.

Total elapsed time: dominated by Solana's fast-finality tier (~2-8s) plus
gRPC/relayer overhead — typically under 15 seconds end-to-end for this
direction, since the Ethereum side only needed a source-chain lock
(near-instant) rather than waiting for destination-side EVM finality.
The reverse direction (Solana -> Ethereum) is bound by the optimistic-tier
dispute window on the Ethereum destination side if a timeout occurs, though
the happy path (proof arrives well before dispute_window_secs elapses) is
still fast.
```

---

## 10. Error Handling & Edge Cases

### 10.1 Error Taxonomy

```rust
#[derive(Error, Debug)]
pub enum PortusError {
    #[error("Invalid address format for chain {chain_id}")]
    InvalidAddressFormat { chain_id: ChainId },

    #[error("Amount must be > 0, got {0}")]
    InvalidAmount(u64),

    #[error("Timeout block {timeout} is not in the future (current: {current})")]
    InvalidTimeout { current: u64, timeout: u64 },

    #[error("No viable route found for {asset} from chain {src} to chain {dst}")]
    NoRouteAvailable { asset: String, src: ChainId, dst: ChainId },

    #[error("Insufficient liquidity: need {need}, available {available}")]
    InsufficientLiquidity { need: u64, available: u64 },

    #[error("Relayer dispatch failed after {attempts} attempts")]
    RelayerDispatchFailed { attempts: u32 },

    #[error("Timeout: block {current} exceeded deadline {timeout}")]
    ExecutionTimeout { current: u64, timeout: u64 },

    #[error("Dispute window still open, closes at {closes_at_secs}")]
    DisputeWindowOpen { closes_at_secs: u64 },

    #[error("Chain proof verification failed for chain {chain_id}: {reason}")]
    ProofVerificationFailed { chain_id: ChainId, reason: String },

    #[error("Replay detected: intent_id {0} already executed")]
    ReplayAttack(Uuid),

    #[error("Relayer acknowledgment rejected: insufficient operator diversity")]
    InsufficientRelayerDiversity,

    #[error("Statera connection lost: {0}")]
    StateraUnavailable(String),

    #[error("HSM signing failure: {0}")]
    HsmError(String),
}
```

### 10.2 Edge Case Handling

| Edge Case | Behavior |
|---|---|
| User sends to own address on another chain | Allowed (valid use case for self-custody migration) |
| Destination chain is halted/down for longer than the dispute window | **Residual risk, flagged explicitly:** rollback proceeds once the window elapses, but if the chain later un-halts and the transaction retroactively confirms, this is a genuine conflict that needs a manual reconciliation path — not fully automatable, and should be documented as such rather than assumed away |
| Relayer submits proof for wrong `intent_id` | Rejected; relayer slashed |
| Double-submission of same `intent_id` | Idempotent: returns existing receipt, no re-execution |
| Amount exceeds liquidity pool depth | Router rejects at Step 3; Statera unlocks funds immediately |
| Gas spike on destination chain mid-execution | Fee budget is pre-calculated with 2x buffer; if exceeded, falls to timeout → dispute window → rollback path |
| Relayer collusion attempt on the 3-ack threshold | Rejected at acknowledgment time per the independence check (§5.2), not just detected after the fact |

---

## 11. Testing Strategy

### 11.1 Unit Tests (per module)

- **Router:** Mock liquidity pools; verify optimal path selection under varying conditions.
- **Proof Verifier:** Test against known-good and known-bad proofs for each chain family.
- **Rollback Engine:** Simulate timeout conditions *and* late-arriving success proofs during the dispute window; verify the race is actually closed, not just the happy-path timeout.
- **Address Validator:** Fuzz test with malformed addresses for every supported chain.

### 11.2 Integration Tests

- Full `tpt-statera` → `tpt-portus` → mock destination chain → receipt loop.
- Concurrent intent execution (100 simultaneous cross-chain transfers).
- Relayer failure simulation (kill 2 of 3 relayers mid-execution).
- **New:** inject a proof that arrives 1 second before and 1 second after `dispute_window_secs` closes, and confirm the outcome differs correctly in each case.

### 11.3 Formal Verification (via `tpt-telos`)

- All `requires`/`ensures` contracts are compiled and checked by `tpt-telos`'s verification backend (which uses Fourier-Motzkin elimination for the linear-arithmetic fragment of its constraints, combined with an SMT solver for the rest — stated plainly here rather than conflating the two).
- Endpoint smart contracts are verified before deployment (no unverified code on-chain).
- State machine transitions (`Pending → InFlight → DisputeWindowOpen → Success/Failed/RolledBack`) are proven to be exhaustive and deadlock-free, including the added `DisputeWindowOpen` state.

### 11.4 Chaos / Adversarial Testing

- Simulate chain reorgs (Ethereum reorg of 3 blocks during in-flight transfer).
- Byzantine relayer submitting valid-looking but fraudulent proofs.
- Network partition between `tpt-statera` and `tpt-portus` during execution.
- **New:** Sybil relayer attack simulating multiple acknowledgments from a single operator behind different addresses, to confirm the diversity check (§5.2) rejects them.

---

## 12. Deployment & Operations

### 12.1 Deployment Topology

```
┌─────────────────────────────────────────────────────┐
│              Kubernetes / Bare Metal                 │
│                                                       │
│  ┌─────────────┐    ┌─────────────┐                  │
│  │ tpt-statera │◄──►│ tpt-portus  │ (gRPC + mTLS)     │
│  │  (3 nodes)  │    │  (3 nodes)  │                   │
│  └─────────────┘    └──────┬──────┘                  │
│                            │                          │
│                     ┌──────▼──────┐                   │
│                     │  NATS Bus   │                   │
│                     │  (Relayer   │                   │
│                     │   Coord.)   │                   │
│                     └──────┬──────┘                   │
│                            │                          │
│              ┌─────────────┼─────────────┐            │
│              ▼             ▼             ▼            │
│        ┌──────────┐ ┌──────────┐ ┌──────────┐        │
│        │ tpt-eve  │ │ tpt-eve  │ │ tpt-anima│        │
│        │ Relayer  │ │ Relayer  │ │ Relayer  │        │
│        └──────────┘ └──────────┘ └──────────┘        │
│                                                       │
│  ┌─────────────────────────────────────────────┐     │
│  │      Monitoring (Prometheus + Grafana)       │     │
│  └─────────────────────────────────────────────┘     │
└───────────────────────────────────────────────────────┘
```

### 12.2 Observability

**Metrics (Prometheus):**
- `portus_intents_total` (counter, labels: `source_chain`, `destination_chain`, `status`)
- `portus_execution_duration_seconds` (histogram)
- `portus_relayer_claim_duration_seconds` (histogram)
- `portus_proof_verification_failures_total` (counter)
- `portus_rollbacks_total` (counter)
- `portus_dispute_window_races_detected_total` (counter — **new**: counts cases where a late success proof arrived during a dispute window, i.e. the mechanism actually firing)
- `portus_liquidity_available` (gauge, per pool)

**Tracing:** OpenTelemetry spans for every intent from receipt to final receipt/rollback, including a span for the dispute-window wait.

**Alerting:** PagerDuty/Opsgenie alerts for:
- Rollback rate > 5% over 10 minutes.
- No relayer claims for > 60 seconds.
- Proof verification failure spike.
- Any `portus_dispute_window_races_detected_total` increment (should be rare; a spike indicates either an attack or a mis-set timeout, and either way merits investigation).

### 12.3 Upgrade Strategy

- Endpoint smart contracts on external chains are immutable once deployed. Upgrades require deploying a new contract version and migrating liquidity.
- `tpt-portus` daemon upgrades are rolling (Kubernetes Deployment with `maxSurge: 1`).
- Protocol version is embedded in every `IntentExecutionPlan` and checked explicitly (§4.4); mismatched versions are rejected, not best-effort-parsed.

---

## 13. Roadmap

**Phase 1: Foundation (Months 1–3)**
- Core Rust library scaffold (`Cargo.toml`, module structure).
- `IntentExecutionPlan` and `ExecutionReceipt` data structures, including the dispute-window fields.
- gRPC service definition and codegen.
- Address format validators for EVM, Solana, Cosmos.
- Router module with single-hop routing.
- Integration with `tpt-statera` handshake (mock Statera for testing).
- **Decide the concrete threshold-signature scheme (threshold-Ed25519 vs. BLS) for Statera plan signing** — flagged as an open item in §6.3.

**Phase 2: Execution & Relaying (Months 4–6)**
- NATS-based Dispatcher module, including relayer independence checks (§5.2).
- `tpt-eve` relayer agent integration (claim, execute, submit proof).
- Proof Verifier for EVM (Merkle Patricia Trie) and Cosmos (IBC light client).
- Rollback Engine with dispute-window logic (§5.5.1) — this should be built and tested before any other module ships to production, given its centrality to the "no stuck funds" guarantee.
- First endpoint contract generated by `tpt-telos` (CosmWasm for Cosmos).

**Phase 3: Multi-Chain & Hardening (Months 7–9)**
- Solana endpoint program (Anchor/Rust).
- EVM endpoint contract (Solidity via `tpt-telos` FFI).
- Bitcoin endpoint using the UTXO-native invariant (§5.4), not the account-model one.
- 2-hop routing (e.g., ETH → Cosmos → Solana).
- Formal verification of all endpoint contracts via `tpt-telos`.
- Chaos testing and adversarial relayer simulation, including the Sybil scenario in §11.4.
- HSM integration for key management.

**Phase 4: Production & Ecosystem (Months 10–12)**
- Mainnet deployment (Ethereum, Cosmos Hub, Solana, Osmosis, Arbitrum).
- `tpt-payments` integration (chain-abstracted merchant/user flows).
- Monitoring dashboards and alerting rules, including the dispute-window-race counter.
- Public SDK for third-party dApps to consume `tpt-portus`.
- Security audit (external firm) — **the dispute-window/rollback mechanism (§5.5.1) and the relayer independence check (§5.2) should be called out explicitly as the highest-priority review items**, since they're the two places v0.1 had the weakest guarantees.
- Documentation site and developer onboarding guides.

---

## 14. Open Risks & Future Work

*(This section was missing from v0.1's numbering — added here rather than silently renumbering §15 down.)*

- **Extended destination-chain outages beyond the dispute window** (§10.2) are not fully automatable and need a documented manual-reconciliation runbook, not just a rollback rule.
- **Threshold signature scheme choice** for Statera plan signing (BLS vs. threshold-Ed25519) is unresolved — affects both performance and the specific compromise-tolerance guarantee.
- **`tpt-telos` soundness is a trust assumption, not a proof** — worth tracking as an explicit dependency risk with its own audit trail, separate from `tpt-portus`'s own audit.
- **Fee-split percentages (§5.2.1)** are a starting proposal, not economically modeled yet; should be stress-tested against relayer profitability under realistic gas-price volatility before mainnet.
- **Third-party liquidity pool risk:** since Portus explicitly does not own the pools it routes through (§2.2), pool-level exploits (e.g., an AMM being drained) are outside Portus's control but still affect user funds mid-transfer — worth a clear incident-response story for "the pool got hacked while our transfer was in flight."

---

## 15. Appendix

### A. Glossary

| Term | Definition |
|---|---|
| Chain Abstraction | Hiding the underlying blockchain from the user; they interact with a single unified interface. |
| Dispute Window | The period after a timeout during which a late execution proof can still be accepted before rollback is finalized (§5.5.1). |
| Endpoint Contract | The lightweight smart contract deployed on an external chain that listens for and executes cross-chain messages. |
| Finality | The point at which a transaction on a blockchain is irreversible. |
| IBC | Inter-Blockchain Communication; the native messaging protocol of the Cosmos ecosystem. |
| Light Client | A minimal on-chain verifier that can check block headers and Merkle proofs without downloading the full chain. |
| Relayer | An off-chain agent that reads events from one chain and submits them to another. |
| Rollback | The reversal of a partially completed cross-chain transaction, returning funds to the sender, gated by the dispute window. |

### B. References

- Cosmos IBC Specification: https://github.com/cosmos/ibc
- Chainlink CCIP Architecture: https://chain.link/cross-chain
- LayerZero Protocol: https://layerzero.network
- Ethereum Merkle Patricia Trie: https://ethereum.org/en/developers/docs/data-structures-and-encoding/patricia-merkle-trie/
- Solana Account Model: https://docs.solana.com/developing/programming-model/accounts

### C. File Structure (Planned)

```
tpt-portus/
├── Cargo.toml
├── README.md
├── DESIGN.md                  ← (this document)
├── proto/
│   └── portus/v1/
│       └── portus.proto
├── src/
│   ├── lib.rs
│   ├── types.rs               ← IntentExecutionPlan, ExecutionReceipt, ChainId, etc.
│   ├── error.rs               ← PortusError enum
│   ├── engine.rs              ← PortusEngine orchestrator
│   ├── router/
│   │   ├── mod.rs
│   │   ├── scoring.rs
│   │   └── liquidity.rs
│   ├── dispatcher/
│   │   ├── mod.rs
│   │   ├── nats_client.rs
│   │   ├── relayer_pool.rs
│   │   └── diversity_check.rs   ← NEW (§5.2)
│   ├── verifier/
│   │   ├── mod.rs
│   │   ├── evm.rs
│   │   ├── cosmos.rs
│   │   ├── solana.rs
│   │   └── bitcoin.rs
│   ├── endpoint_gen/
│   │   ├── mod.rs
│   │   ├── cosmwasm.rs
│   │   ├── solidity.rs
│   │   └── anchor.rs
│   ├── rollback/
│   │   ├── mod.rs
│   │   ├── monitor.rs
│   │   └── dispute_window.rs    ← NEW (§5.5.1)
│   ├── grpc/
│   │   ├── mod.rs
│   │   └── server.rs
│   └── config.rs
├── tests/
│   ├── integration/
│   │   ├── statera_handshake.rs
│   │   ├── full_transfer_evm_cosmos.rs
│   │   ├── timeout_rollback.rs
│   │   └── dispute_window_race.rs   ← NEW
│   └── fuzz/
│       └── address_validation.rs
├── deploy/
│   ├── kubernetes/
│   │   ├── portus-deployment.yaml
│   │   └── portus-service.yaml
│   └── docker/
│       └── Dockerfile
└── docs/
    ├── api.md
    └── operator-guide.md
```

### D. Dispute-Window State Machine (new — reference implementation sketch)

This is the state machine underlying §5.5.1, made concrete. It's deliberately conservative: every transition is a pure function of `(current state, event, current_block, now)`, so it can be unit-tested exhaustively (per §11.1) without touching a real chain.

```rust
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentState {
    Pending,
    InFlight { claimed_by: RelayerId },
    DisputeWindowOpen { opened_at: Instant, window: Duration },
    Success { proof: ChainProof },
    Failed { error_code: u32 },
    RolledBack { marker_tx: TxHash },
}

#[derive(Debug, Clone)]
pub enum IntentEvent {
    RelayerClaimed(RelayerId),
    TimeoutBlockReached,
    ValidProofReceived(ChainProof),
    UnrecoverableFailure(u32),
    FraudulentProofRejected,
    DisputeWindowElapsed,
    RollbackMarkerWritten(TxHash),
}

#[derive(Debug)]
pub enum TransitionError {
    /// Event doesn't apply to the current state — e.g. a proof arriving
    /// after RolledBack is a bug in the caller (should have been
    /// rejected upstream by the ROLLED_BACK check in the endpoint
    /// contract itself, per §5.4), not a state we silently accept.
    InvalidTransition { from: &'static str, event: &'static str },
}

impl IntentState {
    pub fn apply(self, event: IntentEvent) -> Result<IntentState, TransitionError> {
        use IntentEvent::*;
        use IntentState::*;

        match (self, event) {
            (Pending, RelayerClaimed(id)) => Ok(InFlight { claimed_by: id }),

            // Happy path: proof arrives before timeout, no dispute window needed.
            (InFlight { .. }, ValidProofReceived(proof)) => Ok(Success { proof }),

            (InFlight { .. }, UnrecoverableFailure(code)) => Ok(Failed { error_code: code }),

            // Timeout does NOT go straight to rollback — this is the fix from v0.1.
            (InFlight { .. }, TimeoutBlockReached) => Ok(DisputeWindowOpen {
                opened_at: Instant::now(),
                window: Duration::default(), // caller sets the real per-chain window
            }),

            // A late-but-valid proof arriving *during* the dispute window still
            // resolves as Success — this is the race-closing behavior.
            (DisputeWindowOpen { .. }, ValidProofReceived(proof)) => Ok(Success { proof }),

            (DisputeWindowOpen { .. }, FraudulentProofRejected) => {
                // stay in DisputeWindowOpen; a rejected fraudulent proof
                // doesn't advance the clock or the state
                Err(TransitionError::InvalidTransition {
                    from: "DisputeWindowOpen",
                    event: "FraudulentProofRejected",
                })
            }

            // Only after the window elapses with nothing valid received do we
            // write the on-chain ROLLED_BACK marker.
            (DisputeWindowOpen { opened_at, window }, DisputeWindowElapsed) => {
                debug_assert!(opened_at.elapsed() >= window);
                // caller is responsible for actually writing the marker and
                // feeding back RollbackMarkerWritten; this transition just
                // gates that the window really did elapse before allowing it
                Err(TransitionError::InvalidTransition {
                    from: "DisputeWindowOpen",
                    event: "DisputeWindowElapsed — write marker first, then apply RollbackMarkerWritten",
                })
            }

            (DisputeWindowOpen { .. }, RollbackMarkerWritten(tx)) => Ok(RolledBack { marker_tx: tx }),

            (state, event) => Err(TransitionError::InvalidTransition {
                from: state_name(&state),
                event: event_name(&event),
            }),
        }
    }

    /// Used by §11.1's exhaustiveness test: every (state, event) pair not
    /// explicitly handled above must be a rejected transition, never a panic.
    pub fn is_terminal(&self) -> bool {
        matches!(self, IntentState::Success { .. } | IntentState::Failed { .. } | IntentState::RolledBack { .. })
    }
}

fn state_name(s: &IntentState) -> &'static str { /* match arms omitted for brevity */ "unknown" }
fn event_name(e: &IntentEvent) -> &'static str { /* match arms omitted for brevity */ "unknown" }

pub type RelayerId = String;
pub type ChainProof = Vec<u8>;
pub type TxHash = String;
```

**Test obligation this implies (should be added to §11.1):** an exhaustive test over the full `IntentState × IntentEvent` cross product, asserting that every transition either matches one of the explicit arms above or returns `InvalidTransition` — never panics, and never silently no-ops. This is what makes "no double payout" a property you can actually check in CI rather than an assertion in prose.