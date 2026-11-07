//! Calendars added on the calendar page (`/calendars`), next to the read-only baseline in
//! `HQ_CALENDARS`.
//!
//! - **CalDAV accounts** are connected with Nextcloud's Login Flow v2 (Nextcloud, RustiCal: a
//!   QR code on the frame, sign in and approve on a phone, Home HQ receives an app password), or
//!   with a pasted app password for servers without the flow (Radicale, Baïkal). Discovery
//!   lists the account's event calendars. Each has its people, whether it shows, and whether the
//!   frame may add events to it.
//! - **ICS links** are added with their people.
//!
//! Accounts are kept in `$HQ_STATE_DIR/calendars.bin`, encrypted like the Immich grant
//! (AES-256-GCM, key derived from `HQ_KEY` with HKDF). Without `HQ_KEY` nothing is written; without
//! `HQ_STATE_DIR` they last until a restart. Secrets (app passwords, ICS links) never leave the
//! backend. An account's server can't be changed, only removed and added again, so a stored
//! password is only ever sent to the server it was made for. `HQ_CALENDAR_SERVERS` (a
//! comma-separated list of origins) limits which servers can be added.
//!
//! Every change rebuilds the calendar source behind [`Live`], so it applies without a restart.

use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    hkdf,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use super::caldav::config::CalDavConfig;
use super::caldav::dav::{DavClient, DavError, Transport, origin_of};
use super::caldav::login_flow::{self, Flow, Poll};
use super::fetch::Fetch;
use super::ics::{FeedConfig, People};
use super::mixed::{self, Extra};
use super::{CalendarError, CalendarSource, Event, Listing, NewEvent, Range};
use crate::{Clock, settings::Settings};

const MAGIC: &[u8; 4] = b"HQC1";
const SALT: &[u8] = b"home-hq calendar accounts v1";
const INFO: &[u8] = b"calendar accounts";
pub const FILE_NAME: &str = "calendars.bin";

/// How long a login flow waits for approval, as RustiCal keeps it.
pub const FLOW_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Caldav,
    Ics,
}

/// One account: a CalDAV login, or an ICS link.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub kind: Kind,
    /// Shown on the page: the server's host, or the ICS link's name.
    pub name: String,
    /// CalDAV: the address it was added with. ICS: empty (the link is the secret).
    pub server: String,
    pub username: Option<String>,
    /// The app password, or the ICS link. Never sent to the page.
    pub secret: String,
    /// "login flow", "app password" or "ics".
    pub via: String,
    pub added: DateTime<Utc>,
    pub calendars: Vec<Calendar>,
}

/// One calendar of an account (an ICS account has exactly one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub id: String,
    /// The collection URL (CalDAV); empty for ICS.
    pub url: String,
    pub name: String,
    /// Person ids, or `["all"]`.
    pub people: Vec<String>,
    pub show: bool,
    /// The frame may add events to it (CalDAV only).
    pub add_events: bool,
    /// What the server reported for these credentials.
    pub server_writable: bool,
}

/// What the page may change on a calendar.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Patch {
    pub people: Option<Vec<String>>,
    pub show: Option<bool>,
    pub add_events: Option<bool>,
}

/// A failure the page shows as is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub status: u16,
    pub message: String,
}

fn problem(status: u16, message: impl Into<String>) -> Problem {
    Problem {
        status,
        message: message.into(),
    }
}

fn dav_problem(e: DavError) -> Problem {
    match e {
        DavError::Unauthorized => problem(400, "the server refused that user name and app password"),
        DavError::NotFound => problem(400, "no CalDAV calendars at that address"),
        e => problem(502, e.to_string()),
    }
}

// ------------------------------------------------------------------ the encrypted file

/// `calendars.bin`: `MAGIC || nonce || AES-256-GCM(JSON accounts) || tag`.
pub struct Store {
    path: PathBuf,
    key: LessSafeKey,
    rng: SystemRandom,
}

impl Store {
    /// `None` without a dashboard key: accounts are never written unencrypted.
    pub fn new(dir: &Path, hq_key: Option<&str>) -> Option<Self> {
        let hq_key = hq_key.filter(|k| !k.is_empty())?;
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, SALT).extract(hq_key.as_bytes());
        let okm = prk.expand(&[INFO], &AES_256_GCM).expect("HKDF output fits AES-256-GCM");
        Some(Store {
            path: dir.join(FILE_NAME),
            key: LessSafeKey::new(UnboundKey::from(okm)),
            rng: SystemRandom::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The stored accounts; none when there is no file. `Err` when it can't be read or decrypted.
    pub fn load(&self) -> Result<Vec<Account>, String> {
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("can't read {}: {e}", FILE_NAME)),
        };
        let rest = data
            .strip_prefix(MAGIC.as_slice())
            .filter(|r| r.len() >= NONCE_LEN + AES_256_GCM.tag_len())
            .ok_or("not a calendars file")?;
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| "not a calendars file")?;
        let mut sealed = sealed.to_vec();
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(MAGIC), &mut sealed)
            .map_err(|_| "can't decrypt it (was HQ_KEY changed?)")?;
        serde_json::from_slice(plain).map_err(|_| "can't read it".to_string())
    }

    /// Writes atomically (temporary file, fsync, rename), readable by the owner only.
    pub fn save(&self, accounts: &[Account]) -> anyhow::Result<()> {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("no randomness for the nonce"))?;
        let mut sealed = serde_json::to_vec(accounts)?;
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(MAGIC), &mut sealed)
            .map_err(|_| anyhow::anyhow!("could not encrypt the calendars"))?;
        let tmp = self.path.with_extension("tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut file = options.open(&tmp)?;
        file.write_all(MAGIC)?;
        file.write_all(&nonce)?;
        file.write_all(&sealed)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

// ------------------------------------------------------------------ the live source

/// The calendar source the API uses; swapped whole when calendars change.
pub struct Live(RwLock<Arc<dyn CalendarSource>>);

impl Live {
    pub fn new(inner: Arc<dyn CalendarSource>) -> Self {
        Live(RwLock::new(inner))
    }

    fn get(&self) -> Arc<dyn CalendarSource> {
        self.0.read().unwrap().clone()
    }

    fn set(&self, inner: Arc<dyn CalendarSource>) {
        *self.0.write().unwrap() = inner;
    }
}

#[async_trait]
impl CalendarSource for Live {
    fn configured(&self) -> bool {
        self.get().configured()
    }

    fn can_write(&self) -> bool {
        self.get().can_write()
    }

    async fn list(&self, range: Range) -> Listing {
        self.get().list(range).await
    }

    async fn add(&self, new: NewEvent) -> Result<Event, CalendarError> {
        self.get().add(new).await
    }

    async fn remove(&self, id: &str) -> Result<(), CalendarError> {
        self.get().remove(id).await
    }
}

// ------------------------------------------------------------------ the manager

struct Pending {
    flow: Flow,
    server: String,
    deadline: Instant,
}

/// Owns the accounts, the login flow in progress, and the [`Live`] source.
pub struct Manager {
    settings: Settings,
    env: Option<String>,
    fetch: Arc<dyn Fetch>,
    transport: Arc<dyn Transport>,
    clock: Clock,
    store: Option<Store>,
    /// Why accounts aren't kept across restarts, for the page.
    store_note: Option<String>,
    /// `HQ_CALENDAR_SERVERS`: origins that may be added. `None`: any.
    allowed: Option<Vec<String>>,
    accounts: Mutex<Vec<Account>>,
    pending: Mutex<Option<Pending>>,
    /// The last outcome of a login flow, for the page ("expired", an error).
    outcome: Mutex<Option<Value>>,
    live: Arc<Live>,
    rng: SystemRandom,
}

/// Options for [`Manager::new`].
pub struct Config {
    pub settings: Settings,
    /// `HQ_CALENDARS`.
    pub env: Option<String>,
    pub fetch: Arc<dyn Fetch>,
    pub transport: Arc<dyn Transport>,
    pub clock: Clock,
    /// `HQ_STATE_DIR`.
    pub state_dir: Option<PathBuf>,
    /// `HQ_KEY`.
    pub hq_key: Option<String>,
    /// `HQ_CALENDAR_SERVERS`.
    pub allowed_servers: Option<String>,
}

impl Manager {
    pub fn new(c: Config) -> Self {
        let (store, store_note) = match (&c.state_dir, c.hq_key.as_deref().filter(|k| !k.is_empty())) {
            (Some(dir), Some(key)) => (Store::new(dir, Some(key)), None),
            (Some(_), None) => (
                None,
                Some("HQ_KEY is not set, so calendars added here are kept in memory only".to_string()),
            ),
            (None, _) => (
                None,
                Some("HQ_STATE_DIR is not set, so calendars added here last until a restart".to_string()),
            ),
        };
        let accounts = match store.as_ref().map(Store::load) {
            Some(Ok(a)) => a,
            Some(Err(e)) => {
                let store = store.as_ref().unwrap();
                let aside = store.path().with_extension("unreadable");
                tracing::error!(error = %e, kept = %aside.display(), "calendars file unusable; starting without it");
                let _ = std::fs::rename(store.path(), &aside);
                Vec::new()
            }
            None => Vec::new(),
        };
        let allowed = c.allowed_servers.as_deref().map(|v| {
            v.split(',')
                .map(|s| s.trim().trim_end_matches('/').to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        });
        let m = Manager {
            live: Arc::new(Live::new(Arc::new(super::NotConfigured))),
            settings: c.settings,
            env: c.env,
            fetch: c.fetch,
            transport: c.transport,
            clock: c.clock,
            store,
            store_note,
            allowed,
            accounts: Mutex::new(accounts.clone()),
            pending: Mutex::new(None),
            outcome: Mutex::new(None),
            rng: SystemRandom::new(),
        };
        m.rebuild(&accounts);
        m
    }

    /// The source to put in `AppState::calendar`.
    pub fn source(&self) -> Arc<dyn CalendarSource> {
        self.live.clone()
    }

    fn household(&self) -> Vec<String> {
        self.settings.people.iter().map(|p| p.id.clone()).collect()
    }

    fn rebuild(&self, accounts: &[Account]) {
        let mut extra = Extra::default();
        for a in accounts {
            for c in a.calendars.iter().filter(|c| c.show) {
                let people = people_value(&c.people);
                match a.kind {
                    Kind::Ics => extra.ics.push(FeedConfig {
                        url: a.secret.clone(),
                        people,
                        label: Some(c.name.clone()),
                    }),
                    Kind::Caldav => extra.caldav.push(CalDavConfig {
                        id: Some(c.id.clone()),
                        url: c.url.clone(),
                        people,
                        username: a.username.clone(),
                        password: Some(a.secret.clone()),
                        read_only: !c.add_events,
                        number: 0,
                        label: Some(c.name.clone()),
                    }),
                }
            }
        }
        let source = mixed::build(
            self.env.as_deref(),
            extra,
            &self.settings,
            self.fetch.clone(),
            self.transport.clone(),
            self.clock.clone(),
        );
        self.live.set(source);
    }

    /// Saves, then rebuilds the source.
    fn commit(&self, accounts: &[Account]) -> Result<(), Problem> {
        if let Some(store) = &self.store {
            store.save(accounts).map_err(|e| {
                tracing::error!(error = %e, "saving calendars");
                problem(500, "couldn't save the calendars")
            })?;
        }
        self.rebuild(accounts);
        Ok(())
    }

    fn new_id(&self) -> String {
        let mut b = [0u8; 6];
        let _ = self.rng.fill(&mut b);
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// Checks an address typed on the page: http(s), no credentials in it, and allowed.
    fn server(&self, input: &str) -> Result<String, Problem> {
        let s = input.trim();
        let s = if s.contains("://") {
            s.to_string()
        } else {
            format!("https://{s}")
        };
        let lower = s.to_ascii_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("http://")) {
            return Err(problem(400, "the address must start with https://"));
        }
        let rest = &s[s.find("://").unwrap() + 3..];
        let host = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
        if host.is_empty() || host.contains('@') || host.contains(char::is_whitespace) {
            return Err(problem(
                400,
                "that isn't a server address (and passwords don't go in it)",
            ));
        }
        let s = s.trim_end_matches('/').to_string();
        let origin = origin_of(&format!("{s}/")).to_ascii_lowercase();
        if let Some(allowed) = &self.allowed
            && !allowed.contains(&origin)
        {
            return Err(problem(403, "this server isn't in HQ_CALENDAR_SERVERS"));
        }
        Ok(s)
    }

    /// Guesses whose a calendar is from its name: a person's name, else everyone.
    fn guess_people(&self, name: &str) -> Vec<String> {
        let n = name.to_lowercase();
        let hits: Vec<String> = self
            .settings
            .people
            .iter()
            .filter(|p| {
                n.split(|c: char| !c.is_alphanumeric())
                    .any(|w| w == p.name.to_lowercase())
            })
            .map(|p| p.id.clone())
            .collect();
        if hits.is_empty() { vec!["all".into()] } else { hits }
    }

    fn checked_people(&self, people: &[String]) -> Result<Vec<String>, Problem> {
        if people.is_empty() || people.iter().any(|p| p == "all") {
            return Ok(vec!["all".into()]);
        }
        let house = self.household();
        if let Some(p) = people.iter().find(|p| !house.contains(p)) {
            return Err(problem(400, format!("unknown person {p}")));
        }
        Ok(people.to_vec())
    }

    async fn discover(&self, server: &str, user: &str, password: &str) -> Result<Vec<Calendar>, Problem> {
        let client = DavClient::new(self.transport.clone(), Some(user), Some(password));
        let found = client.calendars(server).await.map_err(dav_problem)?;
        if found.is_empty() {
            return Err(problem(400, "signed in, but there are no event calendars there"));
        }
        let mut cals: Vec<Calendar> = found
            .into_iter()
            .map(|c| {
                let name = c.name.clone().unwrap_or_else(|| last_segment(&c.url));
                Calendar {
                    id: self.new_id(),
                    people: self.guess_people(&name),
                    name,
                    url: c.url,
                    show: true,
                    add_events: false,
                    server_writable: c.writable,
                }
            })
            .collect();
        cals.sort_by_key(|c| c.name.to_lowercase());
        Ok(cals)
    }

    async fn add_caldav(&self, server: String, user: String, password: String, via: &str) -> Result<Value, Problem> {
        let calendars = self.discover(&server, &user, &password).await?;
        let account = Account {
            id: self.new_id(),
            kind: Kind::Caldav,
            name: host_of(&server),
            server,
            username: Some(user),
            secret: password,
            via: via.into(),
            added: (self.clock)(),
            calendars,
        };
        let view = public(&account);
        let mut accounts = self.accounts.lock().await;
        accounts.push(account);
        if let Err(e) = self.commit(&accounts) {
            accounts.pop();
            return Err(e);
        }
        tracing::info!(via, "calendar account added");
        Ok(view)
    }

    /// The page's view: accounts without secrets, the people, the login in progress.
    pub async fn view(&self) -> Value {
        let accounts = self.accounts.lock().await;
        let env = super::caldav::config::split(self.env.as_deref());
        let env_count = env.caldav.len()
            + env
                .ics
                .as_deref()
                .and_then(|v| serde_json::from_str::<Vec<Value>>(v).ok())
                .map_or(0, |v| v.len());
        let pending = self.pending.lock().await;
        json!({
            "accounts": accounts.iter().map(public).collect::<Vec<_>>(),
            "people": self.settings.people(false).iter().map(|p| json!({ "id": p.id, "name": p.name, "color": p.color })).collect::<Vec<_>>(),
            "fromEnv": env_count,
            "persisted": self.store.is_some(),
            "persistNote": self.store_note,
            "login": pending.as_ref().map(|p| json!({ "url": p.flow.login, "server": host_of(&p.server) })),
            "outcome": *self.outcome.lock().await,
        })
    }

    /// Starts a Login Flow v2 on `server`, replacing one in progress.
    pub async fn start_login(&self, server: &str) -> Result<Value, Problem> {
        let server = self.server(server)?;
        let origin = origin_of(&format!("{server}/")).to_string();
        let flow = login_flow::start(self.transport.as_ref(), &origin)
            .await
            .map_err(|e| match e {
                DavError::NotFound => problem(
                    400,
                    "this server doesn't offer the Nextcloud login flow: add it with an app password instead",
                ),
                e => dav_problem(e),
            })?;
        let view = json!({ "url": flow.login, "server": host_of(&server) });
        *self.pending.lock().await = Some(Pending {
            flow,
            server,
            deadline: Instant::now() + FLOW_TTL,
        });
        *self.outcome.lock().await = None;
        Ok(view)
    }

    /// The login page as a QR code, for a phone.
    pub async fn login_qr(&self) -> Option<String> {
        let pending = self.pending.lock().await;
        let p = pending.as_ref()?;
        qrcode::QrCode::new(p.flow.login.as_bytes()).ok().map(|q| {
            q.render::<qrcode::render::svg::Color>()
                .min_dimensions(320, 320)
                .build()
        })
    }

    /// Checks the login in progress once: `waiting`, `connected` (with the account), `expired`,
    /// or `none`.
    pub async fn poll_login(&self) -> Result<Value, Problem> {
        let mut pending = self.pending.lock().await;
        let Some(p) = pending.as_ref() else {
            return Ok(json!({ "status": "none" }));
        };
        if Instant::now() > p.deadline {
            *pending = None;
            let out = json!({ "status": "expired" });
            *self.outcome.lock().await = Some(out.clone());
            return Ok(out);
        }
        match login_flow::poll(self.transport.as_ref(), &p.flow).await {
            Ok(Poll::Waiting) => Ok(json!({ "status": "waiting" })),
            Ok(Poll::Gone) => {
                *pending = None;
                let out = json!({ "status": "expired" });
                *self.outcome.lock().await = Some(out.clone());
                Ok(out)
            }
            Ok(Poll::Approved {
                login_name,
                app_password,
            }) => {
                let server = p.server.clone();
                *pending = None;
                drop(pending);
                match self.add_caldav(server, login_name, app_password, "login flow").await {
                    Ok(account) => Ok(json!({ "status": "connected", "account": account })),
                    Err(e) => {
                        *self.outcome.lock().await = Some(json!({ "status": "failed", "error": e.message }));
                        Err(e)
                    }
                }
            }
            Err(e) => Err(dav_problem(e)),
        }
    }

    pub async fn cancel_login(&self) {
        *self.pending.lock().await = None;
        *self.outcome.lock().await = None;
    }

    /// Adds a CalDAV account with an app password (servers without the login flow).
    pub async fn add_with_password(&self, server: &str, user: &str, password: &str) -> Result<Value, Problem> {
        let server = self.server(server)?;
        let (user, password) = (user.trim(), password.trim());
        if user.is_empty() || password.is_empty() {
            return Err(problem(400, "user name and app password are both needed"));
        }
        self.add_caldav(server, user.into(), password.into(), "app password")
            .await
    }

    /// Adds an ICS link.
    pub async fn add_ics(&self, url: &str, name: &str, people: &[String]) -> Result<Value, Problem> {
        let url = url.trim().replacen("webcal://", "https://", 1);
        let checked = self.server(&url)?;
        if !url.contains("://") || path_of(&checked).trim_matches('/').is_empty() {
            return Err(problem(400, "that's a server, not a calendar link"));
        }
        let name = match name.trim() {
            "" => host_of(&checked),
            n => n.chars().take(60).collect(),
        };
        let people = self.checked_people(people)?;
        let account = Account {
            id: self.new_id(),
            kind: Kind::Ics,
            name: name.clone(),
            server: String::new(),
            username: None,
            secret: url,
            via: "ics".into(),
            added: (self.clock)(),
            calendars: vec![Calendar {
                id: self.new_id(),
                url: String::new(),
                name,
                people,
                show: true,
                add_events: false,
                server_writable: false,
            }],
        };
        let view = public(&account);
        let mut accounts = self.accounts.lock().await;
        accounts.push(account);
        if let Err(e) = self.commit(&accounts) {
            accounts.pop();
            return Err(e);
        }
        Ok(view)
    }

    /// Changes a calendar's people, visibility or whether the frame adds events to it.
    pub async fn update(&self, account: &str, calendar: &str, patch: Patch) -> Result<Value, Problem> {
        let people = patch.people.as_deref().map(|p| self.checked_people(p)).transpose()?;
        let mut accounts = self.accounts.lock().await;
        let before = accounts.clone();
        let a = accounts
            .iter_mut()
            .find(|a| a.id == account)
            .ok_or_else(|| problem(404, "no such account"))?;
        let kind = a.kind;
        let c = a
            .calendars
            .iter_mut()
            .find(|c| c.id == calendar)
            .ok_or_else(|| problem(404, "no such calendar"))?;
        if let Some(p) = people {
            c.people = p;
        }
        if let Some(s) = patch.show {
            c.show = s;
        }
        if let Some(w) = patch.add_events {
            if w && kind == Kind::Ics {
                return Err(problem(400, "events can't be added to an ICS link"));
            }
            if w && !c.server_writable {
                return Err(problem(400, "the server doesn't let this account add events here"));
            }
            c.add_events = w;
        }
        let view = public(a);
        if let Err(e) = self.commit(&accounts) {
            *accounts = before;
            return Err(e);
        }
        Ok(view)
    }

    /// Looks for calendars added or removed on the server, keeping the choices for known ones.
    pub async fn refresh(&self, account: &str) -> Result<Value, Problem> {
        let (server, user, password) = {
            let accounts = self.accounts.lock().await;
            let a = accounts
                .iter()
                .find(|a| a.id == account && a.kind == Kind::Caldav)
                .ok_or_else(|| problem(404, "no such CalDAV account"))?;
            (
                a.server.clone(),
                a.username.clone().unwrap_or_default(),
                a.secret.clone(),
            )
        };
        let found = self.discover(&server, &user, &password).await?;
        let mut accounts = self.accounts.lock().await;
        let before = accounts.clone();
        let a = accounts
            .iter_mut()
            .find(|a| a.id == account)
            .ok_or_else(|| problem(404, "no such account"))?;
        a.calendars = found
            .into_iter()
            .map(|mut c| {
                if let Some(old) = a.calendars.iter().find(|o| o.url == c.url) {
                    c.id = old.id.clone();
                    c.people = old.people.clone();
                    c.show = old.show;
                    c.add_events = old.add_events && c.server_writable;
                }
                c
            })
            .collect();
        let view = public(a);
        if let Err(e) = self.commit(&accounts) {
            *accounts = before;
            return Err(e);
        }
        Ok(view)
    }

    /// Forgets an account and its secret. (For a login-flow account, revoke the app password
    /// on the server too.)
    pub async fn remove(&self, account: &str) -> Result<(), Problem> {
        let mut accounts = self.accounts.lock().await;
        let before = accounts.clone();
        let n = accounts.len();
        accounts.retain(|a| a.id != account);
        if accounts.len() == n {
            return Err(problem(404, "no such account"));
        }
        if let Err(e) = self.commit(&accounts) {
            *accounts = before;
            return Err(e);
        }
        Ok(())
    }
}

fn people_value(people: &[String]) -> People {
    if people.is_empty() || people.iter().any(|p| p == "all") {
        People::One("all".into())
    } else {
        People::Many(people.to_vec())
    }
}

/// An account as the page sees it: no app password, no ICS link.
fn public(a: &Account) -> Value {
    json!({
        "id": a.id,
        "kind": a.kind,
        "name": a.name,
        "server": a.server,
        "username": a.username,
        "via": a.via,
        "added": a.added.to_rfc3339(),
        "calendars": a.calendars.iter().map(|c| json!({
            "id": c.id,
            "name": c.name,
            "people": c.people,
            "show": c.show,
            "addEvents": c.add_events,
            "serverWritable": c.server_writable,
        })).collect::<Vec<_>>(),
    })
}

fn host_of(url: &str) -> String {
    let o = origin_of(&format!("{}/", url.trim_end_matches('/'))).to_string();
    o.split_once("://").map_or(o.clone(), |(_, h)| h.to_string())
}

fn path_of(url: &str) -> &str {
    let o = origin_of(url);
    &url[o.len()..]
}

fn last_segment(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("calendar")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir() -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "home-hq-cal-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn account() -> Account {
        Account {
            id: "a1".into(),
            kind: Kind::Caldav,
            name: "dav.example".into(),
            server: "https://dav.example".into(),
            username: Some("alex".into()),
            secret: "very-secret-app-password".into(),
            via: "login flow".into(),
            added: DateTime::from_timestamp(1_794_000_000, 0).unwrap(),
            calendars: vec![],
        }
    }

    #[test]
    fn stores_encrypted_and_only_with_a_key() {
        let d = dir();
        assert!(Store::new(&d, None).is_none());
        assert!(Store::new(&d, Some("")).is_none());
        let store = Store::new(&d, Some("k1")).unwrap();
        assert_eq!(store.load(), Ok(vec![]));
        store.save(&[account()]).unwrap();
        let raw = std::fs::read(store.path()).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("very-secret"));
        assert_eq!(store.load().unwrap(), vec![account()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(store.path()).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Another key can't read it.
        assert!(Store::new(&d, Some("k2")).unwrap().load().is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hosts_and_paths() {
        assert_eq!(host_of("https://dav.example:8443/caldav/"), "dav.example:8443");
        assert_eq!(path_of("https://dav.example/a/b.ics"), "/a/b.ics");
        assert_eq!(last_segment("https://d/caldav/principal/alex/family/"), "family");
    }
}
