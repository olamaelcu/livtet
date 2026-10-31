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
   Enumeration is separate: iroh's lookup resolves a *known* node id, so the
   daemon browses the `_livtet-sync._tcp` record on request
   (`discovery.nearby`, a two-second listen) to list the peers advertising it.
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
5. **Data routes require a session.** `/sync/status`, `/changes`, `/pull-full`,
   `/push`, `/conflicts`, `/conflicts/:id/resolve`, and `/files/:id` accept
   either `Authorization: Bearer <token>` or `x-livtet-session: <token>` and
   validate it against `paired_devices.session_token`. Absent and unknown
   tokens are both `401`. `/sync/identity`, `POST /sync/pair`, and the status
   stream stay public — the stream is gated by a claim instead.
6. **The claim is a secret.** `POST /sync/pair` returns
   `{ status: "pending", claim }`; `/sync/pair/status/:node_id?claim=…` opens
   only when the claim matches. Without it, anyone who learned a node id could
   subscribe and read the session token out of the approval event.
5. **Advertisement.** With the HTTP server running, the daemon registers
   `_livtet-sync._tcp` with TXT `id=<node_id>`, `device_id`,
   `name=<friendly name>`, `version`, `hostname`, `deviceFlavor=desktop`, and
   the SRV record pointing at the bound HTTP port. The name comes from the
   daemon's `--device-name` flag; without one (or when it is blank) the name
   is generated stably from the device id, so a peer never has to show a ULID.
   The desktop persists the chosen name in its settings, passes it at spawn,
   and applies later edits live with `discovery.rename`, which re-registers
   the record. The record is withdrawn when the server stops. A bound `0.0.0.0`
   publishes every interface address; an explicit address (including loopback)
   is published as-is.
7. **LAN is opt-in, not a hardcoded flag.** The daemon binds `127.0.0.1` by
   default and `--lan` (or `--host 0.0.0.0`) opts into a LAN-reachable bind;
   the desktop passes that flag from its own setting. With an unspecified bind
   the mDNS record advertises every interface address.
8. **Handoff stays manual.** A device that cannot browse mDNS enters the node
   id (or scans it as a QR code) and the address, then posts the claim. No
   pairing URI is minted any more.
9. **Transport.** Unchanged: HTTP `/sync/*` carries status, changes, push,
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

* **LAN is a switch, not the default.** The daemon still starts on loopback;
  a LAN-reachable bind is opt-in (`--lan`), and the desktop must surface that
  switch. Until it does, discovery works but pairing only reaches the same
  machine (or an iOS simulator sharing its stack).
* **`POST /sync/pair` is unauthenticated by design.** Any device that can
  reach the port and knows a node id may file a claim; approval on this device
  is the gate. Claims are budgeted per source (ten a minute, `429` beyond
  that), which bounds table growth and approval spam without blocking a
  household behind one NAT.
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
* **Mobile clients are mid-migration.** iOS now claims by `node_id` and keeps
  the session token; sending `x-livtet-session` on data routes and Android's
  pairing UI remain. Until an app sends the header, the data routes answer
  `401` to it.

### Follow-ups

* Desktop LAN switch: pass `--lan` to the sidecar when the user enables it, and
  restart the daemon when the setting flips.
* Migrate the iOS `NetworkClient` claim/SSE to `node_id` and send the session
  header on data routes; add a claim client to Android (today it only browses).
* Discovered peers are surfaced in the desktop pairing panel: it polls
  `discovery.nearby` while open and lists peers by friendly name, device
  flavor, and address, with the node id one copy away. Push-based update
  events remain future work; polling keeps the daemon lifecycle unchanged.
* File and cover transfer over the sync channel; `get_file` is session-gated, but the transfer UX is pending.
