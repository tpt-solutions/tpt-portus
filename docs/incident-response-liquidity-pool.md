# Incident response: liquidity pool exploited during an in-flight transfer

Source: DESIGN.md §2.2 (non-goals) and §14 (third-party liquidity pool risk). Portus does not
own the liquidity pools it routes through. A pool can be drained while a transfer is in
flight, and the user's funds are still affected. This plan covers that case.

## Scope

A third-party pool on a route that Portus used for a live intent is compromised, paused, or
drained. Examples: an AMM with a drained reserve, an exploited bridge contract, or an oracle
that feeds the pool a bad price.

## What Portus can and cannot do today

| Action | Available now | Notes |
|---|---|---|
| Stop routing new intents through the pool | **No** | The router (`src/router/liquidity.rs`) takes candidates as input. There is no registry or deny-list yet. Statera or the operator must remove the pool from the candidate set. |
| Stop dispatching an in-flight plan | **No** | Once claimed, the plan proceeds. |
| Detect the exploit | **No** | No pool monitor exists. Detection is from external alerts and the pool's own status. |
| Let in-flight intents resolve | Yes | An intent that cannot complete reaches its timeout. The dispute window then runs, and if no valid proof arrives, a rollback is recorded and a `RollbackRequest` is queued for Statera (§5.5.1). |
| Confirm which intents used the pool | Partly | `GetIntentStatus` returns the destination chain, not the pool. Pool attribution needs a route record, which is not stored today. |

## Response steps

1. **Declare the incident** and name an incident lead. Notify the owners of Portus and Statera,
   and the owner of the pool's route configuration.
2. **Stop new exposure.** Remove the pool from candidate routes, through Statera's plan
   generation and any operator route list. Record the time.
3. **Identify affected intents.** Pull every intent in a non-terminal state
   (`PENDING`, `IN_FLIGHT`, `DISPUTE_WINDOW_OPEN`) whose route used the pool. Until a route
   record exists, use the transfer logs of the chain that carried the pool's leg.
4. **Let the protocol run.** Do not force-roll-back intents. The timeout and dispute window
   already protect users (§5.5.1). Forcing a state change would skip the window the design relies
   on. An intent that completes through a healthy route is still valid.
5. **Check the rollback outcome for each affected intent.** Rolled-back intents mean Statera has
   been asked to unlock source funds. Confirm Statera unlocked them. Use the procedure in
   `docs/runbook-reconciliation.md` for any intent whose destination leg later confirms.
6. **Quantify the loss.** Separate losses from the pool (the pool's own funds) from user funds
   that were stuck in flight. Only the second is a Portus concern.
7. **Communicate.** Tell affected users what happened, what is pending, and how to reach
   support. Do not state a loss figure before it is reconciled.
8. **Close and review.** Write a postmortem covering the detection gap, the missing deny-list,
   and the missing route record. Those two gaps are what turns this from a procedure into a
   safe response.

## Open decisions for the owners

- Who covers user losses that the protocol cannot recover, and from what source. The 10%
  slash reserve in §5.2.1 is for relayers, and the spec does not assign a source for pool losses.
- Whether Portus should refuse routes to a pool automatically on a health signal, or only on an
  operator decision.

## Gaps to close before production

- A route registry with an operator deny-list that the router consults.
- A route record per intent, stored durably, so affected intents can be found directly.
- A pool health monitor that feeds the deny-list.
