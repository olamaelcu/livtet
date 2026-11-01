# 8. Carry sync over iroh streams and retire livtet-sync-http

Date: 2026-11-01

## Status

Accepted. Supersedes the transport half of [1](0001-split-sync-into-domain-transport-daemon-crates.md)
and the address handling of [6](0006-re-resolve-a-paired-peers-address-from-its-advertisement.md),
including the `listen_on` that [7](0007-pairing-is-mutual.md) adds to the claim.

## Context

Sync data travels over plain HTTP. `livtet-sync-http` serves `/sync/*` from a
`poem` server and dials it with `reqwest`; `livtet-sync-server` drives both from
the daemon. Pairing is already keyed by an iroh node id
([4](0004-pair-devices-by-iroh-node-identity.md)), and every daemon already binds
an iroh endpoint with the `N0` preset, relays, pkarr publishing and an mDNS
address lookup (`livtet-iroh::DiscoveryService`). No bytes use it: a search for
`Endpoint::connect` in the workspace finds nothing.

The consequence is that **the receiver must open an inbound TCP connection to
the sender**, so any host or network that drops inbound TCP makes receiving
impossible while sending still works:

- A Debian laptop with ufw's default `deny (incoming)` accepted the desktop's
  `/sync/pair` (outbound) and answered `ping` (stateful), while every inbound
  TCP port — 7710, 8080, even a throwaway `python3 -m http.server` — timed out
  from the other machine. Same LAN, both apps running, nothing in the app able
  to tell a firewalled port from an absent one.
- The same shape occurs with AP client isolation, CGNAT and VPN policies. None
  of them can be fixed from inside the app: opening a firewall needs root, and
  Tauri exposes no API for it.

The address churn that
[ADR 6](0006-re-resolve-a-paired-peers-address-from-its-advertisement.md)
and "remember the listen port" work around exists only because the peer's
reachable address is an ephemeral `host:port` pair. iroh resolves a node id to
a current address (direct, hole-punched, or relayed), so that class disappears.

## Decision

Sync moves onto iroh request/response streams and `livtet-sync-http` is deleted.

- **One ALPN, framed messages.** `livtet/sync/1` on the existing endpoint.
  Each operation is a `u32` length-prefixed JSON request on a fresh
  bidirectional stream and one JSON response on the same stream; file and cover
  reads append a raw byte phase after the response header. The operations mirror
  today's routes one for one: identity, status, changes, pull-full, push, pair,
  pair decision, conflicts, resolve, file, cover, offer decision.
- **Authorization is per frame.** The session token minted at approval stays in
  `paired_devices.session_token` but travels as a field of the request instead
  of an `x-livtet-session` header, and is checked against the connection's remote
  node id. The pairing claim is the one unauthenticated operation, as before.
- **Pairing holds a connection instead of an SSE stream.** The claimant opens a
  stream, sends `Pair`, and waits on the same stream for the decision frame. The
  approval UI, the pending table and the `pairing.claimed` notification are
  unchanged.
- **The peer's node id is its address.** `paired_devices.listen_on` stops
  carrying an HTTP endpoint; addresses are resolved by iroh on every dial, so
  `re_resolve_peer_address`, `preferred_address`, the port memory and the
  `--host/--port/--lan` spawn flags all retire. mDNS keeps advertising the
  service for discovery (name, node id, version); its SRV port stops being
  dialled.
- **`livtet-sync-http` goes away**, with its `poem`/`reqwest` features, routes,
  session/rate-limit helpers and tests. The transport-agnostic pieces it owns
  (`SyncHttpClient` trait, `SyncSession`, the request DTOs) move into
  `livtet-sync`, where the domain already lives, and the iroh implementation
  lands in a new `livtet-sync-iroh`.

## Consequences

### Easier

- No inbound port to open on either side, so a firewall, CGNAT or an isolated
  access point stops being a sync bug. Both sides dial out.
- One identity end to end: the node id the user pairs with is the address the
  transport dials, so a dead pairing cannot look healthy.
- The desktop's firewall callout and HTTP diagnostics have nothing to report;
  they become endpoint/relay/peer-resolution checks instead.

### Harder / to watch

- **Bulk files ride the relay when hole punching fails** — precisely the
  hostile-network case that motivates the change. Relays forward encrypted QUIC
  packets, but their capacity is not ours; a large library on a blocked network
  will be bandwidth-bound on the n0 relays, so self-hosting a relay becomes the
  escape hatch.
- **The approval window is now a live connection.** A pairing claim that waits
  minutes for a human must survive QUIC idle timeouts; the claimant needs to
  keep the stream (or reconnect) rather than treat a closed stream as a refusal.
- **Anything outside this workspace that speaks `/sync/*` breaks.** The
  `_livtet-sync._tcp` record exists partly for the iOS (`NWBrowser`) and Android
  (`NsdManager`) clients; those clients must move to the framed protocol before
  the HTTP surface is removed.
- iroh 1.x is now on the critical path for data, not just discovery, so an
  upstream breaking change is a sync outage rather than a discovery degradation.

## Migration

1. `livtet/sync/1` codec and message types in a new `livtet-sync-iroh`; the
   transport-agnostic client trait and DTOs move into `livtet-sync`.
2. `livtet-sync-server` gains the accept loop and dispatches frames to the
   engine calls the routes made now — sliced: identity, then pull, then
   pairing, then files and covers.
3. Client calls dial by node id; URL building, address re-resolution and the
   spawn flags are removed.
4. Delete `livtet-sync-http` and the `poem`/`reqwest` dependencies; prune the
   workspace.
5. Desktop: drop the firewall hint and the HTTP probes, regenerate bindings.

Until step 4 the firewall hint in the desktop stays: it is the only thing that
makes a firewalled peer work today.
