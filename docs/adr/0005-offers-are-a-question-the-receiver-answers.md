# 5. Offers are a question the receiver answers

Date: 2026-11-01

## Context

ADR 4 made pairing an identity relationship and ADR 48 (desktop) made sending a
book an **offer + pull**: the sender appends a `file_offer` change row, and the
receiver's daemon downloads the file and applies the catalog rows on its next
pull tick. Nothing asks the reader. A book shoved onto a phone by a mistake
lands in its library, costs the transfer, and has to be deleted by hand.

The desktop wants a dialog showing the book's cover, title, and authors, with
accept and decline; the sender is told which. Three things were missing:

- A pull had no way to stop short of the library.
- The offer payload named ids and a file, not a book: no title, no authors.
- The receiver had no channel back to the sender.

## Decision

An offer is recorded, not consumed.

- **Payload.** `devices.offer_book` adds `title` (edition title, else work
  title) and `authors` (work contributors) to the offer payload, so the
  decision can be shown without any catalog row having landed.
- **Recording.** `pull_device` writes each offer addressed to this device into
  a new client table, `inbound_offers` (migration `client-0009`), and holds
  back the `work`, `edition`, and `digital_inventory` rows whose entity ids
  the payload names. Everything else in the page applies as before; the cursor
  still advances, so the held rows are only ever replayed from the offer row.
- **Deciding.** `offers.decide { offer_id, accepted }` is the only path that
  downloads. Accepting fetches the file and cover, replays the held rows with
  the receiver's paths, and clears them; declining discards them. Either way
  the daemon then `POST`s `/sync/offers/:offer_id/decision { accepted }` to
  the sender, using the token that sender issued, and reports `delivered`
  instead of failing when the peer has gone away.
- **Reading.** `offers.list` returns the pending offers (`from_name` joined
  from `paired_devices`); `offers.cover` proxies the cover from the sender as
  a data URL, because the bytes have not been downloaded yet.
- **Telling.** `offer.received` fires when a pull records a new pending offer;
  the sender's `offer.decided` fires through the route's sink, which the
  daemon wires to its stdio notifier.

## Consequences

### Easier

- A declined book leaves nothing behind: no file, no rows, no library entry.
- The decision dialog needs one round trip and no downloads.
- The sender learns the outcome immediately instead of inferring it.

### Harder / to watch

- `inbound_offers` holds a copy of the held change rows as JSON. They are
  bounded by the offer, but a sender that never gets a decision leaves them
  until the row is decided.
- `pull_device` no longer has a single code path per page: rows are partitioned
  by the offer payload's ids, so an offer whose ids do not match its rows holds
  nothing (and the book lands unasked).
- A decision is reported best-effort. If the sender's daemon is down, the
  receiver keeps its answer and `delivered: false`; nothing retries.
- Offers from a sender that predates the payload fields arrive with
  `"Untitled"` and no authors, and are still held — the receiver asks with
  what it has.
