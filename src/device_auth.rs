//! Backend-only client for PedDavid/immich-device-auth's RFC 8628 flow.
use crate::grant_store::{GrantStore, LoadError, StoredGrant};
use crate::immich::Immich;
use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use reqwest::{Client, Url};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const SCOPE: &str = "asset.read asset.view asset.update";

pub struct DeviceAuth {
    client: Client,
    broker: Url,
    immich_url: String,
    /// Where the active grant is persisted, encrypted; `None` keeps it in memory only.
    store: Option<GrantStore>,
    state: Mutex<Session>,
}
#[derive(Default)]
struct Session {
    pending: Option<Pending>,
    grant: Option<Grant>,
    outcome: Option<&'static str>,
}
struct Pending {
    code: String,
    user_code: String,
    verification_uri: String,
    deadline: Instant,
    interval: u64,
    next_poll: Instant,
}
struct Grant {
    token: String,
    scope: String,
    source: Arc<Immich>,
    expires_at: DateTime<Utc>,
    deadline: Instant,
}
#[derive(Deserialize)]
struct Challenge {
    device_code: String,
    user_code: String,
    verification_uri_complete: String,
    expires_in: u64,
    interval: u64,
}
#[derive(Deserialize)]
struct Token {
    access_token: String,
    token_type: String,
    expires_in: u64,
    expires_at: DateTime<Utc>,
    scope: String,
}

fn base_url(input: &str) -> anyhow::Result<Url> {
    let url = Url::parse(&format!("{}/", input.trim_end_matches('/')))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("invalid device-auth URL");
    }
    Ok(url)
}
/// The issued scope must be exactly the one requested, in any order.
fn valid_scope(scope: &str) -> bool {
    let scope: Vec<_> = scope.split_whitespace().collect();
    SCOPE.split_whitespace().all(|s| scope.contains(&s))
        && scope
            .iter()
            .all(|s| SCOPE.split_whitespace().any(|allowed| allowed == *s))
}
impl Session {
    /// Drops what has expired. Returns whether an active grant was dropped.
    fn expire(&mut self) -> bool {
        if self.pending.as_ref().is_some_and(|p| Instant::now() >= p.deadline) {
            self.pending = None;
            self.outcome = Some("expired");
        }
        if self
            .grant
            .as_ref()
            .is_some_and(|g| Instant::now() >= g.deadline || Utc::now() >= g.expires_at)
        {
            self.grant = None;
            self.outcome = Some("expired");
            return true;
        }
        false
    }
    fn public(&self) -> Value {
        if let Some(g) = &self.grant {
            return json!({"status":"connected","expires_at":g.expires_at});
        }
        if let Some(p) = &self.pending {
            return json!({"status":"pending","user_code":p.user_code,"verification_uri":p.verification_uri,
                "expires_in":p.deadline.saturating_duration_since(Instant::now()).as_secs(),"interval":p.interval});
        }
        json!({"status":self.outcome.unwrap_or("disconnected")})
    }
}
impl DeviceAuth {
    pub fn new(broker: &str, immich_url: &str) -> anyhow::Result<Self> {
        // Validate the configured destination independently of any URL returned by the broker.
        Immich::new(immich_url, "validation-only")?;
        Ok(Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()?,
            broker: base_url(broker)?,
            immich_url: immich_url.into(),
            store: None,
            state: Mutex::new(Session::default()),
        })
    }
    /// A grant store in `dir`, bound to this broker and Immich server; `None` without a key.
    pub fn store_in(&self, dir: &std::path::Path, hq_key: Option<&str>) -> Option<GrantStore> {
        GrantStore::new(dir, hq_key, self.broker.as_str(), &self.immich_url)
    }
    /// Persists grants to `store` and reloads the one stored there, if it is still usable.
    /// An expired, undecryptable or otherwise unusable stored grant is deleted.
    pub fn with_store(mut self, store: Option<GrantStore>) -> Self {
        self.store = store;
        let Some(store) = &self.store else {
            return self;
        };
        let loaded = match store.load() {
            Ok(stored) => self.restore(stored),
            Err(LoadError::Missing) => return self,
            Err(e) => Err(match e {
                LoadError::Undecryptable => "it could not be decrypted (HQ_KEY or the Immich URLs changed?)",
                _ => "it could not be read",
            }),
        };
        match loaded {
            Ok(grant) => {
                tracing::info!(expires_at = %grant.expires_at, "restored the stored Immich device grant");
                self.state.get_mut().grant = Some(grant);
            }
            Err(reason) => {
                tracing::warn!(path = %store.path().display(), "discarding the stored Immich device grant: {reason}");
                self.forget();
            }
        }
        self
    }
    fn restore(&self, stored: StoredGrant) -> Result<Grant, &'static str> {
        let remaining = (stored.deadline.min(stored.expires_at) - Utc::now())
            .to_std()
            .map_err(|_| "it has expired")?;
        if remaining.is_zero() {
            return Err("it has expired");
        }
        if !valid_scope(&stored.scope) || stored.token.is_empty() {
            return Err("its scope is not the one Home HQ requests");
        }
        let source = Immich::new(&self.immich_url, &stored.token).map_err(|_| "it is not a usable key")?;
        Ok(Grant {
            token: stored.token,
            scope: stored.scope,
            source: Arc::new(source),
            expires_at: stored.expires_at,
            deadline: Instant::now() + remaining,
        })
    }
    fn persist(&self, grant: &Grant) {
        let Some(store) = &self.store else { return };
        let stored = StoredGrant {
            token: grant.token.clone(),
            expires_at: grant.expires_at,
            deadline: Utc::now()
                + chrono::Duration::from_std(grant.deadline.saturating_duration_since(Instant::now()))
                    .unwrap_or_default(),
            scope: grant.scope.clone(),
        };
        // Pairing still succeeds; it just won't survive a restart.
        if let Err(e) = store.save(&stored) {
            tracing::error!(path = %store.path().display(), "could not persist the Immich device grant: {e:#}");
        }
    }
    fn forget(&self) {
        if let Some(store) = &self.store
            && let Err(e) = store.delete()
        {
            tracing::error!(path = %store.path().display(), "could not delete the stored Immich device grant: {e}");
        }
    }
    /// Like `Session::expire`, also deleting a stored grant that has expired.
    fn expire(&self, state: &mut Session) {
        if state.expire() {
            self.forget();
        }
    }
    pub fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(broker) = std::env::var("HQ_IMMICH_DEVICE_AUTH_URL").ok() else {
            return Ok(None);
        };
        if std::env::var_os("HQ_IMMICH_API_KEY").is_some() {
            bail!("choose either Immich device auth or a static API key");
        }
        let immich = std::env::var("HQ_IMMICH_URL").context("device auth requires HQ_IMMICH_URL including /api")?;
        let auth = Self::new(&broker, &immich)?;
        let Some(dir) = std::env::var_os("HQ_STATE_DIR").filter(|d| !d.is_empty()) else {
            return Ok(Some(auth));
        };
        let store = auth.store_in(dir.as_ref(), std::env::var("HQ_KEY").ok().as_deref());
        if store.is_none() {
            tracing::warn!("HQ_STATE_DIR is set but HQ_KEY is not: the Immich grant is kept in memory only");
        }
        Ok(Some(auth.with_store(store)))
    }
    pub async fn status(&self) -> Value {
        let mut state = self.state.lock().await;
        self.expire(&mut state);
        state.public()
    }
    pub async fn source(&self) -> Option<Arc<Immich>> {
        let mut state = self.state.lock().await;
        self.expire(&mut state);
        state.grant.as_ref().map(|g| g.source.clone())
    }
    pub async fn start(&self) -> anyhow::Result<Value> {
        let mut state = self.state.lock().await;
        self.expire(&mut state);
        if state.pending.is_some() || state.grant.is_some() {
            return Ok(state.public());
        }
        let challenge: Challenge = self
            .client
            .post(self.broker.join("device/code")?)
            .json(&json!({"client_name":"Home HQ photo frame","scope":SCOPE,"ttl":86400}))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        self.accept_challenge(&mut state, challenge)?;
        Ok(state.public())
    }
    fn accept_challenge(&self, state: &mut Session, c: Challenge) -> anyhow::Result<()> {
        let url = Url::parse(&c.verification_uri_complete)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || c.device_code.is_empty()
            || c.user_code.is_empty()
            || !(1..=3600).contains(&c.expires_in)
            || !(1..=300).contains(&c.interval)
        {
            bail!("invalid device challenge");
        }
        // Generate our own SVG; never render markup supplied by the broker.
        qrcode::QrCode::new(c.verification_uri_complete.as_bytes())?;
        state.pending = Some(Pending {
            code: c.device_code,
            user_code: c.user_code,
            verification_uri: c.verification_uri_complete,
            deadline: Instant::now() + Duration::from_secs(c.expires_in),
            interval: c.interval,
            next_poll: Instant::now() + Duration::from_secs(c.interval),
        });
        state.outcome = None;
        Ok(())
    }
    pub async fn qr(&self) -> anyhow::Result<Option<String>> {
        let mut state = self.state.lock().await;
        self.expire(&mut state);
        state
            .pending
            .as_ref()
            .map(|p| {
                Ok(qrcode::QrCode::new(p.verification_uri.as_bytes())?
                    .render::<qrcode::render::svg::Color>()
                    .min_dimensions(320, 320)
                    .build())
            })
            .transpose()
    }
    pub async fn poll(&self) -> anyhow::Result<Value> {
        let mut state = self.state.lock().await;
        self.expire(&mut state);
        let Some(p) = &mut state.pending else {
            return Ok(state.public());
        };
        if Instant::now() < p.next_poll {
            return Ok(state.public());
        }
        p.next_poll = Instant::now() + Duration::from_secs(p.interval);
        let response = self
            .client
            .post(self.broker.join("device/token")?)
            .json(&json!({"grant_type":"urn:ietf:params:oauth:grant-type:device_code","device_code":p.code}))
            .send()
            .await?;
        let success = response.status().is_success();
        let body: Value = response.json().await?;
        if !success {
            match body["error"].as_str() {
                Some("authorization_pending") => {}
                Some("slow_down") => {
                    p.interval = (p.interval + 5).min(300);
                }
                Some("access_denied") => {
                    state.pending = None;
                    state.outcome = Some("denied");
                }
                Some("expired_token") => {
                    state.pending = None;
                    state.outcome = Some("expired");
                }
                _ => bail!("device authorization failed"),
            }
            if let Some(p) = &mut state.pending {
                p.next_poll = Instant::now() + Duration::from_secs(p.interval);
            }
            return Ok(state.public());
        }
        let token: Token = serde_json::from_value(body)?;
        let remaining = (token.expires_at - Utc::now()).num_seconds();
        if token.token_type != "immich_api_key" || remaining <= 0 || token.expires_in == 0 || !valid_scope(&token.scope)
        {
            bail!("invalid device grant");
        }
        let source = Arc::new(Immich::new(&self.immich_url, &token.access_token)?);
        let grant = Grant {
            token: token.access_token,
            scope: token.scope,
            source,
            expires_at: token.expires_at,
            deadline: Instant::now() + Duration::from_secs(token.expires_in.min(remaining as u64).min(7 * 86400)),
        };
        self.persist(&grant);
        state.grant = Some(grant);
        state.pending = None;
        state.outcome = None;
        Ok(state.public())
    }
    pub async fn disconnect(&self) -> anyhow::Result<Value> {
        let mut state = self.state.lock().await;
        // Retain the grant if revocation fails, so the user can retry.
        if let Some(g) = &state.grant {
            self.client
                .post(self.broker.join("device/revoke")?)
                .json(&json!({"token":g.token}))
                .send()
                .await?
                .error_for_status()?;
        }
        *state = Session::default();
        // Also when nothing was in memory, so a retry clears a file a failed delete left behind.
        if let Some(store) = &self.store {
            store
                .delete()
                .context("could not delete the stored Immich device grant")?;
        }
        Ok(state.public())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grant_store::tests::TempDir;
    use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
    use http_body_util::BodyExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[derive(Clone)]
    struct Mock {
        reply: Arc<Mutex<(StatusCode, Value)>>,
        polls: Arc<AtomicUsize>,
        codes: Arc<AtomicUsize>,
        revokes: Arc<AtomicUsize>,
        revoke_fails: Arc<std::sync::atomic::AtomicBool>,
    }
    async fn code(State(mock): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
        mock.codes.fetch_add(1, Ordering::SeqCst);
        assert_eq!(body["scope"], SCOPE);
        assert_eq!(body["ttl"], 86400);
        Json(json!({"device_code":"private-device-code","user_code":"ABCD-EFGH",
            "verification_uri_complete":"https://broker.example/device?user_code=ABCD-EFGH",
            "verification_uri_complete_qr":"<svg><script>bad()</script></svg>",
            "expires_in":600,"interval":5}))
    }
    async fn token(State(mock): State<Mock>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
        assert_eq!(body["device_code"], "private-device-code");
        assert_eq!(body["grant_type"], "urn:ietf:params:oauth:grant-type:device_code");
        mock.polls.fetch_add(1, Ordering::SeqCst);
        let (status, body) = mock.reply.lock().await.clone();
        (status, Json(body))
    }
    async fn revoke(State(mock): State<Mock>, Json(body): Json<Value>) -> StatusCode {
        assert_eq!(body["token"], "private-issued-key");
        mock.revokes.fetch_add(1, Ordering::SeqCst);
        if mock.revoke_fails.load(Ordering::SeqCst) {
            StatusCode::BAD_GATEWAY
        } else {
            StatusCode::OK
        }
    }
    const IMMICH: &str = "https://trusted-immich.example/api";
    async fn fixture() -> (Arc<DeviceAuth>, Mock, tokio::task::JoinHandle<()>) {
        let (url, mock, task) = broker().await;
        (Arc::new(DeviceAuth::new(&url, IMMICH).unwrap()), mock, task)
    }
    /// A mock broker; returns its base URL.
    async fn broker() -> (String, Mock, tokio::task::JoinHandle<()>) {
        let mock = Mock {
            reply: Arc::new(Mutex::new((
                StatusCode::BAD_REQUEST,
                json!({"error":"authorization_pending"}),
            ))),
            polls: Arc::new(AtomicUsize::new(0)),
            codes: Arc::new(AtomicUsize::new(0)),
            revokes: Arc::new(AtomicUsize::new(0)),
            revoke_fails: Arc::new(false.into()),
        };
        let server = Router::new()
            .route("/device/code", post(code))
            .route("/device/token", post(token))
            .route("/device/revoke", post(revoke))
            .with_state(mock.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, server).await.unwrap();
        });
        (url, mock, task)
    }
    /// What a (re)started instance sees: a fresh `DeviceAuth` over the same state directory.
    fn boot(url: &str, dir: &TempDir, hq_key: Option<&str>) -> DeviceAuth {
        let auth = DeviceAuth::new(url, IMMICH).unwrap();
        let store = auth.store_in(&dir.0, hq_key);
        auth.with_store(store)
    }
    async fn pair(auth: &DeviceAuth, mock: &Mock) {
        auth.start().await.unwrap();
        *mock.reply.lock().await = (StatusCode::OK, issued());
        due(auth).await;
        assert_eq!(auth.poll().await.unwrap()["status"], "connected");
    }
    async fn token_of(auth: &DeviceAuth) -> Option<String> {
        auth.state.lock().await.grant.as_ref().map(|g| g.token.clone())
    }
    fn stored(dir: &TempDir) -> bool {
        dir.0.join(crate::grant_store::FILE_NAME).exists()
    }
    #[tokio::test]
    async fn grant_survives_a_restart_without_pairing_again() {
        let (url, mock, task) = broker().await;
        let dir = TempDir::new();
        let first = boot(&url, &dir, Some("k1"));
        pair(&first, &mock).await;
        let expires_at = first.status().await["expires_at"].clone();
        assert!(stored(&dir));
        drop(first);
        let second = boot(&url, &dir, Some("k1"));
        let status = second.status().await;
        assert_eq!(status["status"], "connected");
        assert_eq!(status["expires_at"], expires_at);
        assert_eq!(token_of(&second).await.as_deref(), Some("private-issued-key"));
        assert!(second.source().await.is_some());
        // The reloaded grant keeps its original lifetime rather than starting a new one.
        let left = second.state.lock().await.grant.as_ref().unwrap().deadline - Instant::now();
        assert!(left <= Duration::from_secs(3600));
        // No new pairing, so no new keys in Immich.
        assert_eq!(mock.codes.load(Ordering::SeqCst), 1);
        assert_eq!(second.start().await.unwrap()["status"], "connected");
        assert_eq!(mock.codes.load(Ordering::SeqCst), 1);
        task.abort();
    }
    #[tokio::test]
    async fn expired_stored_grants_are_discarded_on_load() {
        let (url, _, task) = broker().await;
        let dir = TempDir::new();
        let auth = boot(&url, &dir, Some("k1"));
        let store = auth.store_in(&dir.0, Some("k1")).unwrap();
        let fresh = Utc::now() + chrono::Duration::hours(1);
        let past = Utc::now() - chrono::Duration::seconds(1);
        for (expires_at, deadline) in [(past, fresh), (fresh, past)] {
            store
                .save(&StoredGrant {
                    token: "private-issued-key".into(),
                    expires_at,
                    deadline,
                    scope: SCOPE.into(),
                })
                .unwrap();
            let auth = boot(&url, &dir, Some("k1"));
            assert_eq!(auth.status().await["status"], "disconnected");
            assert!(auth.source().await.is_none());
            assert!(!stored(&dir));
        }
        // A stored grant whose scope no longer matches what Home HQ requests is not used either.
        store
            .save(&StoredGrant {
                token: "private-issued-key".into(),
                expires_at: fresh,
                deadline: fresh,
                scope: "asset.read asset.delete".into(),
            })
            .unwrap();
        assert_eq!(boot(&url, &dir, Some("k1")).status().await["status"], "disconnected");
        assert!(!stored(&dir));
        task.abort();
    }
    #[tokio::test]
    async fn a_grant_expiring_while_running_is_deleted_from_storage() {
        let (url, mock, task) = broker().await;
        let dir = TempDir::new();
        let auth = boot(&url, &dir, Some("k1"));
        pair(&auth, &mock).await;
        auth.state.lock().await.grant.as_mut().unwrap().deadline = Instant::now();
        assert_eq!(auth.status().await["status"], "expired");
        assert!(!stored(&dir));
        task.abort();
    }
    #[tokio::test]
    async fn a_changed_hq_key_discards_the_stored_grant() {
        let (url, mock, task) = broker().await;
        let dir = TempDir::new();
        pair(&boot(&url, &dir, Some("k1")), &mock).await;
        let auth = boot(&url, &dir, Some("k2"));
        assert_eq!(auth.status().await["status"], "disconnected");
        assert!(auth.source().await.is_none());
        assert!(!stored(&dir));
        // Discarded for good: going back to the old key doesn't bring it back.
        assert_eq!(boot(&url, &dir, Some("k1")).status().await["status"], "disconnected");
        // A different Immich server never receives a key stored for another one.
        pair(&boot(&url, &dir, Some("k1")), &mock).await;
        let elsewhere = DeviceAuth::new(&url, "https://other-immich.example/api").unwrap();
        let store = elsewhere.store_in(&dir.0, Some("k1"));
        assert_eq!(elsewhere.with_store(store).status().await["status"], "disconnected");
        assert!(!stored(&dir));
        task.abort();
    }
    #[tokio::test]
    async fn without_hq_key_nothing_is_persisted() {
        let (url, mock, task) = broker().await;
        let dir = TempDir::new();
        let auth = DeviceAuth::new(&url, IMMICH).unwrap();
        assert!(auth.store_in(&dir.0, None).is_none());
        assert!(auth.store_in(&dir.0, Some("")).is_none());
        let auth = boot(&url, &dir, None);
        pair(&auth, &mock).await;
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 0);
        assert_eq!(boot(&url, &dir, None).status().await["status"], "disconnected");
        task.abort();
    }
    #[tokio::test]
    async fn disconnect_revokes_then_clears_storage() {
        let (url, mock, task) = broker().await;
        let dir = TempDir::new();
        let auth = boot(&url, &dir, Some("k1"));
        pair(&auth, &mock).await;
        // A failed revocation keeps the grant, in memory and on disk, so it can be retried.
        mock.revoke_fails.store(true, Ordering::SeqCst);
        assert!(auth.disconnect().await.is_err());
        assert!(stored(&dir));
        assert_eq!(boot(&url, &dir, Some("k1")).status().await["status"], "connected");
        mock.revoke_fails.store(false, Ordering::SeqCst);
        assert_eq!(auth.disconnect().await.unwrap()["status"], "disconnected");
        assert_eq!(mock.revokes.load(Ordering::SeqCst), 2);
        assert!(!stored(&dir));
        assert_eq!(boot(&url, &dir, Some("k1")).status().await["status"], "disconnected");
        task.abort();
    }
    async fn due(auth: &DeviceAuth) {
        auth.state.lock().await.pending.as_mut().unwrap().next_poll = Instant::now();
    }
    fn issued() -> Value {
        json!({"token_type":"immich_api_key","access_token":"private-issued-key","expires_in":3600,
            "expires_at":Utc::now()+chrono::Duration::hours(1),"scope":SCOPE,"immich_url":"https://untrusted.example"})
    }
    #[tokio::test]
    async fn pairing_throttles_polls_and_keeps_credentials_private() {
        let (auth, mock, task) = fixture().await;
        let public = auth.start().await.unwrap();
        assert_eq!(public["status"], "pending");
        assert!(!public.to_string().contains("private-device-code"));
        let svg = auth.qr().await.unwrap().unwrap();
        assert!(svg.contains("<svg"));
        assert!(!svg.contains("script"));
        auth.poll().await.unwrap();
        assert_eq!(mock.polls.load(Ordering::SeqCst), 0);
        due(&auth).await;
        assert_eq!(auth.poll().await.unwrap()["status"], "pending");
        *mock.reply.lock().await = (StatusCode::BAD_REQUEST, json!({"error":"slow_down"}));
        due(&auth).await;
        assert_eq!(auth.poll().await.unwrap()["interval"], 10);
        *mock.reply.lock().await = (StatusCode::OK, issued());
        due(&auth).await;
        let public = auth.poll().await.unwrap();
        assert_eq!(public["status"], "connected");
        assert!(!public.to_string().contains("private-issued-key"));
        assert!(auth.source().await.is_some());
        assert!(auth.qr().await.unwrap().is_none());
        mock.revoke_fails.store(true, Ordering::SeqCst);
        assert!(auth.disconnect().await.is_err());
        assert!(auth.source().await.is_some());
        mock.revoke_fails.store(false, Ordering::SeqCst);
        assert_eq!(auth.disconnect().await.unwrap()["status"], "disconnected");
        assert!(auth.source().await.is_none());
        task.abort();
    }
    #[tokio::test]
    async fn denial_expiry_and_invalid_scopes_never_enable_photos() {
        let (auth, mock, task) = fixture().await;
        auth.start().await.unwrap();
        *mock.reply.lock().await = (StatusCode::BAD_REQUEST, json!({"error":"access_denied"}));
        due(&auth).await;
        assert_eq!(auth.poll().await.unwrap()["status"], "denied");
        auth.start().await.unwrap();
        auth.state.lock().await.pending.as_mut().unwrap().deadline = Instant::now();
        assert_eq!(auth.status().await["status"], "expired");
        auth.start().await.unwrap();
        let mut invalid = issued();
        invalid["scope"] = json!("asset.read");
        *mock.reply.lock().await = (StatusCode::OK, invalid);
        due(&auth).await;
        assert!(auth.poll().await.is_err());
        assert!(auth.source().await.is_none());
        *mock.reply.lock().await = (StatusCode::OK, issued());
        due(&auth).await;
        auth.poll().await.unwrap();
        auth.state.lock().await.grant.as_mut().unwrap().deadline = Instant::now();
        assert!(auth.source().await.is_none());
        assert_eq!(auth.status().await["status"], "expired");
        task.abort();
    }
    #[tokio::test]
    async fn routes_require_dashboard_auth_and_do_not_fall_back_to_demo() {
        let (auth, _, task) = fixture().await;
        let mut state = crate::AppState::new(
            crate::settings::Settings::parse(crate::settings::EXAMPLE).unwrap(),
            Some("dashboard-key".into()),
            Arc::new(Utc::now),
        );
        state.device_auth = Some(auth);
        let app = crate::router(state);
        for path in ["/api/immich/auth", "/api/immich/auth/qr"] {
            let response = app
                .clone()
                .oneshot(axum::http::Request::get(path).body(axum::body::Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::get("/api/state")
                    .header("x-hq-key", "dashboard-key")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let body: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["photos"], json!([]));
        assert_eq!(body["photoAuth"]["status"], "disconnected");
        // Requiring JSON also prevents cross-origin HTML forms from starting approval flows.
        let response = app
            .oneshot(
                axum::http::Request::post("/api/immich/auth")
                    .header("x-hq-key", "dashboard-key")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        task.abort();
    }
}
