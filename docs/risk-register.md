# Dependency & Trust Risk Register

Tracked risks that sit outside `tpt-portus`'s own code and audit. Source: DESIGN.md §6.2, §14.

| ID | Dependency | Risk | Assumption / mitigation | Status |
|---|---|---|---|---|
| DR-001 | `tpt-telos` (compiler backend) | The formal verifier's compiler backend may be unsound. Any `#[telos::verify]` guarantee inherits this. | Treated as a trust assumption, not a given. Tracked separately from Portus's own audit (§6.2). | Open |
| DR-002 | External chain consensus (Ethereum PoS, Tendermint BFT, Solana PoH) | Safety depends on each chain's consensus at the configured confirmation depth. | Assumed secure at `finality_blocks` / `finality_slots` / `confirmation_depth` from `portus.toml` (§6.2). | Open |
| DR-003 | `tpt-statera` | Sole authority for `IntentExecutionPlan`. A compromised signing key is a direct double-spend vector. | Threshold signature scheme (§6.3). HSM recommended. Scheme choice (threshold-Ed25519 vs BLS) is still open (§14). | Open |

## Open spec items affecting this register

- **Chain identity in config.** DESIGN.md §7.1 still shows numeric `chain_id` keys. §4.2 states these are a bug. The shipped `portus.toml` omits them and identity comes from `ChainId`.
- **License identifier.** Resolved as `MIT OR Apache-2.0` (see `LICENSE-MIT`, `LICENSE-APACHE`).

## Blocked formal-verification obligations

| ID | Obligation | Blocked on | Notes |
|---|---|---|---|
| DR-004 | Dust-threshold `ensures: abs(value(destination_amount) - value(amount)) <= dust_threshold` (DESIGN.md §4.2.1) | `tpt-telos` fixed-point reasoning support (§14) | The conversion rule itself is not yet implemented. Only the `AssetDescriptor` struct exists. |
| DR-005 | `#[telos::verify]` contract annotations on `IntentExecutionPlan` and `ExecutionReceipt` (DESIGN.md §4.2, §4.3) | `tpt-telos` is an external repository and not available here | Structs are defined without the annotations. Add them when the verifier is available. |
