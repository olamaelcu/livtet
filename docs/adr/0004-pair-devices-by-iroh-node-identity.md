# 4. Pair devices by iroh node identity instead of a single-use pairing token

Date: 2026-10-31

## Status

Accepted

## Context

Pairing — the step that admits a second device to a library — was built around
a **single-use token**. `pairing.begin` minted a ULID, inserted a
`pending_pairings` row keyed by that token, and returned
`livtet://sync?host=…&port=…&token=…`. A remote device had to receive that URI
out of band (QR code, copy/paste) and POST it back to `/sync/pair`; the desktop
then approved or rejected the row.

Two properties of that design block peer discovery:

1. **The address is baked into the ticket.** The URI carries the host and port
   the device must reach, so the peers must already be mutually reachable
   (same LAN, or a port forward). There is no discovery step: a device that can
   see the network cannot find a peer on it.
2. **The ticket is an artifact, not an identity.** Once redeemed it is dead.
   Nothing in the flow names the peer in a way that survives the pairing, so a
   later connection has nothing to authenticate against except the minted
   `session_token`.

`iroh` provides the missing layer: an endpoint with a stable node id (public
key) and discovery over relays, DNS/Pkarr, and mDNS. The id *is* the peer's
name on the network, and it survives disconnects and address changes.

## Decision

Introduce a workspace crate, `livtet-iroh`, that owns peer identity and
discovery, and make pairing key on that identity:

1. **Identity.** `PeerIdentity { node_id, name, device_type }` and `NodeId`.
   `NodeId` parses the wire form (hex from iroh, or the Crockford-base32 ULID
   older clients send) and rejects empty/oversized/whitespace-bearing values.
   `pending_pairings.token` — already the row key — stores the node id; no
   schema change is needed.
2. **Discovery.** `DiscoveryService` is the seam peers start before pairing.
   `pairing.begin` starts it, publishes this device's identity, and creates no
   row: a pending pairing exists only once a *remote* device has actually
   asked to pair.
3. **Claim.** `POST /sync/pair` upserts the `pending_pairings` row keyed by the
   caller's node id, recording the origin address and a 300-second expiry. A
   decided row is never reopened by replay (409). `pairing.list`,
   `pairing.approve`, and `pairing.reject` address the row by node id, and the
   JSON-RPC parameter keeps its `token` key for wire compatibility.
4. **Transport.** Unchanged: HTTP `/sync/*` still carries status, changes,
   push, conflicts, and files. Only discovery and identity move to iroh, so the
   data path does not depend on the relay being reachable.
5. **Inheritance.** `livtet-core` re-exports the crate as `core::iroh`, so both
   the desktop daemon and the mobile FFI reach the same primitives from one
   dependency.

## Consequences

### Easier

* The address is no longer part of the ticket: a peer is named by its node id,
  and discovery can supply a current address later.
* Both clients share one identity type and one validation rule.
* `pairing.begin` no longer writes a row that the user must clean up; only real
  claims appear in the list, and expired rows are pruned on startup.

### Harder / to watch

* **Deployment ordering.** The desktop consumes `livtet-*` as a git dependency.
  The desktop's identity DTOs and the daemon's identity responses must land in
  the same revision, or `pairing.begin`/`list` fail to deserialize.
* **Two notions of id.** `DiscoveryService` currently publishes the daemon's
  device id; once the endpoint is bound it must publish iroh's endpoint id
  instead, and that is the value clients will key on.
* **TLS provider is now global.** `iroh-relay` enables `rustls-no-provider` on
  `reqwest`, so any `reqwest::Client` built in a process that links iroh panics
  unless a provider is selected. `livtet-sync-http` enables `reqwest`'s
  `rustls` feature to select one for the whole workspace; adding another
  `reqwest` user with `default-features = false` must not drop it.

### Follow-ups

* Bind a real `iroh::Endpoint` in `DiscoveryService` (mDNS for the LAN, relay
  for the wider network) and switch `pairing.begin` to the endpoint's node id.
* Session-token validation on `/sync/*` (still deferred from desktop ADR 12) —
  required before the server binds a non-loopback interface.
* Expose `DiscoveryService` and `PeerIdentity` through `livtet-ffi` once the
  endpoint lifecycle is settled.
* File and cover transfer over the sync channel; `get_file` is unauthenticated
  today.
