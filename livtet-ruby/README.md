# livtet (Ruby)

Minimal Ruby wrapper around `livtet-core`. **V1 exposes only:**

```ruby
require "livtet"

Livtet.paths
# => { bundle: "net.olamaelcu.livtet", data: "...", config: "...", logs: "..." }

db = Livtet::Database.open           # default data dir (<data>/livtet.db)
db = Livtet::Database.open("/tmp/x.db")
db.path   # => "/tmp/x.db"
db.close

Livtet::Database.open("/tmp/x.db") do |db|
  # auto-closed (even on exception)
end
```

No query, data, or JSON APIs in v1. One database may be open at a
time per process; opening twice, or using a handle after `close`,
raises `Livtet::Error` (a `StandardError`).

## Build & test

Requires a Rust toolchain (workspace builds with Cargo):

```
rake          # compile + test
rake compile  # build native ext into lib/livtet/livtet.so
rake test     # minitest suite (needs `rake compile` first)
```

The native extension is plain `cargo build -p livtet-ruby` output
copied to `lib/livtet/livtet.so` — no `rb_sys`/`rake-compiler`
dependency. `Livtet::Database.open` runs migrations on open and
`PRAGMA optimize` + pool close on close.
