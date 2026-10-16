use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, DecodingKey, Validation};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

pub const AUTH: &str = "https://auth.openai.com/api/accounts/authorize";
pub const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
pub const RESOURCE: &str = "https://api.openai.com/v1";
pub const DIRECT: &str = "chatgpt.tokens.use.direct";
pub const DYNAMIC: &str = "dynamic_agent_client";

pub fn random() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub async fn open_browser(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid authorization URL")?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("auth.openai.com")
        || parsed.path() != "/api/accounts/authorize"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.port().is_some()
    {
        return Err("Untrusted authorization URL".into());
    }
    // Avoid browser-helper libraries that log full URLs (including ID hints).
    #[cfg(target_os = "macos")]
    let executable = "/usr/bin/open";
    #[cfg(not(target_os = "macos"))]
    let executable = "/usr/bin/xdg-open";
    let status = tokio::process::Command::new(executable)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|_| "Cannot open the system browser")?;
    if status.success() {
        Ok(())
    } else {
        Err("Cannot open the system browser".into())
    }
}

pub fn authorize(
    client: &str,
    host: &str,
    redirect: &str,
    state: &str,
    nonce: &str,
    verifier: &str,
    hint: Option<&str>,
) -> String {
    let mut url = reqwest::Url::parse(AUTH).unwrap();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    {
        let mut q = url.query_pairs_mut();
        q.extend_pairs([
            ("client_id", client),
            ("ext_agent_host_id", host),
            ("redirect_uri", redirect),
            ("response_type", "code"),
            (
                "scope",
                "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
            ),
            ("resource", RESOURCE),
            ("state", state),
            ("nonce", nonce),
            ("code_challenge_method", "S256"),
            ("code_challenge", &challenge),
        ]);
        if client == DYNAMIC {
            q.append_pair("agent_name_hint", "Gloss");
        }
        if let Some(hint) = hint {
            q.append_pair("id_token_hint", hint);
        }
    }
    url.into()
}

pub fn callback(target: &str, state: &str, client: &str) -> Result<(String, String), String> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .map_err(|_| "Invalid callback")?;
    if url.path() != "/auth/callback" {
        return Err("Invalid callback path".into());
    }
    let mut pairs = std::collections::HashMap::new();
    for (k, v) in url.query_pairs() {
        if pairs.insert(k.into_owned(), v.into_owned()).is_some() {
            return Err("Duplicate callback parameter".into());
        }
    }
    if pairs.get("state").map(String::as_str) != Some(state) {
        return Err("Invalid OAuth state".into());
    }
    if pairs.contains_key("error") {
        return Err("ChatGPT authorization was declined or failed".into());
    }
    let issued = pairs.get("client_id").map(String::as_str).unwrap_or(client);
    if issued.is_empty() || issued == DYNAMIC || (client != DYNAMIC && issued != client) {
        return Err("OAuth client registration mismatch".into());
    }
    let code = pairs
        .get("code")
        .filter(|s| !s.is_empty())
        .ok_or("Missing authorization code")?;
    Ok((code.clone(), issued.into()))
}

pub async fn listen(
    listener: tokio::net::TcpListener,
    state: &str,
    client: &str,
    cancel: &CancellationToken,
) -> Result<(String, String), String> {
    let task = async {
        loop {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|_| "Callback listener failed")?;
            let read = async {
                let mut bytes = Vec::new();
                loop {
                    let byte = socket
                        .read_u8()
                        .await
                        .map_err(|_| "Invalid callback request")?;
                    bytes.push(byte);
                    if bytes.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    if bytes.len() > 16384 {
                        return Err("Callback request too large");
                    }
                }
                String::from_utf8(bytes).map_err(|_| "Invalid callback request")
            };
            let Ok(Ok(request)) =
                tokio::time::timeout(std::time::Duration::from_secs(3), read).await
            else {
                continue;
            };
            let mut parts = request.lines().next().unwrap_or("").split_whitespace();
            if parts.next() != Some("GET") {
                continue;
            }
            let target = parts.next().unwrap_or("");
            // Ignore unrelated browser requests, but terminate on a callback with invalid bindings.
            if !target.starts_with("/auth/callback?") {
                continue;
            }
            let result = callback(target, state, client);
            let body = if result.is_ok() {
                "Authorization received. Return to Gloss."
            } else {
                "Authorization failed. Return to Gloss."
            };
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(1),
                socket.write_all(response.as_bytes()),
            )
            .await;
            return result;
        }
    };
    tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("Sign in canceled".into()),
        result = tokio::time::timeout(std::time::Duration::from_secs(300), task) => result.map_err(|_| "Sign in timed out".to_string())?,
    }
}

#[derive(Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub jwks_uri: String,
    pub revocation_endpoint: Option<String>,
}

pub fn official(url: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(url).map_err(|_| "Invalid OpenAI discovery endpoint")?;
    if url.scheme() != "https"
        || url.host_str() != Some("auth.openai.com")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Untrusted OpenAI discovery endpoint".into());
    }
    Ok(url)
}

pub async fn discovery(http: &reqwest::Client) -> Result<Discovery, String> {
    let doc: Discovery = http
        .get("https://auth.openai.com/.well-known/openid-configuration")
        .send()
        .await
        .map_err(|_| "OIDC discovery unavailable")?
        .error_for_status()
        .map_err(|_| "OIDC discovery failed")?
        .json()
        .await
        .map_err(|_| "Invalid OIDC discovery")?;
    if doc.issuer != "https://auth.openai.com" {
        return Err("Unexpected OpenAI issuer".into());
    }
    official(&doc.jwks_uri)?;
    Ok(doc)
}

#[derive(Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub nonce: Option<String>,
    pub email: Option<String>,
    pub aud: serde_json::Value,
    pub azp: Option<String>,
}

pub fn validate(
    token: &str,
    jwks: &JwkSet,
    issuer: &str,
    client: &str,
    nonce: Option<&str>,
    subject: Option<&str>,
) -> Result<Claims, String> {
    let fail = || "ID token validation failed".to_string();
    let header = decode_header(token).map_err(|_| fail())?;
    if !matches!(
        header.alg,
        jsonwebtoken::Algorithm::RS256 | jsonwebtoken::Algorithm::ES256
    ) {
        return Err(fail());
    }
    let key = jwks
        .find(header.kid.as_deref().ok_or_else(fail)?)
        .ok_or_else(fail)?;
    let mut rules = Validation::new(header.alg);
    rules.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    rules.set_issuer(&[issuer]);
    rules.set_audience(&[client]);
    rules.leeway = 0;
    let claims = decode::<Claims>(
        token,
        &DecodingKey::from_jwk(key).map_err(|_| fail())?,
        &rules,
    )
    .map_err(|_| fail())?
    .claims;
    if claims.sub.is_empty()
        || nonce.is_some_and(|n| claims.nonce.as_deref() != Some(n))
        || subject.is_some_and(|s| s != claims.sub)
        || claims.azp.as_deref().is_some_and(|azp| azp != client)
        || (claims.aud.as_array().is_some_and(|aud| aud.len() > 1)
            && claims.azp.as_deref() != Some(client))
    {
        return Err(fail());
    }
    Ok(claims)
}

pub async fn verify(
    http: &reqwest::Client,
    token: &str,
    client: &str,
    nonce: Option<&str>,
    subject: Option<&str>,
) -> Result<Claims, String> {
    let doc = discovery(http).await?;
    let keys: JwkSet = http
        .get(official(&doc.jwks_uri)?)
        .send()
        .await
        .map_err(|_| "JWKS unavailable")?
        .error_for_status()
        .map_err(|_| "JWKS unavailable")?
        .json()
        .await
        .map_err(|_| "Invalid JWKS")?;
    validate(token, &keys, &doc.issuer, client, nonce, subject)
}
