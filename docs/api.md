# tpt-portus gRPC API

Service `tpt.portus.v1.PortusService`, defined in `proto/portus/v1/portus.proto`. The
server is `grpc::server::PortusGrpc<E>` over any `engine::PortusEngine`. Only the in-memory
engine exists so far, and it is not durable.

## Methods

| RPC | Request | Response | Status |
|---|---|---|---|
| `ExecuteIntent` | `IntentExecutionPlan` | `AcceptanceAck` | Implemented |
| `GetIntentStatus` | `IntentIdQuery` | `ExecutionReceipt` | Implemented |
| `WatchIntentStatus` | `IntentIdQuery` | stream of `ExecutionReceipt` | Returns `UNIMPLEMENTED` until the event bus exists |
| `SubmitProof` | `ProofSubmission` | `ProofAck` | Stake and rate checks run, then it rejects. Verification is not wired to the engine yet. |
| `GetDisputeWindow` | `ChainQuery` | `DisputeWindowConfig` | Implemented |
| `HealthCheck` | `HealthRequest` | `HealthResponse` | Implemented. Reports `engine: ok` only. |

## Behaviour

**ExecuteIntent**
- An identical plan resubmitted under the same `intent_id` returns an accepted
  `AcceptanceAck`. The receipt is read with `GetIntentStatus`.
- A different plan under an existing `intent_id` is rejected as a replay.
- `dispute_window_secs` must equal the value Portus has configured for the destination chain.
  A mismatch is rejected, so Statera cannot silently use its own window (DESIGN.md §4.5).
- A destination chain with no configured window is rejected.

**SubmitProof** checks in this order:
1. The intent exists (otherwise `NOT_FOUND`).
2. `relayer_stake_ref` is registered. Otherwise the ack is `accepted = false` with the reason
   "relayer stake reference not registered".
3. The relayer is within its rate limit (otherwise "rate limited").
4. Verification. This is not implemented, so the ack is `accepted = false`. No proof is ever
   accepted without verification.

**GetIntentStatus** returns the status derived from the intent's state machine. The timeout
window closes on the clock, not on an event, so a proof after the close is rejected.

## Status codes

| Condition | gRPC code |
|---|---|
| Intent not found | `NOT_FOUND` |
| Replay (same id, different plan) | `ALREADY_EXISTS` |
| Zero amount, timeout not in the future, bad address, window drift, malformed field | `INVALID_ARGUMENT` |
| Unsupported destination chain, no route, insufficient liquidity or relayer diversity, window still open, proof verification failed | `FAILED_PRECONDITION` |
| Anything else | `INTERNAL` |
| Streaming status | `UNIMPLEMENTED` |

## Lossy fields

The proto `ExecutionStatus` is a bare enum (DESIGN.md §8.1). The domain status carries data
(§4.3). Over the wire, only the status kind is sent. These fields are not sent:

- `tx_hash` and `block_number` for `Success`
- `error_code` and `revert_reason` for `Failed`
- `current_block` and `timeout_block` for `Timeout`
- `closes_at_secs` for `DisputeWindowOpen`

`ExecutionReceipt.chain_proof` and `portus_signature` are sent. The signature is empty until
the HSM is integrated.

## Identifiers

- `intent_id` is a UUID in its hyphenated string form.
- `ChainId` is a tagged union: `evm`, `cosmos`, `solana`, or `bitcoin`. Cosmos chains are
  identified by their IBC chain-id string, for example `osmosis-1`.
