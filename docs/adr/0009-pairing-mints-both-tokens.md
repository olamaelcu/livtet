# Pairing mints both tokens in one claim

Date: 2026-11-08

Status: Accepted. Supersedes [ADR-0007](0007-pairing-is-mutual.md).

## Context

One claim minted one token. The approver stored it in `session_token` and sent
it to the claimant, which stored it in `peer_session_token`. Inbound auth
reads only `session_token`; outbound dialling reads only `peer_session_token`.
So the token authenticated the claimant to the approver and nothing else, while
ADR-0007 and the UI both said the pairing was mutual. A reversed claim could
not repair it: the approver refuses any claim whose pending row is already
`Approved` or `Rejected`, and that row survives until daemon restart.

## Decision

- The sync protocol is revision 3 (revision 2 was the iroh-streams cut). Older
  peers are refused for every sync operation with
  an explicit “update Livtet on that device”; the client stops retrying.
- A pairing claim carries the claimant's own pre-minted token. The approver
  stores it as `peer_session_token`, mints its `session_token`, and returns it.
  The claimant writes both tokens when it records the pair.
- Both tokens are minted fresh on every approval. The claimant's token lives
  only in memory until approval; restarting the app cancels the claim.
- Claims are refused while this device still has a paired row for the peer.
  Repair is therefore explicit: an authenticated remote revoke plus a local
  revoke, then a normal claim. If the peer is offline, only the local row is
  removed, with a notice.
- Revoke removes the local pending row for that node. A new claim arriving with
  no local paired row resets any leftover decided row to `Pending`; no daemon
  restart is needed.
- The tokens are named **inbound token** (we issued it, authorizes the peer) and
  **outbound token** (the peer issued it, we present it). Database columns keep
  their names.

## Consequences

- One claim gives both directions; “one-way pairing” remains only as a legacy
  state to be repaired.
- Mixed versions cannot sync until both sides run the same revision.
- Revoke/Repair is a two-sided operation by intent; the remote call is
  best-effort and reported when it fails.
