# Runbook: destination-chain outage beyond the dispute window

Source: DESIGN.md §10.2 (residual risk) and §14. This runbook covers the case the design
says cannot be fully automated: a destination chain stops for longer than the dispute window,
rollback proceeds, and the chain later resumes and confirms the transfer after all.

## When this applies

- An intent is `RolledBack` (the `ROLLED_BACK` marker is written and the `RollbackRequest` is
  queued for Statera), **and**
- the destination chain later confirms a transaction for the same intent, or the recipient
  reports receiving funds for it.

Both funds have now been released: the user's source funds were unlocked by Statera, and the
destination transfer completed. This is a double payment. Nothing in the protocol resolves it.

## What the code does and does not do

| Step | Available now | Notes |
|---|---|---|
| Find rolled-back intents | Yes, via `GetIntentStatus` returning `ROLLED_BACK`, and `drain_rollback_requests` in-process | The drain is in-memory and is lost on restart. Do not rely on it for records. |
| Detect a late destination confirmation | **No** | No chain monitor exists. Detection is manual: query the destination chain's explorer or RPC for the recipient and amount. |
| Stop further payouts for the intent | **No** | Terminal states accept no events, so the intent cannot be re-executed. Nothing else needs stopping. |
| Unlock or claw back source funds | **No** | Statera owns this. Portus only emits the request. |
| Reverse a destination transfer | **No** | Depends on the destination chain. Usually off-protocol. |

## Procedure

1. **Confirm the case.** Record the `intent_id`, the destination chain, the `marker_tx` from the
   rollback, and the late confirmation's transaction hash and block. Check it on the chain
   explorer, not only in a relayer report.
2. **Open an incident record** with the owners of Portus and Statera. Do not decide the
   resolution alone. The design leaves it open.
3. **Freeze related intents if needed.** Portus has no pause control yet. Until one exists, the
   practical stop is to refuse new plans to that destination chain at Statera, which owns plan
   generation. Coordinate this with Statera.
4. **Quantify the exposure.** Sum the amounts of every rolled-back intent to that destination
   during the outage, using `GetIntentStatus` for each `intent_id`. Include intents that rolled
   back and were later confirmed, and intents that were not yet confirmed.
5. **Choose a resolution with the owners.** The options are outside the protocol and are
   business decisions:
   - Ask the recipient to return the duplicate funds, with the amount and transaction recorded.
   - Absorb the loss from a reserve. Note that the 10% slash reserve in §5.2.1 is for relayer
     slashing, not for this. The spec does not designate a source for this loss.
   - Leave the duplicate in place and record it as a loss.
6. **Record the outcome** against each `intent_id` in the incident record. Do not edit intent
   state, because the state machine is terminal. The record lives in the incident log.
7. **Post-incident.** Decide whether the dispute window for that chain was too short. Changing
   the window is a config change that Statera must follow (§4.5): bump the config version,
   update `portus.toml`, and coordinate the rollout.

## Known gaps to close before production

- A durable store for intent state and the rollback outbox. The current engine is in memory.
- A chain monitor that flags destination confirmations for intents already rolled back.
- A pause control for a destination chain, owned by Portus operators.
- An agreed owner and funding source for unrecoverable duplicates.
