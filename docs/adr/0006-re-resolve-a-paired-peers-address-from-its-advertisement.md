# 6. Re-resolve a paired peer's address from its advertisement

Date: 2026-11-01

## Status

Accepted

## Context

`paired_devices.listen_on` is written once — when a pairing is approved or a
claim is filed — and every pull dials it. The daemon binds its embedded HTTP
server per launch (`--port 0` by default) and advertises whatever port it got
over mDNS, but nothing ever folded those advertisements back into the pairing
rows. So a peer that restarted kept a pairing that no longer resolved:

```
WARN livtet_sync_server::daemon: pull from paired device failed
  device_id=01M652BF35STC9CWKPF4WKDAZV
  error=server error: pull changes: error sending request for url
  (http://192.168.1.226:61057/sync/changes?since_version=110&limit=100)
```

once a minute, forever, with no path back except pairing again. Offers a peer
recorded (`ADR 5`) never arrived, so the receiver's prompt never opened. The
advertisement already carries the stable `device_id` in its TXT record, so the
information needed to repair the row was on the network the whole time.

The desktop side (ADR 54 in the desktop repository) now asks for a stable port,
which removes the common cause. This record covers the half that keeps a
pairing working when the cause is something else: a new DHCP lease, a moved
interface, or a peer still on an older build.

## Decision

`pull_device` becomes a wrapper around `pull_device_once`:

- The pull runs exactly as before, from the address on file.
- If it fails, `re_resolve_peer_address` browses `_livtet-sync._tcp` for
  `mdns::NEARBY_WINDOW`, finds the advertisement whose `device_id` matches the
  paired device, and rewrites `listen_on` when the resolved address differs.
- Only when that rewrite happened is the pull retried, once. Every other
  failure — a peer that is asleep, an expired token, a bad response — is
  reported unchanged, and a browse that heard nothing leaves the row alone.
- `preferred_address` picks the first routable address the advertisement
  lists, skipping loopback, unspecified, and link-local ones, and falls back to
  the first listed so two instances on one machine still sync.

No RPC surface changes: the retry is internal to `devices.pull` and the pull
tick.

## Consequences

### Easier

- A pairing survives a peer restart, a new port, and a new lease without the
  user pairing again.
- The failure stops being silent: the retry logs `peer address changed`, and a
  pull that still fails reports the transport error it started with.

### Harder / to watch

- A failed pull costs one blocking 2-second browse (off the async runtime, as
  `discovery.nearby` already does) before it is reported. With several
  unreachable peers that is 2 seconds each, once a minute.
- The row can be rewritten by any host on the link that advertises a paired
  `device_id`. That is the same trust boundary as pairing (mDNS is unauthenticated);
  the session token still gates every data route, so a forged address can
  only redirect a request that will be refused.
- A peer that changes its `device_id` (a reinstall) is a new device, not a
  moved one, and still needs pairing.
- The desktop only benefits once it pins a revision that contains this commit.
