//! Native-only ChatGPT OAuth. SQLite contains metadata; the OS credential service
//! contains one indivisible token set per issued client registration. No fallback.
mod oauth;
mod stream;
#[cfg(test)]
mod tests;

use crate::storage::Storage;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
const CANCELED: &str = "Request canceled";
const KEYRING_ERROR: &str =
    "OS credential service unavailable. Unlock or enable your system keyring and try again.";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AIAccount {
    id: String,
    label: String,
    connected: bool,
    can_use_plan: bool,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AIModel {
    id: String,
    label: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AIStatus {
    accounts: Vec<AIAccount>,
    active_account_id: Option<String>,
    selected_model: Option<String>,
    models: Vec<AIModel>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AIRequest {
    request_id: String,
    account_id: String,
    model: String,
    instructions: String,
    messages: Vec<Message>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    role: Role,
    content: String,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
    Assistant,
}
#[derive(Serialize)]
pub struct AIDelta {
    text: String,
}
#[derive(Serialize)]
pub struct Answer {
    text: String,
}
#[derive(Serialize)]
pub struct Disconnect {
    status: AIStatus,
    warning: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    id: String,
    subject: String,
    label: String,
    models: Vec<AIModel>,
    selected: Option<String>,
    // A non-secret tombstone prevents a failed keyring deletion from reconnecting
    // the session after restart. Only validated login clears it.
    #[serde(default)]
    signed_out: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    host: String,
    registrations: Vec<Registration>,
    active: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Credentials {
    client: String,
    subject: String,
    access: String,
    refresh: String,
    id_token: String,
    scopes: Vec<String>,
    expires: u64,
}
impl Credentials {
    fn can_use_plan(&self) -> bool {
        self.scopes.iter().any(|s| s == oauth::DIRECT)
    }
}

trait Vault: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<Credentials>, String>;
    fn set(&self, id: &str, credentials: &Credentials) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}
struct OsVault;
impl OsVault {
    fn entry(id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new("com.gloss.desktop.chatgpt", id).map_err(|_| KEYRING_ERROR.into())
    }
}
impl Vault for OsVault {
    fn get(&self, id: &str) -> Result<Option<Credentials>, String> {
        match Self::entry(id)?.get_password() {
            Ok(value) => serde_json::from_str(&value)
                .map(Some)
                .map_err(|_| "Stored ChatGPT credential record is invalid".into()),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(KEYRING_ERROR.into()),
        }
    }
    fn set(&self, id: &str, credentials: &Credentials) -> Result<(), String> {
        // A single keyring item replacement keeps rotating tokens together.
        let value = serde_json::to_string(credentials).map_err(|_| "Credential encoding failed")?;
        Self::entry(id)?
            .set_password(&value)
            .map_err(|_| KEYRING_ERROR.into())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        match Self::entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(KEYRING_ERROR.into()),
        }
    }
}

struct Inner {
    metadata: Metadata,
    credentials: HashMap<String, Credentials>,
    login: Option<(String, CancellationToken)>,
    requests: HashMap<String, CancellationToken>,
    early_cancels: HashMap<String, std::time::Instant>,
    epoch: u64,
}
impl Inner {
    fn status(&self) -> AIStatus {
        let current = self
            .metadata
            .registrations
            .iter()
            .find(|r| Some(&r.id) == self.metadata.active.as_ref());
        AIStatus {
            accounts: self
                .metadata
                .registrations
                .iter()
                .map(|r| AIAccount {
                    id: r.id.clone(),
                    label: r.label.clone(),
                    connected: !r.signed_out && self.credentials.contains_key(&r.id),
                    can_use_plan: !r.signed_out
                        && self
                            .credentials
                            .get(&r.id)
                            .is_some_and(Credentials::can_use_plan),
                })
                .collect(),
            active_account_id: self.metadata.active.clone(),
            selected_model: current.and_then(|r| r.selected.clone()),
            models: current.map(|r| r.models.clone()).unwrap_or_default(),
        }
    }
    fn cancel_requests(&mut self) {
        for token in self.requests.values() {
            token.cancel();
        }
    }
    fn invalidate(&mut self) {
        self.epoch += 1;
        self.cancel_requests();
        if let Some((_, token)) = &self.login {
            token.cancel();
        }
    }
    fn save(&self, storage: &Storage) -> Result<(), String> {
        storage.save_state(
            "gloss:ai",
            &serde_json::to_string(&self.metadata)
                .map_err(|_| "Connection metadata encoding failed")?,
        )
    }
}

#[derive(Clone)]
pub struct Ai {
    inner: Arc<Mutex<Inner>>,
    // Serializes refresh and all credential writes/deletes; never held by status/cancel.
    gate: Arc<tokio::sync::Mutex<()>>,
    vault: Arc<dyn Vault>,
    http: reqwest::Client,
    #[cfg(test)]
    endpoints: HashMap<&'static str, String>,
}
impl Ai {
    pub fn new(storage: &Storage) -> Result<Self, String> {
        Self::with_vault(storage, Arc::new(OsVault))
    }
    fn with_vault(storage: &Storage, vault: Arc<dyn Vault>) -> Result<Self, String> {
        let metadata = match storage.load_state("gloss:ai")? {
            Some(value) => {
                serde_json::from_str(&value).map_err(|_| "Invalid ChatGPT connection metadata")?
            }
            None => Metadata {
                host: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
                registrations: vec![],
                active: None,
            },
        };
        let mut credentials = HashMap::new();
        for r in &metadata.registrations {
            // No credential service is a disconnected account, not a startup failure.
            if let Ok(Some(c)) = if r.signed_out {
                Ok(None)
            } else {
                vault.get(&r.id)
            } {
                if c.client == r.id && c.subject == r.subject {
                    credentials.insert(r.id.clone(), c);
                }
            }
        }
        let inner = Inner {
            metadata,
            credentials,
            login: None,
            requests: HashMap::new(),
            early_cancels: HashMap::new(),
            epoch: 0,
        };
        inner.save(storage)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(inner)),
            gate: Arc::new(tokio::sync::Mutex::new(())),
            vault,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(300))
                .build()
                .map_err(|_| "Cannot initialize native HTTP")?,
            #[cfg(test)]
            endpoints: HashMap::new(),
        })
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn endpoint(&self, name: &str) -> &str {
        #[cfg(test)]
        if let Some(endpoint) = self.endpoints.get(name) {
            return endpoint;
        }
        match name {
            "token" => oauth::TOKEN,
            "models" => "https://api.openai.com/v1/models",
            "responses" => "https://api.openai.com/v1/responses",
            _ => unreachable!(),
        }
    }
    async fn revocation_url(&self) -> Result<reqwest::Url, String> {
        #[cfg(test)]
        if let Some(url) = self.endpoints.get("revoke") {
            return url.parse().map_err(|_| "Invalid test URL".into());
        }
        let doc = oauth::discovery(&self.http).await?;
        oauth::official(
            doc.revocation_endpoint
                .as_deref()
                .ok_or("Missing revocation endpoint")?,
        )
    }

    async fn token(&self, id: &str, epoch: u64) -> Result<String, String> {
        // A caller cancel must not discard a refresh response after the provider
        // rotates its token. This bounded task owns renewal through the keyring
        // commit; inference HTTP itself remains immediately cancelable.
        let ai = self.clone();
        let id = id.to_owned();
        tokio::spawn(async move { ai.renew(&id, epoch).await })
            .await
            .map_err(|_| "Session renewal interrupted".to_string())?
    }
    async fn renew(&self, id: &str, epoch: u64) -> Result<String, String> {
        let _gate = self.gate.lock().await;
        let old = {
            let inner = self.lock();
            if inner.epoch != epoch {
                return Err(CANCELED.into());
            }
            if inner
                .metadata
                .registrations
                .iter()
                .any(|r| r.id == id && r.signed_out)
            {
                return Err("ChatGPT account is disconnected".into());
            }
            inner
                .credentials
                .get(id)
                .cloned()
                .ok_or("ChatGPT account is disconnected")?
        };
        if !old.can_use_plan() {
            return Err(
                "ChatGPT plan use was not authorized. Reconnect and grant plan sharing permission."
                    .into(),
            );
        }
        if old.expires > now() + 60 {
            return Ok(old.access);
        }
        let response = self
            .http
            .post(self.endpoint("token"))
            .timeout(Duration::from_secs(30))
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", id),
                ("refresh_token", &old.refresh),
                ("resource", oauth::RESOURCE),
            ])
            .send()
            .await
            .map_err(|_| "ChatGPT session renewal unavailable")?;
        let tokens = token_response(response).await?;
        if let Some(id_token) = &tokens.id_token {
            oauth::verify(&self.http, id_token, id, None, Some(&old.subject)).await?;
        }
        let updated = tokens.credentials(id, &old.subject, Some(&old))?;
        let mut inner = self.lock();
        // Switching models/accounts can invalidate the caller, not the rotating
        // credentials. A disconnect tombstone keeps the session unavailable while
        // allowing its in-flight rotation to finish before revoking the latest token.
        if !inner
            .credentials
            .get(id)
            .is_some_and(|c| c.refresh == old.refresh && c.subject == old.subject)
        {
            return Err(CANCELED.into());
        }
        self.vault.set(id, &updated)?;
        let access = updated.access.clone();
        let allowed = updated.can_use_plan();
        inner.credentials.insert(id.into(), updated);
        if inner.epoch != epoch {
            return Err(CANCELED.into());
        }
        if !allowed {
            return Err(
                "ChatGPT plan sharing permission is no longer granted. Reconnect your account."
                    .into(),
            );
        }
        Ok(access)
    }

    async fn login(&self, storage: &Storage, account: Option<String>) -> Result<AIStatus, String> {
        let cancel = CancellationToken::new();
        let attempt = uuid::Uuid::new_v4().to_string();
        let (host, registration, hint) = {
            let mut inner = self.lock();
            let registration = account
                .as_ref()
                .map(|id| {
                    inner
                        .metadata
                        .registrations
                        .iter()
                        .find(|r| &r.id == id)
                        .cloned()
                        .ok_or("Unknown ChatGPT registration")
                })
                .transpose()?;
            if let Some((_, previous)) = inner.login.take() {
                previous.cancel();
            }
            let hint = account
                .as_ref()
                .filter(|_| !registration.as_ref().is_some_and(|r| r.signed_out))
                .and_then(|id| inner.credentials.get(id))
                .map(|c| c.id_token.clone());
            inner.login = Some((attempt.clone(), cancel.clone()));
            (inner.metadata.host.clone(), registration, hint)
        };
        let result = tokio::select! {
            biased;
            _ = cancel.cancelled() => Err("Sign in canceled".into()),
            result = tokio::time::timeout(Duration::from_secs(300), self.login_attempt(storage, registration, host, hint, &cancel)) => result.unwrap_or_else(|_| Err("Sign in timed out".into())),
        };
        let mut inner = self.lock();
        if inner.login.as_ref().is_some_and(|(id, _)| id == &attempt) {
            inner.login = None;
        }
        result
    }

    async fn login_attempt(
        &self,
        storage: &Storage,
        registration: Option<Registration>,
        host: String,
        hint: Option<String>,
        cancel: &CancellationToken,
    ) -> Result<AIStatus, String> {
        let mut client = registration
            .as_ref()
            .map(|r| r.id.as_str())
            .unwrap_or(oauth::DYNAMIC)
            .to_owned();
        let (issued, nonce, tokens) = loop {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|_| "Cannot bind sign-in callback")?;
            let redirect = format!(
                "http://127.0.0.1:{}/auth/callback",
                listener
                    .local_addr()
                    .map_err(|_| "Callback listener unavailable")?
                    .port()
            );
            let state = oauth::random();
            let nonce = oauth::random();
            let verifier = oauth::random();
            let url = oauth::authorize(
                &client,
                &host,
                &redirect,
                &state,
                &nonce,
                &verifier,
                hint.as_deref(),
            );
            if cancel.is_cancelled() {
                return Err("Sign in canceled".into());
            }
            // This API launches the OS browser; never sends the URL to the webview.
            oauth::open_browser(&url).await?;
            let (code, issued) = oauth::listen(listener, &state, &client, cancel).await?;
            let response = self
                .http
                .post(oauth::TOKEN)
                .form(&[
                    ("grant_type", "authorization_code"),
                    ("client_id", &issued),
                    ("code", &code),
                    ("code_verifier", &verifier),
                    ("redirect_uri", &redirect),
                    ("resource", oauth::RESOURCE),
                ])
                .send()
                .await
                .map_err(|_| "ChatGPT token exchange unavailable")?;
            if !response.status().is_success() {
                let error: serde_json::Value = response.json().await.unwrap_or_default();
                if error["error"].as_str() == Some("invalid_grant") {
                    // The code is single-use; restart with fresh bindings, but keep
                    // the just-issued registration rather than registering again.
                    client = issued;
                    continue;
                }
                return Err("ChatGPT session exchange failed. Please sign in again.".into());
            }
            let tokens = token_response(response).await?;
            break (issued, nonce, tokens);
        };
        let claims = oauth::verify(
            &self.http,
            tokens.id_token.as_deref().ok_or("Missing ID token")?,
            &issued,
            Some(&nonce),
            registration.as_ref().map(|r| r.subject.as_str()),
        )
        .await?;
        let credentials = tokens.credentials(&issued, &claims.sub, None)?;
        let _gate = self.gate.lock().await;
        let mut inner = self.lock();
        if cancel.is_cancelled() {
            return Err("Sign in canceled".into());
        }
        if let Some(r) = inner.metadata.registrations.iter().find(|r| r.id == issued) {
            if r.subject != claims.sub {
                return Err("ChatGPT registration identity mismatch".into());
            }
        }
        self.vault.set(&issued, &credentials)?;
        if !inner.metadata.registrations.iter().any(|r| r.id == issued) {
            let label = format!(
                "{} · {}",
                claims.email.unwrap_or_else(|| "ChatGPT account".into()),
                &issued
            );
            inner.metadata.registrations.push(Registration {
                id: issued.clone(),
                subject: claims.sub,
                label,
                models: vec![],
                selected: None,
                signed_out: false,
            });
        }
        inner
            .metadata
            .registrations
            .iter_mut()
            .find(|r| r.id == issued)
            .unwrap()
            .signed_out = false;
        inner.cancel_requests();
        inner.epoch += 1;
        inner.credentials.insert(issued.clone(), credentials);
        inner.metadata.active = Some(issued);
        inner.save(storage)?;
        Ok(inner.status())
    }
}

#[derive(Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    token_type: String,
    expires_in: u64,
    scope: Option<String>,
}
impl Tokens {
    fn credentials(
        self,
        client: &str,
        subject: &str,
        old: Option<&Credentials>,
    ) -> Result<Credentials, String> {
        if !self.token_type.eq_ignore_ascii_case("bearer") || self.access_token.is_empty() {
            return Err("Invalid ChatGPT token response".into());
        }
        let scopes = match self.scope {
            Some(scope) => scope.split_whitespace().map(str::to_owned).collect(),
            None => old.map(|c| c.scopes.clone()).unwrap_or_default(),
        };
        Ok(Credentials {
            client: client.into(),
            subject: subject.into(),
            access: self.access_token,
            refresh: self
                .refresh_token
                .or_else(|| old.map(|c| c.refresh.clone()))
                .filter(|s| !s.is_empty())
                .ok_or("Missing renewable ChatGPT session")?,
            id_token: self
                .id_token
                .or_else(|| old.map(|c| c.id_token.clone()))
                .ok_or("Missing ID token")?,
            scopes,
            expires: now().saturating_add(self.expires_in),
        })
    }
}
async fn token_response(response: reqwest::Response) -> Result<Tokens, String> {
    if !response.status().is_success() {
        return Err("ChatGPT session exchange failed. Please sign in again.".into());
    }
    response
        .json()
        .await
        .map_err(|_| "Invalid ChatGPT token response".into())
}

#[tauri::command]
pub fn ai_status(ai: State<'_, Ai>) -> AIStatus {
    ai.lock().status()
}
#[tauri::command]
pub async fn ai_login(
    ai: State<'_, Ai>,
    storage: State<'_, Storage>,
    account_id: Option<String>,
) -> Result<AIStatus, String> {
    ai.login(&storage, account_id).await
}
#[tauri::command]
pub fn ai_cancel_login(ai: State<'_, Ai>) {
    if let Some((_, token)) = &ai.lock().login {
        token.cancel();
    }
}

#[tauri::command]
pub async fn ai_models(ai: State<'_, Ai>, storage: State<'_, Storage>) -> Result<AIStatus, String> {
    let (id, epoch) = {
        let inner = ai.lock();
        (
            inner
                .metadata
                .active
                .clone()
                .ok_or("Select a ChatGPT account")?,
            inner.epoch,
        )
    };
    let access = ai.token(&id, epoch).await?;
    let response = ai
        .http
        .get(ai.endpoint("models"))
        .bearer_auth(access)
        .send()
        .await
        .map_err(|_| "ChatGPT model catalog unavailable")?;
    if !response.status().is_success() {
        return Err("Cannot load ChatGPT models. Reconnect your account or try again.".into());
    }
    #[derive(Deserialize)]
    struct Catalog {
        models: Vec<CatalogModel>,
    }
    #[derive(Deserialize)]
    struct CatalogModel {
        visibility: String,
        slug: String,
        display_name: String,
    }
    let catalog: Catalog = response
        .json()
        .await
        .map_err(|_| "Invalid ChatGPT model catalog")?;
    let models: Vec<_> = catalog
        .models
        .into_iter()
        .filter(|m| m.visibility == "list")
        .map(|m| AIModel {
            id: m.slug,
            label: m.display_name,
        })
        .collect();
    let mut inner = ai.lock();
    if inner.epoch != epoch || inner.metadata.active.as_ref() != Some(&id) {
        return Err(CANCELED.into());
    }
    let r = inner
        .metadata
        .registrations
        .iter_mut()
        .find(|r| r.id == id)
        .ok_or("Unknown registration")?;
    let previous = r.selected.clone();
    r.models = models;
    if !r.models.iter().any(|m| Some(&m.id) == r.selected.as_ref()) {
        r.selected = r.models.first().map(|m| m.id.clone());
    }
    if previous != r.selected {
        inner.cancel_requests();
        inner.epoch += 1;
    }
    inner.save(&storage)?;
    Ok(inner.status())
}

#[tauri::command]
pub async fn ai_select(
    ai: State<'_, Ai>,
    storage: State<'_, Storage>,
    account_id: String,
    model: Option<String>,
) -> Result<AIStatus, String> {
    let mut inner = ai.lock();
    let r = inner
        .metadata
        .registrations
        .iter()
        .find(|r| r.id == account_id)
        .ok_or("Unknown ChatGPT registration")?;
    if model
        .as_ref()
        .is_some_and(|id| !r.models.iter().any(|m| &m.id == id))
    {
        return Err("Select a model from this account's catalog".into());
    }
    if inner.metadata.active.as_ref() != Some(&account_id) || r.selected != model {
        inner.invalidate();
        inner.metadata.active = Some(account_id.clone());
        inner
            .metadata
            .registrations
            .iter_mut()
            .find(|r| r.id == account_id)
            .unwrap()
            .selected = model;
        inner.save(&storage)?;
    }
    Ok(inner.status())
}

#[tauri::command]
pub async fn ai_disconnect(
    ai: State<'_, Ai>,
    storage: State<'_, Storage>,
    account_id: String,
) -> Result<Disconnect, String> {
    {
        let mut inner = ai.lock();
        if !inner
            .metadata
            .registrations
            .iter()
            .any(|r| r.id == account_id)
        {
            return Err("Unknown ChatGPT registration".into());
        }
        inner.invalidate();
        inner
            .metadata
            .registrations
            .iter_mut()
            .find(|r| r.id == account_id)
            .unwrap()
            .signed_out = true;
        if inner.metadata.active.as_ref() == Some(&account_id) {
            inner.metadata.active = None;
        }
        inner.save(&storage)?;
    }
    let _gate = ai.gate.lock().await;
    let credentials = ai.lock().credentials.remove(&account_id);
    let mut warning = None;
    if let Some(c) = credentials {
        let revoke = async {
            let endpoint = ai.revocation_url().await?;
            for attempt in 0..3 {
                let response = ai
                    .http
                    .post(endpoint.clone())
                    .form(&[
                        ("token", c.refresh.as_str()),
                        ("token_type_hint", "refresh_token"),
                        ("client_id", account_id.as_str()),
                    ])
                    .send()
                    .await;
                match response {
                    Ok(r) if r.status() == reqwest::StatusCode::OK => return Ok::<_, String>(()),
                    Ok(r) if !r.status().is_server_error() => break,
                    _ => {}
                }
                if attempt < 2 {
                    tokio::time::sleep(Duration::from_millis(250 * (1 << attempt))).await;
                }
            }
            Err("Remote revocation unconfirmed".into())
        };
        if !matches!(
            tokio::time::timeout(Duration::from_secs(15), revoke).await,
            Ok(Ok(()))
        ) {
            warning = Some("Signed out locally, but remote revocation was not confirmed. Disconnect Gloss in ChatGPT Settings.".into());
        }
    }
    if ai.vault.delete(&account_id).is_err() {
        warning = Some("Signed out in this process, but the OS credential service could not erase the saved session. Unlock your keyring and disconnect again; also disconnect Gloss in ChatGPT Settings.".into());
    }
    let mut inner = ai.lock();
    if inner.metadata.active.as_ref() == Some(&account_id) {
        inner.metadata.active = None;
    }
    inner.save(&storage)?;
    Ok(Disconnect {
        status: inner.status(),
        warning,
    })
}

#[tauri::command]
pub fn ai_cancel(ai: State<'_, Ai>, request_id: String) {
    let mut inner = ai.lock();
    if let Some(token) = inner.requests.get(&request_id) {
        token.cancel();
    } else {
        // IPC cancel may overtake scheduling of the async ask command.
        inner
            .early_cancels
            .retain(|_, time| time.elapsed() < Duration::from_secs(300));
        if inner.early_cancels.len() >= 1024 {
            if let Some(oldest) = inner
                .early_cancels
                .iter()
                .min_by_key(|(_, time)| *time)
                .map(|(id, _)| id.clone())
            {
                inner.early_cancels.remove(&oldest);
            }
        }
        inner
            .early_cancels
            .insert(request_id, std::time::Instant::now());
    }
}

struct RequestGuard<'a> {
    ai: &'a Ai,
    id: String,
}
impl Drop for RequestGuard<'_> {
    fn drop(&mut self) {
        self.ai.lock().requests.remove(&self.id);
    }
}

#[tauri::command]
pub async fn ai_ask(
    ai: State<'_, Ai>,
    request: AIRequest,
    on_delta: Channel<AIDelta>,
) -> Result<Answer, String> {
    let cancel = CancellationToken::new();
    let epoch = {
        let mut inner = ai.lock();
        if request.request_id.is_empty() || inner.requests.contains_key(&request.request_id) {
            return Err("Duplicate or empty request ID".into());
        }
        if inner.early_cancels.remove(&request.request_id).is_some() {
            return Err(CANCELED.into());
        }
        if inner.metadata.active.as_ref() != Some(&request.account_id)
            || !inner.credentials.contains_key(&request.account_id)
        {
            return Err("Requested ChatGPT account is not active and connected".into());
        }
        let r = inner
            .metadata
            .registrations
            .iter()
            .find(|r| r.id == request.account_id)
            .ok_or("Unknown registration")?;
        if r.signed_out
            || r.selected.as_ref() != Some(&request.model)
            || !r.models.iter().any(|m| m.id == request.model)
        {
            return Err("Requested model is not selected for this account".into());
        }
        inner
            .requests
            .insert(request.request_id.clone(), cancel.clone());
        inner.epoch
    };
    let _guard = RequestGuard {
        ai: &ai,
        id: request.request_id.clone(),
    };
    // Register before any await. Dropping this future aborts both send and stream.
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err(CANCELED.into()),
        result = async {
            let access = ai.token(&request.account_id, epoch).await?;
            if cancel.is_cancelled() { return Err(CANCELED.into()); }
            let response = ai.http.post(ai.endpoint("responses")).bearer_auth(access).json(&serde_json::json!({
                "model": request.model, "instructions": request.instructions, "input": request.messages, "store": false, "stream": true,
            })).send().await.map_err(|_| "ChatGPT request could not connect")?;
            if !response.status().is_success() {
                let value: serde_json::Value = response.json().await.unwrap_or_default();
                return Err(stream::provider_error(value.pointer("/error/code").and_then(serde_json::Value::as_str)));
            }
            let mut body = response.bytes_stream();
            let mut parser = stream::Stream::default();
            let mut text = String::new();
            while let Some(chunk) = body.next().await {
                let bytes = chunk.map_err(|_| "ChatGPT stream interrupted")?;
                parser.push(&bytes, |delta| {
                    if cancel.is_cancelled() { return Err(CANCELED.into()); }
                    on_delta.send(AIDelta { text: delta.clone() }).map_err(|_| "Response receiver closed")?;
                    text.push_str(&delta);
                    Ok(())
                })?;
                if parser.completed { return Ok(Answer { text }); }
            }
            parser.finish()?;
            Ok(Answer { text })
        } => result,
    }
}
