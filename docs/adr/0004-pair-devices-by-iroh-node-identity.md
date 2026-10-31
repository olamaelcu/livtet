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

Three properties of that design block peer discovery:

1. **The address is baked into the ticket.** The URI carries the host and port
   the device must reach, so the peers must already be mutually reachable
   (same LAN, or a port forward). There is no discovery step.
2. **The ticket is an artifact, not an identity.** Once redeemed it is dead.
   Nothing in the flow names the peer in a way that survives the pairing.
3. **Nothing advertises the desktop.** The iOS client (`NWBrowser`) and the
   Android client (`NsdManager`) already browse `_livtet-sync._tcp`, but no
   process ever registered that service — the `livtet_mdns::advertise_sync`
   the Android source cites does not exist.

`iroh` provides the missing identity and address-resolution layer: an endpoint
with a stable node id (public key), relay/pkarr/DNS lookup, and an mDNS address
lookup for the LAN. The id *is* the peer's name on the network and survives
address churn.

The mobile apps also cannot consume in-process Rust APIs: the UniFFI facade
(`livtet-ffi`) exposes the data layer only, and iOS pairing is stubbed out with
`syncUnavailable` for that reason.

## Decision

Introduce a workspace crate, `livtet-iroh`, that owns peer identity and
discovery, and make pairing key on that identity over **HTTP**:

1. **Identity.** `PeerIdentity { node_id, name, device_type }` and `NodeId`.
   `NodeId` parses the wire form (hex from iroh, or the Crockford-base32 ULID
   older clients send) and rejects empty/oversized/whitespace-bearing values.
   `pending_pairings.token` — already the row key — stores the node id; no
   schema change is needed.
2. **Discovery.** `DiscoveryService::bind()` binds an `iroh` endpoint with the
   `N0` preset (relay + pkarr + DNS) and attaches
   `iroh_mdns_address_lookup::MdnsAddressLookup`, so a node id resolves to a
   current address on the LAN, falling back to the relay. The daemon binds it
   once at startup; a failure degrades to explicit addresses, not to a crash.
3. **Claim.** `pairing.begin` publishes the identity and creates no row.
   `POST /sync/pair` upserts the `pending_pairings` row keyed by the caller's
   node id, recording the origin address and a 300-second expiry; a decided row
   is never reopened by replay (409). `pairing.list`, `pairing.approve`, and
   `pairing.reject` address the row by node id, and the JSON-RPC parameter
   keeps its `token` key for wire compatibility.
4. **Mobile integration is HTTP, not FFI.** `GET /sync/identity` returns
   `{ node_id, device_id }` so a discovered peer can confirm the identity it
   saw in the mDNS record; `/sync/pair` and `/sync/pair/status/:node_id` carry
   the rest. `livtet-ffi` gains no pairing API — one fewer lifecycle to keep
   in step with the Rust endpoint, and the same surface a future CLI can use.
5. **Advertisement.** With the HTTP server running, the daemon registers
   `_livtet-sync._tcp` with TXT `id=<node_id>`, `device_id`, `version`,
   `hostname`, `deviceFlavor=desktop`, and the SRV record pointing at the
   bound HTTP port. The record is withdrawn when the server stops. A bound
   `0.0.0.0` publishes every interface address; an explicit address (including
   loopback) is published as-is.
6. **Handoff stays manual.** A device that cannot browse mDNS enters the node
   id (or scans it as a QR code) and the address, then posts the claim. No
   pairing URI is minted any more.
7. **Transport.** Unchanged: HTTP `/sync/*` carries status, changes, push,
   conflicts, and files. Only identity, discovery, and advertisement move to
   iroh/mDNS, so the data path does not depend on the relay being reachable.

`livtet-core` re-exports the crate as `core::iroh`, so both the desktop daemon
and any future Rust consumer reach the same primitives from one dependency.

## Consequences

### Easier

* The address is no longer part of the ticket: a peer is named by its node id,
  and mDNS or the relay supplies a current address.
* Both clients share one identity type and one validation rule.
* The existing iOS and Android browsers start returning results; no client
  rewrite is needed to discover, only to adopt the new claim body.
* `pairing.begin` no longer writes a row that the user must clean up; only real
  claims appear in the list, and expired rows are pruned on startup.

### Harder / to watch

* **Loopback default.** The desktop launches the daemon with `--host
  127.0.0.1`, so the advertisement is only reachable from the same machine (or
  an iOS simulator sharing its stack). LAN peering needs a non-loopback bind,
  which is gated on authenticating `/sync/*` (see below).
* **Two mDNS records.** `_livtet-sync._tcp` (app discovery, HTTP port) and
  iroh's own address-lookup records (node id → QUIC address) are separate
  protocols with separate lifetimes. They must both come down on shutdown.
* **Deployment ordering.** The desktop consumes `livtet-*` as a git dependency,
  so the desktop's identity DTOs and the daemon's identity responses must land
  in the same revision, or `pairing.begin`/`list` fail to deserialize.
* **TLS provider is global.** `iroh-relay` enables `rustls-no-provider` on
  `reqwest`, so any `reqwest::Client` built in a process that links iroh panics
  unless a provider is selected. `livtet-sync-http` enables `reqwest`'s
  `rustls` feature to select one for the whole workspace.
* **Mobile clients still speak the old claim.** iOS posts `token` and polls by
  token; both must move to `node_id` before pairing works end to end.

### Follow-ups

* Session-token validation on `/sync/*` (still deferred from desktop ADR 12) —
  required before the daemon binds a non-loopback interface by default.
* Migrate the iOS `NetworkClient` claim/SSE to `node_id`, and add a claim client
  to Android (today it only browses).
* Surface discovered peers in the desktop UI, driven by the daemon's mDNS/iroh
  events instead of one-off manual entry.
* File and cover transfer over the sync channel; `get_file` is unauthenticated.
