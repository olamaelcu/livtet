use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};

/// Messages sent from the main process to the plugin host.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MainToHost {
    LoadPlugin {
        plugin_id: String,
        manifest: serde_json::Value,
        source: String,
        /// Per-plugin data directory used to resolve relative paths
        /// passed to `host.read_file` / `host.sqlite_query` and to
        /// serve `host.plugin_dir` / `host.plugin_asset` calls. May
        /// be `None` for legacy single-file plugins loaded from
        /// stdin or when the caller has no notion of a per-plugin
        /// data directory (e.g. fixture tests that only exercise
        /// in-host functions).
        #[serde(default)]
        data_dir: Option<Utf8PathBuf>,
        /// Pre-loaded settings from the `plugin_settings` DB table.
        /// The main process queries the DB before sending the
        /// LoadPlugin message so the host process can populate its
        /// in-memory map without a second round-trip. `None` if no
        /// DB is available or no settings exist for the plugin yet.
        #[serde(default)]
        settings: Option<std::collections::HashMap<String, String>>,
        /// LuaRocks rock names this plugin depends on (mirrors
        /// `PluginMeta.rocks`). The host process uses this to know
        /// which rocks it should expect to resolve via
        /// `host.require` once the parent has installed them via
        /// `luarocks` and exported `LUA_PATH` / `LUA_CPATH` to the
        /// child. The field is declared LAST on the variant
        /// because rmp_serde encodes the enum as a positional
        /// array — adding a field in the middle would shift the
        /// wire positions of every subsequent field. `#[serde(default)]`
        /// means older senders that omit it deserialize cleanly.
        #[serde(default)]
        rocks: Vec<String>,
    },
    UnloadPlugin {
        plugin_id: String,
    },
    Call {
        id: String,
        plugin_id: String,
        capability: String,
        args: Vec<serde_json::Value>,
    },
    Shutdown,
}

/// Messages sent from the plugin host back to the main process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostToMain {
    /// Result of a plugin `Call`. Wire shape is flattened to
    /// `{ id, ok, value, error }` instead of a nested
    /// `Result<_, _>` for the same reason documented on
    /// [`MainToHostCallback`]: rmp_serde encodes
    /// `#[serde(tag = "type", ...)]` enums as positional arrays,
    /// so `Result` (which serde itself represents as an
    /// internally-tagged enum) would write three slots — variant
    /// tag, discriminant string, payload — and the receiving
    /// `CallResult` would see only the first two, with the
    /// payload showing up as the next field's slot. The flattened
    /// shape matches the existing `FetchProgressResult` /
    /// `UpsertProgressResult` callback style: `ok = true` and
    /// `value = Some(_)` on success, `ok = false` and `error =
    /// Some(_)` on failure. `value` and `error` are always
    /// present (as `null`) so the array length stays constant
    /// for the positional decoder.
    CallResult {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        value: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    HttpRequest {
        id: String,
        plugin_id: String,
        method: String,
        url: String,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    Log {
        plugin_id: String,
        level: String,
        message: String,
    },
    PluginLoaded {
        plugin_id: String,
        load_state: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        missing_optional: Vec<String>,
    },
    PluginUnloaded {
        plugin_id: String,
    },
    Ready {
        runtime: String,
    },
    PluginLoadError {
        plugin_id: String,
        error: String,
    },
    /// Request a secret from the OS keychain, namespaced to the
    /// plugin. The main process looks the value up via the
    /// `keyring` crate and replies with [`MainToHostCallback::SecretResult`].
    SecretRequest {
        id: String,
        plugin_id: String,
        name: String,
    },
    /// Request to write a secret. The main process enforces the
    /// "{plugin_id}:{name}" keyspace and replies with
    /// [`MainToHostCallback::SecretResult`]. The `value` field is
    /// `None`; the variant is reused for both reads and writes
    /// because the result shape is the same: ok or error.
    SetSecretRequest {
        id: String,
        plugin_id: String,
        name: String,
        value: String,
    },
    /// Write a single setting key/value to the `plugin_settings`
    /// DB table. Reply via [`MainToHostCallback::SettingResult`].
    /// The host fires this from `host.set_setting(key, value)`.
    /// Reads go through the in-memory settings map populated at
    /// LoadPlugin time and do not need an IPC round-trip.
    SetSettingRequest {
        id: String,
        plugin_id: String,
        key: String,
        value: String,
    },
    /// Read a file under the plugin's data directory. The host
    /// enforces the path policy (no `..` traversal, relative paths
    /// resolve under `data_dir`, absolute paths must be inside
    /// `data_dir`).
    ReadFileRequest {
        id: String,
        plugin_id: String,
        path: String,
    },
    /// Run a read-only SELECT against a SQLite file. The host
    /// enforces: non-SELECT → error, 10s busy timeout, 10k row
    /// cap, read-only connection.
    ///
    /// `params` is intentionally NOT `skip_serializing_if`: it
    /// sits in the middle of the field list, and rmp_serde
    /// encodes `#[serde(tag = "type", ...)]` enums as a
    /// positional array — if `params` is elided, the next field
    /// (`limit`) lands in the `params` slot on the receiving
    /// side and the decoder errors with "expected a sequence".
    /// `limit` is at the end so it can still be elided.
    SqliteQueryRequest {
        id: String,
        plugin_id: String,
        path: String,
        sql: String,
        params: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<i64>,
    },
    /// Fire-and-forget event emission. The main process forwards
    /// the payload to the Tauri `app.emit_all("plugin:event", ...)`
    /// channel. Unknown event types are logged and dropped on the
    /// main-process side; the host does not wait for a response.
    EmitEvent {
        plugin_id: String,
        event_type: String,
        payload: serde_json::Value,
    },
    /// Read a file from the plugin's `assets/` directory. The host
    /// reads in-process (the data_dir is on the host's filesystem)
    /// and replies via [`MainToHostCallback::AssetResult`].
    ReadAssetRequest {
        id: String,
        plugin_id: String,
        filename: String,
    },
    /// Look up the first edition whose identifiers or work
    /// identifiers match the given URN. Reply via
    /// [`MainToHostCallback::ResolveIdentifierResult`].
    ResolveIdentifierRequest {
        id: String,
        plugin_id: String,
        urn: String,
    },
    /// Batch version of [`Self::ResolveIdentifierRequest`].
    /// Reply via [`MainToHostCallback::ResolveIdentifiersResult`].
    ResolveIdentifiersRequest {
        id: String,
        plugin_id: String,
        urns: Vec<String>,
    },
    /// Read a single edition's metadata by id. Reply via
    /// [`MainToHostCallback::EditionInfoResult`].
    GetEditionInfoRequest {
        id: String,
        plugin_id: String,
        edition_id: String,
    },
    /// List every URN linked to the given edition. Reply via
    /// [`MainToHostCallback::EditionIdentifiersResult`].
    GetEditionIdentifiersRequest {
        id: String,
        plugin_id: String,
        edition_id: String,
    },
    /// Look up the reading-progress row for an edition
    /// (identified by URN) on the local DB. The dispatcher
    /// resolves the URN to an `edition_id` via
    /// `find_edition_id_by_urn`, picks the first format for
    /// that edition via `find_default_format_for_edition`, and
    /// reads the `reading_progress` row. Reply via
    /// [`MainToHostCallback::FetchProgressResult`].
    ///
    /// Tracks Commit 4 of the plugin roadmap
    /// (`docs/plans/2026-06-07-plugin-roadmap.md`).
    FetchProgressRequest {
        id: String,
        plugin_id: String,
        urn: String,
    },
    /// Write/overwrite the reading-progress row for an edition
    /// (identified by URN) on the local DB and emit a
    /// `"reading-progress-updated"` Tauri event so the desktop
    /// UI can react. Reply via
    /// [`MainToHostCallback::UpsertProgressResult`].
    ///
    /// `progress` is 0.0..=1.0; `last_location` is the Readium
    /// `Locator.toString()` blob or `None`; `total_secs` is the
    /// cumulative reading-time in seconds. `format_id` is NOT
    /// part of the payload yet — for now the dispatcher reads
    /// the edition's canonical `editions.format_id` (m0002)
    /// via `find_default_format_for_edition` (a follow-up will
    /// let the plugin surface a `format_id` field). If the
    /// edition has no format binding the request fails fast
    /// with a clear error rather than silently dropping the
    /// write.
    UpsertProgressRequest {
        id: String,
        plugin_id: String,
        urn: String,
        progress: f64,
        // No `skip_serializing_if = "Option::is_none"`: rmp_serde
        // encodes the `HostToMain` enum as a positional array and
        // eliding a field shifts every subsequent field into the
        // wrong slot on the receiving end. Keeping `None` on the
        // wire (as `serde_json::Value::Null` → msgpack `nil`) keeps
        // the array length constant and the positional decoder
        // honest. See the comment on `MainToHostCallback` for the
        // same rule on the reply path.
        last_location: Option<String>,
        total_reading_time_secs: i64,
    },
    /// Store an embedding vector for an edition. The main process
    /// upserts into `edition_embeddings` and replies with
    /// [`MainToHostCallback::StoreEmbeddingResult`].
    StoreEmbeddingRequest {
        id: String,
        plugin_id: String,
        edition_id: String,
        model: String,
        vector: Vec<u8>,
    },
    /// Retrieve a stored embedding vector for an edition. The main
    /// process reads from `edition_embeddings` and replies with
    /// [`MainToHostCallback::GetEmbeddingResult`].
    GetEmbeddingRequest {
        id: String,
        plugin_id: String,
        edition_id: String,
        model: String,
    },
    /// Find editions with embedding vectors similar to the query
    /// vector using cosine similarity. The main process reads all
    /// matching embeddings from `edition_embeddings`, scores them,
    /// and replies with
    /// [`MainToHostCallback::FindSimilarEditionsResult`].
    FindSimilarEditionsRequest {
        id: String,
        plugin_id: String,
        query_vector: Vec<u8>,
        model: String,
        limit: usize,
    },
    /// Run the OAuth redemption flow for a provider and return a
    /// fresh access token. The main process opens the system
    /// browser, handles the PKCE redirect, exchanges the code at
    /// the provider's token endpoint, and stores the grant +
    /// refresh token in its secure storage (OS keychain on
    /// desktop). Replies via [`MainToHostCallback::OAuthRedeemResult`].
    ///
    /// `provider` is the host's opaque provider ID (e.g.
    /// `livtet_cloud`). Scopes are taken from the plugin manifest
    /// at load time — the host combines the manifest-declared
    /// scopes with any already-granted scopes and prompts the
    /// user for the union.
    OAuthRedeemRequest {
        id: String,
        plugin_id: String,
        provider: String,
    },
    /// Return a currently valid access token for a provider,
    /// refreshing transparently if the stored token is within
    /// 60s of expiry. If no grant exists, the main process runs
    /// the full flow (equivalent to `OAuthRedeemRequest`).
    /// Replies via [`MainToHostCallback::OAuthTokenResult`].
    OAuthTokenLookupRequest {
        id: String,
        plugin_id: String,
        provider: String,
    },
    /// Delete the stored grant + refresh token and clear any
    /// cached access token. Idempotent — replies with `Ok` even
    /// if no grant existed. Replies via
    /// [`MainToHostCallback::OAuthRevokeResult`].
    OAuthRevokeRequest {
        id: String,
        plugin_id: String,
        provider: String,
    },
    /// Fire-and-forget OAuth authorization. The main process opens
    /// the system browser and registers the pending consent, but
    /// returns immediately without waiting for the user to complete
    /// the flow. The plugin subsequently calls
    /// `host.oauth_redeem_token(provider)` to obtain the access
    /// token (which will hit the cached grant if the user has
    /// authorised in the meantime, otherwise start a fresh PKCE
    /// flow).
    ///
    /// Replies via [`MainToHostCallback::OAuthAuthorizeResult`]
    /// with `ok: true` if the initiation succeeded (browser
    /// launched, pending consent registered).
    OAuthAuthorizeRequest {
        id: String,
        plugin_id: String,
        provider: String,
    },
}

/// Callback responses from the main process back to the host.
///
/// Inline `skip_serializing_if = "..."` is intentionally NOT
/// used on `Option` / `Vec` fields here: rmp_serde encodes
/// `#[serde(tag = "type", ...)]` enums as a positional array
/// (not a tagged struct), and the host's `MainMessage`
/// counterpart decodes by position. If a field is `None`, the
/// array shortens and the host's positional decoder reads the
/// next field into the wrong slot — e.g. an `error: Some(_)`
/// becomes `content: Some(_)` because `content: None` was
/// elided from the wire. Keeping every `Option` / `Vec` field
/// on the wire (as `None` / `[]` for empty) keeps the array
/// length constant and the positional decoder honest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MainToHostCallback {
    HttpResponse {
        id: String,
        status: u16,
        #[serde(default)]
        body: Option<String>,
        #[serde(default)]
        headers: Vec<(String, String)>,
    },
    /// Response to both `SecretRequest` and `SetSecretRequest`. The
    /// `value` field is `Some` for reads (with the resolved value
    /// or `None` if the key was missing) and `None` for writes.
    /// `error` is `Some` on failure for either direction.
    SecretResult {
        id: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::SetSettingRequest`]. `ok = true`
    /// means the setting was upserted in `plugin_settings`;
    /// `error = Some(_)` means the write failed (no DB pool, SQL
    /// error, etc.). Mirrors the flattened `CallResult` shape
    /// used elsewhere — no nested `Result` because rmp_serde
    /// encodes `#[serde(tag = "type", ...)]` enums as positional
    /// arrays.
    SettingResult {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    ReadFileResult {
        id: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    SqliteResult {
        id: String,
        #[serde(default)]
        columns: Vec<String>,
        #[serde(default)]
        rows: Vec<Vec<serde_json::Value>>,
        #[serde(default)]
        error: Option<String>,
    },
    AssetResult {
        id: String,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::ResolveIdentifierRequest`].
    ResolveIdentifierResult {
        id: String,
        #[serde(default)]
        edition_id: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::ResolveIdentifiersRequest`].
    /// One [`Option<String>`] per URN in the request, in order.
    ResolveIdentifiersResult {
        id: String,
        #[serde(default)]
        edition_ids: Vec<Option<String>>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::GetEditionInfoRequest`].
    EditionInfoResult {
        id: String,
        #[serde(default)]
        info: Option<serde_json::Value>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::GetEditionIdentifiersRequest`].
    EditionIdentifiersResult {
        id: String,
        #[serde(default)]
        urns: Vec<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::FetchProgressRequest`].
    /// `progress` is `Some` when the dispatcher found a row in
    /// the local DB; `None` means "no row exists yet for this
    /// (edition, default format) pair" — not an error. Errors
    /// land in `error` (e.g. URN unresolved, DB pool missing).
    FetchProgressResult {
        id: String,
        #[serde(default)]
        progress: Option<crate::progress_entry::ProgressEntry>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::UpsertProgressRequest`].
    /// `ok = true` means the row was written (or updated) and the
    /// `"reading-progress-updated"` Tauri event was emitted.
    /// `edition_id` and `format_id` echo back the resolved
    /// identifiers so the caller can correlate the round-trip.
    /// `error` is `Some` on failure (e.g. URN unresolved, no
    /// format row for the edition, DB error).
    UpsertProgressResult {
        id: String,
        #[serde(default)]
        edition_id: Option<String>,
        #[serde(default)]
        format_id: Option<String>,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::StoreEmbeddingRequest`].
    StoreEmbeddingResult {
        id: String,
        #[serde(default)]
        row_id: Option<String>,
        #[serde(default)]
        dimensions: Option<usize>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::GetEmbeddingRequest`].
    GetEmbeddingResult {
        id: String,
        #[serde(default)]
        vector: Option<Vec<u8>>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::FindSimilarEditionsRequest`].
    FindSimilarEditionsResult {
        id: String,
        #[serde(default)]
        results: Vec<(String, f32)>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::OAuthRedeemRequest`]. `token` is
    /// `Some` on success; `error` is `Some` on failure (user
    /// denied, network error, PKCE verification failed, etc.).
    OAuthRedeemResult {
        id: String,
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::OAuthTokenLookupRequest`]. Same
    /// shape as `OAuthRedeemResult` — `token` may be `None` only
    /// if `error` is `Some`.
    OAuthTokenResult {
        id: String,
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::OAuthRevokeRequest`]. `ok = true`
    /// means the stored grant (if any) was deleted and the
    /// provider's revocation endpoint was called. `error` is
    /// `Some` on transport-level failure; local deletion succeeds
    /// regardless.
    OAuthRevokeResult {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    /// Response to [`HostToMain::OAuthAuthorizeRequest`].
    /// `ok = true` means the browser was opened and a pending
    /// consent entry was registered. The user still needs to
    /// complete the PKCE flow; the plugin calls
    /// `host.oauth_redeem_token(provider)` to retrieve the
    /// resulting access token. `error` is `Some` if the
    /// provider was unknown or the browser couldn't be opened.
    OAuthAuthorizeResult {
        id: String,
        #[serde(default)]
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
}
