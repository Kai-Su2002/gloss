use super::*;

#[test]
fn callback_binds_state_and_issued_client() {
    assert!(oauth::callback(
        "/auth/callback?code=c&state=s&client_id=oaiapp_a",
        "s",
        oauth::DYNAMIC
    )
    .is_ok());
    assert!(oauth::callback("/auth/callback?code=c&state=s", "s", "oaiapp_a").is_ok());
    for (target, client) in [
        (
            "/auth/callback?code=c&state=wrong&client_id=oaiapp_a",
            oauth::DYNAMIC,
        ),
        ("/auth/callback?code=c&state=s", oauth::DYNAMIC),
        (
            "/auth/callback?code=c&state=s&client_id=dynamic_agent_client",
            oauth::DYNAMIC,
        ),
        (
            "/auth/callback?code=c&state=s&client_id=oaiapp_b",
            "oaiapp_a",
        ),
        ("/auth/callback?code=c&state=s&state=s", "oaiapp_a"),
        ("/auth/callback?error=access_denied&state=s", "oaiapp_a"),
        ("/callback?code=c&state=s", "oaiapp_a"),
    ] {
        assert!(oauth::callback(target, "s", client).is_err());
    }
}

#[test]
fn authorization_is_public_oidc_pkce_and_official_only() {
    let url = oauth::authorize(
        oauth::DYNAMIC,
        "urn:uuid:host",
        "http://127.0.0.1:1234/auth/callback",
        "state",
        "nonce",
        "verifier",
        None,
    );
    let url = reqwest::Url::parse(&url).unwrap();
    assert_eq!(url.host_str(), Some("auth.openai.com"));
    let q: HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(q["agent_name_hint"], "Gloss");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["resource"], oauth::RESOURCE);
    assert_ne!(q["code_challenge"], "verifier");
    assert!(!q.contains_key("client_secret"));
    let returning = oauth::authorize(
        "issued",
        "host",
        "redirect",
        "s",
        "n",
        "v",
        Some("test-id-token"),
    );
    assert!(!returning.contains("agent_name_hint"));
    for url in [
        "http://auth.openai.com/key",
        "https://evil.test/key",
        "https://auth.openai.com.evil.test/key",
        "https://user@auth.openai.com/key",
    ] {
        assert!(oauth::official(url).is_err());
    }
}

#[test]
fn sse_handles_every_byte_boundary_and_requires_completion() {
    let data = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hé🦀\"}\r\n\r\ndata: {\"type\":\"response.completed\"}\n\n";
    for split in 0..=data.len() {
        let mut parser = stream::Stream::default();
        let mut text = String::new();
        for chunk in [&data.as_bytes()[..split], &data.as_bytes()[split..]] {
            parser
                .push(chunk, |d| {
                    text.push_str(&d);
                    Ok(())
                })
                .unwrap();
        }
        parser.finish().unwrap();
        assert_eq!(text, "hé🦀");
    }
    let mut parser = stream::Stream::default();
    for byte in data.bytes() {
        parser.push(&[byte], |_| Ok(())).unwrap();
    }
    parser.finish().unwrap();
    for data in [
        "",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
        "data: [DONE]\n\n",
        "data: {\"type\":\"response.incomplete\"}\n\n",
    ] {
        let mut parser = stream::Stream::default();
        assert!(parser
            .push(data.as_bytes(), |_| Ok(()))
            .and_then(|_| parser.finish())
            .is_err());
    }
}

#[test]
fn late_quota_error_preserves_deltas_without_exposing_body() {
    let mut parser = stream::Stream::default();
    let mut text = String::new();
    let data = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\ndata: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"SECRET\"}}}\n\n";
    let error = parser
        .push(data.as_bytes(), |d| {
            text.push_str(&d);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(text, "partial");
    assert!(error.contains("sharing limit"));
    assert!(!error.contains("SECRET"));
}

#[tokio::test]
async fn loopback_listener_cancellation_is_immediate() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        oauth::listen(listener, "s", "client", &cancel),
    )
    .await
    .unwrap();
    assert!(result.is_err());
}

#[test]
fn signed_id_tokens_validate_signature_issuer_audience_expiry_nonce_and_subject() {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let key =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let public = key.public_key().as_ref();
    let keys = serde_json::from_value(serde_json::json!({"keys":[{
        "kty":"EC", "crv":"P-256", "kid":"test", "alg":"ES256", "use":"sig",
        "x":URL_SAFE_NO_PAD.encode(&public[1..33]), "y":URL_SAFE_NO_PAD.encode(&public[33..65]),
    }]}))
    .unwrap();
    let claims = serde_json::json!({"sub":"subject", "iss":"https://auth.openai.com", "aud":"issued", "exp":now()+300, "nonce":"nonce"});
    let sign = |claims: &serde_json::Value| {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        header.kid = Some("test".into());
        jsonwebtoken::encode(
            &header,
            claims,
            &jsonwebtoken::EncodingKey::from_ec_der(pkcs8.as_ref()),
        )
        .unwrap()
    };
    let token = sign(&claims);
    assert!(oauth::validate(
        &token,
        &keys,
        "https://auth.openai.com",
        "issued",
        Some("nonce"),
        Some("subject")
    )
    .is_ok());
    for (field, bad) in [
        ("iss", serde_json::json!("https://evil.test")),
        ("aud", serde_json::json!("other")),
        ("exp", serde_json::json!(now() - 1)),
        ("nonce", serde_json::json!("other")),
        ("sub", serde_json::json!("other")),
    ] {
        let mut bad_claims = claims.clone();
        bad_claims[field] = bad;
        assert!(
            oauth::validate(
                &sign(&bad_claims),
                &keys,
                "https://auth.openai.com",
                "issued",
                Some("nonce"),
                Some("subject")
            )
            .is_err(),
            "{field}"
        );
    }
    for field in ["iss", "aud", "exp", "nonce", "sub"] {
        let mut missing = claims.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            oauth::validate(
                &sign(&missing),
                &keys,
                "https://auth.openai.com",
                "issued",
                Some("nonce"),
                None
            )
            .is_err(),
            "missing {field}"
        );
    }
    let mut forged = token.into_bytes();
    let last = forged.len() - 5;
    forged[last] = if forged[last] == b'A' { b'B' } else { b'A' };
    assert!(oauth::validate(
        std::str::from_utf8(&forged).unwrap(),
        &keys,
        "https://auth.openai.com",
        "issued",
        Some("nonce"),
        None
    )
    .is_err());
}

#[derive(Default)]
struct TestVault {
    values: Mutex<HashMap<String, Credentials>>,
    fail: std::sync::atomic::AtomicBool,
}
impl Vault for TestVault {
    fn get(&self, id: &str) -> Result<Option<Credentials>, String> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(KEYRING_ERROR.into());
        }
        Ok(self.values.lock().unwrap().get(id).cloned())
    }
    fn set(&self, id: &str, c: &Credentials) -> Result<(), String> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(KEYRING_ERROR.into());
        }
        self.values.lock().unwrap().insert(id.into(), c.clone());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(KEYRING_ERROR.into());
        }
        self.values.lock().unwrap().remove(id);
        Ok(())
    }
}
fn credential(id: &str) -> Credentials {
    Credentials {
        client: id.into(),
        subject: "same-subject".into(),
        access: format!("access-{id}"),
        refresh: format!("refresh-{id}"),
        id_token: format!("id-{id}"),
        scopes: vec![oauth::DIRECT.into()],
        expires: now() + 3600,
    }
}
fn fixture() -> (tempfile::TempDir, Storage, Ai, Arc<TestVault>) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(&dir.path().join("test.sqlite3")).unwrap();
    let metadata = Metadata {
        host: "urn:uuid:stable-host".into(),
        active: Some("a".into()),
        registrations: ["a", "b"]
            .into_iter()
            .map(|id| Registration {
                id: id.into(),
                subject: "same-subject".into(),
                label: format!("same@email.test · {id}"),
                models: vec![AIModel {
                    id: format!("model-{id}"),
                    label: "Model".into(),
                }],
                selected: Some(format!("model-{id}")),
                signed_out: false,
            })
            .collect(),
    };
    storage
        .save_state("gloss:ai", &serde_json::to_string(&metadata).unwrap())
        .unwrap();
    let vault = Arc::new(TestVault::default());
    for id in ["a", "b"] {
        vault.set(id, &credential(id)).unwrap();
    }
    let ai = Ai::with_vault(&storage, vault.clone()).unwrap();
    (dir, storage, ai, vault)
}
fn app(ai: Ai, storage: Storage) -> tauri::App<tauri::test::MockRuntime> {
    tauri::test::mock_builder()
        .manage(ai)
        .manage(storage)
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}
fn request(id: &str) -> AIRequest {
    AIRequest {
        request_id: id.into(),
        account_id: "a".into(),
        model: "model-a".into(),
        instructions: "Answer briefly".into(),
        messages: vec![Message {
            role: Role::User,
            content: "hello".into(),
        }],
    }
}
fn channel() -> Channel<AIDelta> {
    Channel::new(|_| Ok(()))
}

/// Test-only transport: production endpoints are constants, not IPC input.
async fn server(
    body: String,
    delay: Duration,
) -> (
    String,
    tokio::sync::oneshot::Receiver<String>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}/", listener.local_addr().unwrap());
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut data = Vec::new();
        let header_end = loop {
            data.push(socket.read_u8().await.unwrap());
            if data.ends_with(b"\r\n\r\n") {
                break data.len();
            }
        };
        let header = String::from_utf8(data.clone()).unwrap();
        let length = header
            .lines()
            .find_map(|s| {
                s.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|n| n.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        data.resize(header_end + length, 0);
        socket.read_exact(&mut data[header_end..]).await.unwrap();
        let _ = tx.send(String::from_utf8(data).unwrap());
        tokio::time::sleep(delay).await;
        let headers = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
        let _ = socket.write_all(headers.as_bytes()).await;
        let _ = socket.write_all(body.as_bytes()).await;
    });
    (address, rx, task)
}

#[tokio::test]
async fn public_status_switch_and_offline_startup_keep_registration_isolation() {
    use tauri::Manager;
    let (_dir, storage, ai, vault) = fixture();
    let app = app(ai, storage);
    let status = ai_status(app.state());
    assert_eq!(status.accounts.len(), 2);
    assert_ne!(status.accounts[0].label, status.accounts[1].label);
    assert!(
        ai_select(app.state(), app.state(), "b".into(), Some("model-a".into()))
            .await
            .is_err()
    );
    let status = ai_select(app.state(), app.state(), "b".into(), Some("model-b".into()))
        .await
        .unwrap();
    assert_eq!(status.active_account_id.as_deref(), Some("b"));
    let saved = app
        .state::<Storage>()
        .load_state("gloss:ai")
        .unwrap()
        .unwrap();
    for secret in ["access-a", "refresh-a", "id-a", "access-b"] {
        assert!(!saved.contains(secret));
    }
    vault.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let restarted = Ai::with_vault(&app.state::<Storage>(), vault).unwrap();
    assert!(restarted
        .lock()
        .status()
        .accounts
        .iter()
        .all(|a| !a.connected));
    assert_eq!(restarted.lock().metadata.host, "urn:uuid:stable-host");
}

#[tokio::test]
async fn refresh_is_serialized_rotates_whole_record_and_leaves_other_account_untouched() {
    let (_dir, _storage, mut ai, vault) = fixture();
    ai.lock().credentials.get_mut("a").unwrap().expires = 0;
    let body = serde_json::json!({"access_token":"rotated-access", "refresh_token":"rotated-refresh", "token_type":"Bearer", "expires_in":3600, "scope":oauth::DIRECT}).to_string();
    let (url, request, server) = server(body, Duration::from_millis(20)).await;
    ai.endpoints.insert("token", url);
    let (first, second) = tokio::join!(ai.token("a", 0), ai.token("a", 0));
    assert_eq!(first.unwrap(), "rotated-access");
    assert_eq!(second.unwrap(), "rotated-access");
    let sent = request.await.unwrap();
    assert!(sent.contains("client_id=a"));
    assert!(sent.contains("refresh_token=refresh-a"));
    assert!(!sent.contains("scope="));
    server.await.unwrap();
    let stored = vault.get("a").unwrap().unwrap();
    assert_eq!(stored.refresh, "rotated-refresh");
    assert_eq!(stored.access, "rotated-access");
    assert_eq!(stored.id_token, "id-a");
    assert!(stored.can_use_plan());
    assert_eq!(vault.get("b").unwrap().unwrap().refresh, "refresh-b");
}

#[tokio::test]
async fn models_preserve_server_order_and_choose_first_visible_model() {
    use tauri::Manager;
    let (_dir, storage, mut ai, _vault) = fixture();
    let body = serde_json::json!({"models":[
        {"visibility":"hidden","slug":"hidden","display_name":"Hidden"},
        {"visibility":"list","slug":"second","display_name":"Second"},
        {"visibility":"list","slug":"first","display_name":"First"},
    ]})
    .to_string();
    let (url, received, server) = server(body, Duration::ZERO).await;
    ai.endpoints.insert("models", url);
    let app = app(ai, storage);
    let status = ai_models(app.state(), app.state()).await.unwrap();
    assert_eq!(
        status
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
    assert_eq!(status.selected_model.as_deref(), Some("second"));
    assert!(received.await.unwrap().contains("Bearer access-a"));
    server.await.unwrap();
}

#[tokio::test]
async fn public_ask_has_exact_payload_and_only_succeeds_after_completed() {
    use tauri::Manager;
    for terminal in [
        "response.completed",
        "response.failed",
        "response.incomplete",
        "abrupt",
    ] {
        let (_dir, storage, mut ai, _vault) = fixture();
        let mut body =
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hé🦀\"}\n\n".to_owned();
        if terminal != "abrupt" {
            body.push_str(&format!("data: {{\"type\":\"{terminal}\"}}\n\n"));
        }
        let (url, received, server) = server(body, Duration::ZERO).await;
        ai.endpoints.insert("responses", url);
        let app = app(ai, storage);
        let deltas = Arc::new(Mutex::new(String::new()));
        let output = deltas.clone();
        let channel = Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
            let value: serde_json::Value = body.deserialize().unwrap();
            output
                .lock()
                .unwrap()
                .push_str(value["text"].as_str().unwrap());
            Ok(())
        });
        let answer = ai_ask(app.state(), request("req"), channel).await;
        assert_eq!(answer.is_ok(), terminal == "response.completed");
        if let Ok(answer) = answer {
            assert_eq!(answer.text, "hé🦀");
        }
        assert_eq!(*deltas.lock().unwrap(), "hé🦀");
        let sent = received.await.unwrap();
        let payload: serde_json::Value =
            serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(
            payload,
            serde_json::json!({"model":"model-a","instructions":"Answer briefly","input":[{"role":"user","content":"hello"}],"store":false,"stream":true})
        );
        assert!(app.state::<Ai>().lock().requests.is_empty());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn cancellation_before_start_and_during_http_and_duplicate_ids_are_safe() {
    use tauri::Manager;
    let (_dir, storage, mut ai, _vault) = fixture();
    let (url, received, server) = server(String::new(), Duration::from_secs(60)).await;
    ai.endpoints.insert("responses", url);
    let app = app(ai, storage);
    ai_cancel(app.state(), "early".into());
    assert_eq!(
        ai_ask(app.state(), request("early"), channel())
            .await
            .err()
            .unwrap(),
        CANCELED
    );
    let ask = ai_ask(app.state(), request("active"), channel());
    tokio::pin!(ask);
    tokio::select! { _ = received => {}, _ = &mut ask => panic!("request should wait for HTTP") }
    assert!(ai_ask(app.state(), request("active"), channel())
        .await
        .err()
        .unwrap()
        .contains("Duplicate"));
    assert_eq!(app.state::<Ai>().lock().requests.len(), 1);
    ai_cancel(app.state(), "active".into());
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), &mut ask)
            .await
            .unwrap()
            .err()
            .unwrap(),
        CANCELED
    );
    assert!(app.state::<Ai>().lock().requests.is_empty());
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn failed_keyring_erase_does_not_reconnect_after_restart() {
    use tauri::Manager;
    let (_dir, storage, ai, vault) = fixture();
    // Simulate a credential service lost after startup; avoid external revocation.
    ai.lock().credentials.remove("a");
    vault.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let app = app(ai, storage);
    let result = ai_disconnect(app.state(), app.state(), "a".into())
        .await
        .unwrap();
    assert!(result.warning.unwrap().contains("could not erase"));
    vault.fail.store(false, std::sync::atomic::Ordering::SeqCst);
    let restarted = Ai::with_vault(&app.state::<Storage>(), vault).unwrap();
    let status = restarted.lock().status();
    assert!(!status.accounts[0].connected);
    assert!(status.accounts[1].connected);
    assert_eq!(restarted.lock().metadata.host, "urn:uuid:stable-host");
}

#[tokio::test]
async fn logout_during_rotation_revokes_latest_refresh_and_cannot_resurrect_session() {
    use tauri::Manager;
    let (_dir, storage, mut ai, vault) = fixture();
    ai.lock().credentials.get_mut("a").unwrap().expires = 0;
    let pending_login = CancellationToken::new();
    ai.lock().login = Some(("pending".into(), pending_login.clone()));
    let body = serde_json::json!({"access_token":"new-access", "refresh_token":"new-refresh", "token_type":"Bearer", "expires_in":3600}).to_string();
    let (token_url, token_received, token_server) = server(body, Duration::from_millis(30)).await;
    let (revoke_url, revoke_received, revoke_server) = server(String::new(), Duration::ZERO).await;
    ai.endpoints.insert("token", token_url);
    ai.endpoints.insert("revoke", revoke_url);
    let renewal_ai = ai.clone();
    let renewal = tokio::spawn(async move { renewal_ai.token("a", 0).await });
    token_received.await.unwrap();
    let app = app(ai, storage);
    let logout = ai_disconnect(app.state(), app.state(), "a".into());
    tokio::pin!(logout);
    tokio::select! {
        biased;
        result = &mut logout => panic!("logout should wait for rotation: {}", result.is_ok()),
        _ = tokio::task::yield_now() => {},
    }
    assert!(pending_login.is_cancelled());
    assert!(!ai_status(app.state()).accounts[0].connected);
    let disconnected = logout.await.unwrap();
    assert!(disconnected.warning.is_none());
    assert!(!disconnected.status.accounts[0].connected);
    assert_eq!(renewal.await.unwrap().unwrap_err(), CANCELED);
    assert!(revoke_received.await.unwrap().contains("token=new-refresh"));
    token_server.await.unwrap();
    revoke_server.await.unwrap();
    assert!(vault.get("a").unwrap().is_none());
    assert!(vault.get("b").unwrap().is_some());
    let restarted = Ai::with_vault(&app.state::<Storage>(), vault).unwrap();
    assert!(!restarted.lock().status().accounts[0].connected);
}

#[tokio::test]
async fn switching_account_cancels_http_without_destroying_saved_sessions() {
    use tauri::Manager;
    let (_dir, storage, mut ai, vault) = fixture();
    let (url, received, server) = server(String::new(), Duration::from_secs(60)).await;
    ai.endpoints.insert("responses", url);
    let app = app(ai, storage);
    let ask = ai_ask(app.state(), request("switch"), channel());
    tokio::pin!(ask);
    tokio::select! { _ = received => {}, _ = &mut ask => panic!("request should wait for HTTP") }
    ai_select(app.state(), app.state(), "b".into(), Some("model-b".into()))
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), &mut ask)
            .await
            .unwrap()
            .err()
            .unwrap(),
        CANCELED
    );
    assert!(ai_status(app.state()).accounts.iter().all(|a| a.connected));
    assert!(vault.get("a").unwrap().is_some());
    server.abort();
    let _ = server.await;
}

#[tokio::test]
async fn native_http_never_follows_credential_bearing_redirects() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, _storage, ai, _vault) = fixture();
    let source = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sink = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", source.local_addr().unwrap());
    let destination = format!("http://{}/", sink.local_addr().unwrap());
    let serve = tokio::spawn(async move {
        let (mut socket, _) = source.accept().await.unwrap();
        let mut data = vec![];
        while !data.ends_with(b"\r\n\r\n") {
            data.push(socket.read_u8().await.unwrap());
        }
        socket.write_all(format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
    });
    let response = ai
        .http
        .get(url)
        .bearer_auth("test-only-secret")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
    assert!(
        tokio::time::timeout(Duration::from_millis(50), sink.accept())
            .await
            .is_err()
    );
    serve.await.unwrap();
}

#[tokio::test]
async fn cancel_while_streaming_preserves_delta_and_closes_http_connection() {
    use tauri::Manager;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (_dir, storage, mut ai, _vault) = fixture();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    ai.endpoints.insert(
        "responses",
        format!("http://{}/", listener.local_addr().unwrap()),
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut headers = Vec::new();
        while !headers.ends_with(b"\r\n\r\n") {
            headers.push(socket.read_u8().await.unwrap());
        }
        let headers = String::from_utf8(headers).unwrap();
        let length = headers
            .lines()
            .find_map(|line| {
                line.to_lowercase()
                    .strip_prefix("content-length:")
                    .map(|s| s.trim().parse::<usize>().unwrap())
            })
            .unwrap();
        let mut request = vec![0; length];
        socket.read_exact(&mut request).await.unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n").await.unwrap();
        let mut byte = [0];
        // No completed event: cancellation must drop the response/socket, rather
        // than just stop forwarding deltas while inference keeps running.
        tokio::time::timeout(Duration::from_secs(1), socket.read(&mut byte))
            .await
            .unwrap()
            .unwrap()
    });
    let app = app(ai, storage);
    let deltas = Arc::new(Mutex::new(String::new()));
    let output = deltas.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let on_delta = Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
        let value: serde_json::Value = body.deserialize().unwrap();
        output
            .lock()
            .unwrap()
            .push_str(value["text"].as_str().unwrap());
        tx.send(()).unwrap();
        Ok(())
    });
    let ask = ai_ask(app.state(), request("stream"), on_delta);
    tokio::pin!(ask);
    tokio::select! { _ = rx.recv() => {}, _ = &mut ask => panic!("stream should wait for completion") }
    ai_cancel(app.state(), "stream".into());
    assert_eq!(ask.await.err().unwrap(), CANCELED);
    assert_eq!(*deltas.lock().unwrap(), "partial");
    assert_eq!(server.await.unwrap(), 0);
}

#[tokio::test]
async fn identity_without_response_scope_cannot_start_inference() {
    use tauri::Manager;
    let (_dir, storage, ai, _vault) = fixture();
    ai.lock().credentials.get_mut("a").unwrap().scopes.clear();
    let app = app(ai, storage);
    let status = ai_status(app.state());
    assert!(status.accounts[0].connected);
    assert!(!status.accounts[0].can_use_plan);
    let error = ai_ask(app.state(), request("no-plan"), channel())
        .await
        .err()
        .unwrap();
    assert!(error.contains("not authorized"));
    assert!(app.state::<Ai>().lock().requests.is_empty());
    let json = serde_json::to_string(&status).unwrap();
    for secret in ["access-a", "refresh-a", "id-a"] {
        assert!(!json.contains(secret));
    }
}
