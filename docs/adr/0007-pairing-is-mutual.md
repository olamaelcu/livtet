# 7. Pairing is mutual

Date: 2026-11-01

## Status

Accepted

## Context

Pairing recorded a direction, not a relationship. The device that *approved*
a claim wrote `paired_devices.outbound = false`, and `pull_device` refuses that
flag, so the approver could never read the claimer — while the claimer could
read the approver freely. The asymmetry was invisible in the UI (both devices
listed each other) and read as a network fault: one machine pulls, the other
silently never does.

The token never justified the asymmetry. Approval mints one `session_token`,
stores it, and sends the same value to the claimer; each side keeps it, so
either can authenticate to the other — `authorize` simply looks the presented
token up in the local `paired_devices` row. The only thing missing was the
address: `post_pair` stored the address the claim was *served* on, which is the
approver's own, so a mutual pairing would have dialled itself.

## Decision

- `PairRequest` carries an optional `listen_on`: where the claiming device
  serves. The daemon fills it from its own bind — the bound address when that
  is specific, otherwise the interface the kernel would route from.
- `post_pair` stores that address; the served address remains the fallback for
  older peers.
- Approval sets `outbound = true` only when it has the peer's own address. A
  claim from a pre-mutual peer leaves `outbound = false` and no address, so it
  stays one-way instead of dialling this device.
- On startup, rows with a token and a specific address are promoted to
  `outbound = true`, so pairings approved before this change gain their missing
  half without re-pairing. Wildcard addresses (`0.0.0.0`, `[::]`) are skipped:
  those are the old approval's own address, not a peer.
- The daemon reports 0.2.1, so the two ends can be told apart while rolling
  this out.

## Consequences

### Easier

- One pairing action gives both directions: books, offers, and pushes work
  either way without a second handshake.
- Existing one-way pairings repair themselves on the next start.
- "Pull works, the other direction does not" stops being a legitimate symptom.

### Harder / to watch

- The token is a shared bearer secret by construction. It already was — both
  sides stored the same value — but that is now load-bearing rather than
  incidental, so it should be treated like a password per pairing.
- A peer that predates this change still needs to claim again (or be claimed)
  before the missing direction exists; the promotion can only help rows that
  already carry a peer address.
- `outbound` no longer records *who filed the claim*, only whether this device
  may pull. Anything that wanted the old meaning needs another source.
