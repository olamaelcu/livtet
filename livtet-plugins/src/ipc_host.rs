//! Desktop IPC implementation of the plugin host traits.
//!
//! Every host function call is serialized and sent to the Tauri main
//! process over stdin/stdout MessagePack IPC. Responses are routed
//! back by id via the callback router.

use std::{
    io::{self, Write},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use super::host_trait::{
    GetEmbeddingResponse, HostBase, HostDatabase, HostEmbeddings, HostError, HostFiles, HostHttp,
    HostHttpResponse, HostLog, HostOAuth, HostSecrets, HostSettings, HostSystemSecrets,
    SimilarEdition, StoreEmbeddingResponse,
};
use crate::{
    error::{PluginError, PluginResult},
    protocol::{HostToMain, MainToHostCallback},
};

pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;
pub type CallbackRouter =
    Arc<Mutex<std::collections::HashMap<String, mpsc::Sender<MainToHostCallback>>>>;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(1200);
const HTTP_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Desktop IPC-backed host. Each method serializes a request to the
/// sidecar's stdout and blocks on the callback router for a response.
pub struct IpcHost {
    writer: SharedWriter,
    callback_router: CallbackRouter,
}

impl IpcHost {
    pub fn new(writer: SharedWriter, callback_router: CallbackRouter) -> Self {
        Self {
            writer,
            callback_router,
        }
    }

    fn write_message(&self, msg: &HostToMain) -> PluginResult<()> {
        let payload = rmp_serde::to_vec_named(msg)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let len = (payload.len() as u32).to_le_bytes();
        let mut guard = self
            .writer
            .lock()
            .map_err(|e| PluginError::MutexPoisoned(format!("writer: {e}")))?;
        guard.write_all(&len)?;
        guard.write_all(&payload)?;
        guard.flush()?;
        Ok(())
    }

    fn blocking_callback(&self, req: &HostToMain) -> PluginResult<MainToHostCallback> {
        let id = callback_request_id(req)
            .ok_or_else(|| {
                PluginError::Ipc("blocking_callback: not a request variant".to_string())
            })?
            .to_string();
        let (tx, rx) = mpsc::channel();
        {
            let mut map = self
                .callback_router
                .lock()
                .map_err(|e| PluginError::MutexPoisoned(format!("router: {e}")))?;
            map.insert(id.clone(), tx);
        }
        self.write_message(req)?;
        let response = match rx.recv_timeout(REQUEST_TIMEOUT) {
            Ok(cb) => cb,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut map) = self.callback_router.lock() {
                    map.remove(&id);
                }
                return Err(PluginError::Timeout(format!(
                    "request {id} timed out after {:?}",
                    REQUEST_TIMEOUT
                )));
            }
            Err(e) => {
                if let Ok(mut map) = self.callback_router.lock() {
                    map.remove(&id);
                }
                return Err(PluginError::Ipc(format!("receive callback: {e}")));
            }
        };
        Ok(response)
    }

    fn blocking_http_request(&self, req: &HostToMain) -> PluginResult<HostHttpResponse> {
        let id = match req {
            HostToMain::HttpRequest { id, .. } => id.clone(),
            other => {
                return Err(PluginError::Ipc(format!(
                    "blocking_http_request: not an HttpRequest: {other:?}"
                )));
            }
        };
        let (tx, rx) = mpsc::channel();
        {
            let mut map = self
                .callback_router
                .lock()
                .map_err(|e| PluginError::MutexPoisoned(format!("router: {e}")))?;
            map.insert(id.clone(), tx);
        }
        self.write_message(req)?;
        let response = match rx.recv_timeout(HTTP_REQUEST_TIMEOUT) {
            Ok(cb) => cb,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(mut map) = self.callback_router.lock() {
                    map.remove(&id);
                }
                return Err(PluginError::Timeout(format!(
                    "http request {id} timed out after {:?}",
                    HTTP_REQUEST_TIMEOUT
                )));
            }
            Err(e) => {
                if let Ok(mut map) = self.callback_router.lock() {
                    map.remove(&id);
                }
                return Err(PluginError::Ipc(format!("http callback: {e}")));
            }
        };
        if let MainToHostCallback::HttpResponse {
            id: _,
            status,
            body,
            headers,
        } = response
        {
            Ok(HostHttpResponse {
                status,
                body,
                headers,
            })
        } else {
            Err(PluginError::Ipc(format!(
                "unexpected callback for HttpRequest: {response:?}"
            )))
        }
    }
}

impl HostBase for IpcHost {}

impl HostHttp for IpcHost {
    fn http_get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        dispatch_http_impl(self, "GET", url, None, headers)
    }

    fn http_post(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        dispatch_http_impl(self, "POST", url, body, headers)
    }

    fn http_put(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        dispatch_http_impl(self, "PUT", url, body, headers)
    }

    fn http_patch(
        &self,
        url: &str,
        body: Option<&str>,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        dispatch_http_impl(self, "PATCH", url, body, headers)
    }

    fn http_delete(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HostHttpResponse, HostError> {
        dispatch_http_impl(self, "DELETE", url, None, headers)
    }
}

/// Build a `HostToMain::HttpRequest`, push it through the blocking
/// IPC channel, and map the error to `HostError::Http`. Shared by
/// every `http_*` method on `IpcHost`.
fn dispatch_http_impl(
    host: &IpcHost,
    method: &str,
    url: &str,
    body: Option<&str>,
    headers: &[(String, String)],
) -> Result<HostHttpResponse, HostError> {
    let req = HostToMain::HttpRequest {
        id: ulid::Ulid::new().to_string(),
        plugin_id: "ipc_host".to_string(),
        method: method.to_string(),
        url: url.to_string(),
        body: body.map(str::to_string),
        headers: headers.to_vec(),
    };
    host.blocking_http_request(&req)
        .map_err(|e| HostError::Http(e.to_string()))
}

impl HostLog for IpcHost {
    fn log(&self, plugin_id: &str, level: &str, message: &str) {
        let msg = HostToMain::Log {
            plugin_id: plugin_id.to_string(),
            level: level.to_string(),
            message: message.to_string(),
        };
        let _ = self.write_message(&msg);
    }
}

impl HostSecrets for IpcHost {
    fn get_secret(&self, plugin_id: &str, name: &str) -> Result<Option<String>, HostError> {
        let req = HostToMain::SecretRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            name: name.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::SecretResult { value, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(value)
            }
            _ => Err(HostError::Message(
                "unexpected callback for SecretRequest".to_string(),
            )),
        }
    }

    fn set_secret(&self, plugin_id: &str, name: &str, value: &str) -> Result<(), HostError> {
        let req = HostToMain::SetSecretRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            name: name.to_string(),
            value: value.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::SecretResult { error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(())
            }
            _ => Err(HostError::Message(
                "unexpected callback for SetSecretRequest".to_string(),
            )),
        }
    }
}

impl HostSystemSecrets for IpcHost {}

impl HostSettings for IpcHost {
    fn get_setting(&self, _plugin_id: &str, _key: &str) -> Option<String> {
        None
    }

    fn set_setting(&self, plugin_id: &str, key: &str, value: &str) -> Result<(), HostError> {
        let req = HostToMain::SetSettingRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            key: key.to_string(),
            value: value.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::SettingResult { ok, error, .. } => {
                if ok {
                    Ok(())
                } else {
                    Err(HostError::Message(error.unwrap_or_default()))
                }
            }
            _ => Err(HostError::Message(
                "unexpected callback for SetSettingRequest".to_string(),
            )),
        }
    }
}

impl HostDatabase for IpcHost {
    fn resolve_identifier(&self, urn: &str) -> Result<Option<String>, HostError> {
        let req = HostToMain::ResolveIdentifierRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            urn: urn.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::ResolveIdentifierResult {
                edition_id, error, ..
            } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(edition_id)
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn resolve_identifiers(&self, urns: &[String]) -> Result<Vec<Option<String>>, HostError> {
        let req = HostToMain::ResolveIdentifiersRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            urns: urns.to_vec(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::ResolveIdentifiersResult {
                edition_ids, error, ..
            } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(edition_ids)
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn get_edition_info(&self, edition_id: &str) -> Result<Option<serde_json::Value>, HostError> {
        let req = HostToMain::GetEditionInfoRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            edition_id: edition_id.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::EditionInfoResult { info, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(info)
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn get_edition_identifiers(&self, edition_id: &str) -> Result<Vec<String>, HostError> {
        let req = HostToMain::GetEditionIdentifiersRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            edition_id: edition_id.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::EditionIdentifiersResult { urns, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(urns)
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn fetch_progress(
        &self,
        urn: &str,
    ) -> Result<Option<crate::progress_entry::ProgressEntry>, HostError> {
        let req = HostToMain::FetchProgressRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            urn: urn.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::FetchProgressResult {
                progress, error, ..
            } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(progress)
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn upsert_progress(
        &self,
        urn: &str,
        progress: f64,
        last_location: Option<String>,
        total_reading_time_secs: i64,
    ) -> Result<serde_json::Value, HostError> {
        let req = HostToMain::UpsertProgressRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            urn: urn.to_string(),
            progress,
            last_location,
            total_reading_time_secs,
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::UpsertProgressResult {
                edition_id,
                format_id,
                ok,
                error,
                ..
            } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                let mut map = serde_json::Map::new();
                map.insert("ok".to_string(), serde_json::Value::Bool(ok));
                if let Some(eid) = edition_id {
                    map.insert("edition_id".to_string(), serde_json::Value::String(eid));
                }
                if let Some(fid) = format_id {
                    map.insert("format_id".to_string(), serde_json::Value::String(fid));
                }
                Ok(serde_json::Value::Object(map))
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }
}

impl HostEmbeddings for IpcHost {
    fn store_embedding(
        &self,
        edition_id: &str,
        model: &str,
        vector_bytes: &[u8],
    ) -> Result<StoreEmbeddingResponse, HostError> {
        let req = HostToMain::StoreEmbeddingRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            edition_id: edition_id.to_string(),
            model: model.to_string(),
            vector: vector_bytes.to_vec(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::StoreEmbeddingResult {
                row_id,
                dimensions,
                error,
                ..
            } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(StoreEmbeddingResponse {
                    row_id: row_id.unwrap_or_default(),
                    dimensions: dimensions.unwrap_or(0),
                })
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn get_embedding(
        &self,
        edition_id: &str,
        model: &str,
    ) -> Result<Option<GetEmbeddingResponse>, HostError> {
        let req = HostToMain::GetEmbeddingRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            edition_id: edition_id.to_string(),
            model: model.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::GetEmbeddingResult {
                vector,
                model,
                error,
                ..
            } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                match (vector, model) {
                    (Some(v), Some(m)) => Ok(Some(GetEmbeddingResponse {
                        vector: v,
                        model: m,
                    })),
                    (None, None) => Ok(None),
                    _ => Err(HostError::Message(
                        "inconsistent embedding response".to_string(),
                    )),
                }
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }

    fn find_similar_editions(
        &self,
        query_vector: &[u8],
        model: &str,
        limit: usize,
    ) -> Result<Vec<SimilarEdition>, HostError> {
        let req = HostToMain::FindSimilarEditionsRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: "ipc_host".to_string(),
            query_vector: query_vector.to_vec(),
            model: model.to_string(),
            limit,
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::FindSimilarEditionsResult { results, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                Ok(results
                    .into_iter()
                    .map(|(edition_id, score)| SimilarEdition { edition_id, score })
                    .collect())
            }
            _ => Err(HostError::Message("unexpected callback".to_string())),
        }
    }
}

impl HostFiles for IpcHost {
    fn read_file(&self, _path: &str) -> Result<Option<String>, HostError> {
        Err(HostError::Unsupported)
    }

    fn plugin_asset(&self, _plugin_dir: &str, _filename: &str) -> Result<Vec<u8>, HostError> {
        Err(HostError::Unsupported)
    }
}

/// OAuth redemption flow over IPC.
///
/// Each method sends a typed request to the Tauri main process and
/// blocks until the reply arrives (or the 20-minute
/// `REQUEST_TIMEOUT` elapses). The main process owns the PKCE
/// browser dance, the token storage (OS keychain), and the
/// transparent refresh — the host process only sees the access
/// token (or an error). Refresh tokens never cross the IPC
/// boundary.
impl HostOAuth for IpcHost {
    fn redeem_token(&self, plugin_id: &str, provider: &str) -> Result<String, HostError> {
        let req = HostToMain::OAuthRedeemRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            provider: provider.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::OAuthRedeemResult { token, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                token.ok_or_else(|| HostError::Message("oauth_redeem: empty response".to_string()))
            }
            other => Err(HostError::Message(format!(
                "oauth_redeem: unexpected callback: {other:?}"
            ))),
        }
    }

    fn get_valid_token(&self, plugin_id: &str, provider: &str) -> Result<String, HostError> {
        let req = HostToMain::OAuthTokenLookupRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            provider: provider.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::OAuthTokenResult { token, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                token.ok_or_else(|| {
                    HostError::Message("oauth_get_valid: empty response".to_string())
                })
            }
            other => Err(HostError::Message(format!(
                "oauth_get_valid: unexpected callback: {other:?}"
            ))),
        }
    }

    fn revoke_token(&self, plugin_id: &str, provider: &str) -> Result<(), HostError> {
        let req = HostToMain::OAuthRevokeRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            provider: provider.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::OAuthRevokeResult { ok, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                if ok {
                    Ok(())
                } else {
                    Err(HostError::Message(
                        "oauth_revoke: main process reported failure".to_string(),
                    ))
                }
            }
            other => Err(HostError::Message(format!(
                "oauth_revoke: unexpected callback: {other:?}"
            ))),
        }
    }

    fn authorize(&self, plugin_id: &str, provider: &str) -> Result<(), HostError> {
        let req = HostToMain::OAuthAuthorizeRequest {
            id: ulid::Ulid::new().to_string(),
            plugin_id: plugin_id.to_string(),
            provider: provider.to_string(),
        };
        let response = self
            .blocking_callback(&req)
            .map_err(|e| HostError::Message(e.to_string()))?;
        match response {
            MainToHostCallback::OAuthAuthorizeResult { ok, error, .. } => {
                if let Some(err) = error {
                    return Err(HostError::Message(err));
                }
                if ok {
                    Ok(())
                } else {
                    Err(HostError::Message(
                        "oauth_authorize: main process reported failure".to_string(),
                    ))
                }
            }
            other => Err(HostError::Message(format!(
                "oauth_authorize: unexpected callback: {other:?}"
            ))),
        }
    }
}

fn callback_request_id(req: &HostToMain) -> Option<&str> {
    match req {
        HostToMain::SecretRequest { id, .. }
        | HostToMain::SetSecretRequest { id, .. }
        | HostToMain::SetSettingRequest { id, .. }
        | HostToMain::HttpRequest { id, .. }
        | HostToMain::ReadFileRequest { id, .. }
        | HostToMain::SqliteQueryRequest { id, .. }
        | HostToMain::ReadAssetRequest { id, .. }
        | HostToMain::ResolveIdentifierRequest { id, .. }
        | HostToMain::ResolveIdentifiersRequest { id, .. }
        | HostToMain::GetEditionInfoRequest { id, .. }
        | HostToMain::GetEditionIdentifiersRequest { id, .. }
        | HostToMain::FetchProgressRequest { id, .. }
        | HostToMain::UpsertProgressRequest { id, .. }
        | HostToMain::StoreEmbeddingRequest { id, .. }
        | HostToMain::GetEmbeddingRequest { id, .. }
        | HostToMain::FindSimilarEditionsRequest { id, .. }
        | HostToMain::OAuthRedeemRequest { id, .. }
        | HostToMain::OAuthTokenLookupRequest { id, .. }
        | HostToMain::OAuthRevokeRequest { id, .. }
        | HostToMain::OAuthAuthorizeRequest { id, .. } => Some(id.as_str()),
        _ => None,
    }
}
