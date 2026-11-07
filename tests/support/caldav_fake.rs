//! Test transports for the CalDAV adapter: an in-memory CalDAV server, and replay of responses
//! recorded from a real one (Radicale). No network.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use home_hq::calendar::caldav::dav::{HttpRequest, HttpResponse, Transport};
use serde::{Deserialize, Serialize};

pub const ORIGIN: &str = "https://dav.example";

#[derive(Debug, Clone)]
pub struct Collection {
    pub objects: BTreeMap<String, (String, String)>, // name -> (etag, ics)
    pub components: Vec<&'static str>,
    pub writable: bool,
    /// Answer 500 to everything.
    pub broken: bool,
    /// Answer 500 to PUT and DELETE only.
    pub broken_writes: bool,
    pub name: String,
}

impl Collection {
    pub fn events(name: &str) -> Self {
        Collection {
            objects: BTreeMap::new(),
            components: vec!["VEVENT"],
            writable: true,
            broken: false,
            broken_writes: false,
            name: name.into(),
        }
    }
}

/// An in-memory CalDAV server: one account (`/alex/`) holding collections by path.
pub struct FakeDav {
    pub collections: Mutex<BTreeMap<String, Collection>>,
    pub authorization: Option<String>,
    pub log: Mutex<Vec<HttpRequest>>,
    etags: Mutex<u64>,
    /// Nextcloud Login Flow v2: `None` until started, then whether the phone has approved.
    pub login: Mutex<Option<bool>>,
    /// The app password the login flow hands out (it must match `authorization`).
    pub app_password: Mutex<String>,
}

impl FakeDav {
    /// `authorization`: the `Authorization` header every request must carry.
    pub fn new(authorization: Option<&str>) -> Self {
        FakeDav {
            collections: Mutex::new(BTreeMap::new()),
            authorization: authorization.map(String::from),
            log: Mutex::new(Vec::new()),
            etags: Mutex::new(0),
            login: Mutex::new(None),
            app_password: Mutex::new("secret-pw".into()),
        }
    }

    /// The phone approves the login in progress.
    pub fn approve(&self) {
        let mut l = self.login.lock().unwrap();
        if l.is_some() {
            *l = Some(true);
        }
    }

    pub fn url(path: &str) -> String {
        format!("{ORIGIN}{path}")
    }

    pub fn add_collection(&self, path: &str, c: Collection) {
        self.collections.lock().unwrap().insert(path.into(), c);
    }

    fn next_etag(&self) -> String {
        let mut n = self.etags.lock().unwrap();
        *n += 1;
        format!("\"etag-{n}\"")
    }

    /// Stores each event of `ics` (grouped by UID) as its own object, as CalDAV requires.
    pub fn seed(&self, path: &str, ics: &str) {
        for (uid, obj) in split_objects(ics) {
            let etag = self.next_etag();
            let name = format!("{}.ics", uid.replace('@', "-"));
            self.collections
                .lock()
                .unwrap()
                .get_mut(path)
                .unwrap()
                .objects
                .insert(name, (etag, obj));
        }
    }

    /// Replaces an object's body behind the adapter's back (a phone edited it), changing its ETag.
    pub fn touch(&self, path: &str, name: &str) {
        let etag = self.next_etag();
        let mut cols = self.collections.lock().unwrap();
        let obj = cols.get_mut(path).unwrap().objects.get_mut(name).unwrap();
        obj.0 = etag;
    }

    pub fn object(&self, path: &str, name: &str) -> Option<(String, String)> {
        self.collections.lock().unwrap().get(path)?.objects.get(name).cloned()
    }

    pub fn requests(&self, method: &str) -> Vec<HttpRequest> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.method == method)
            .cloned()
            .collect()
    }

    fn handle(&self, req: &HttpRequest) -> HttpResponse {
        let resp = |status: u16, body: String| HttpResponse {
            status,
            body,
            ..Default::default()
        };
        // Login Flow v2 needs no credentials (the person signs in on their phone).
        let p = req.url.strip_prefix(ORIGIN).unwrap_or_default();
        if req.method == "POST" && p == "/index.php/login/v2" {
            *self.login.lock().unwrap() = Some(false);
            return resp(
                200,
                format!(
                    r#"{{"login":"{ORIGIN}/index.php/login/v2/flow/f1","poll":{{"token":"tok-1","endpoint":"{ORIGIN}/index.php/login/v2/poll/f1"}}}}"#
                ),
            );
        }
        if req.method == "POST" && p == "/index.php/login/v2/poll/f1" {
            let mut l = self.login.lock().unwrap();
            return match (*l, req.body.as_deref()) {
                (Some(true), Some("token=tok-1")) => {
                    *l = None;
                    resp(
                        200,
                        format!(
                            r#"{{"server":"{ORIGIN}","loginName":"alex","appPassword":"{}"}}"#,
                            self.app_password.lock().unwrap()
                        ),
                    )
                }
                (Some(false), Some("token=tok-1")) => resp(404, String::new()),
                _ => resp(401, String::new()),
            };
        }
        if self.authorization.as_deref() != req.header("Authorization") {
            return resp(401, "Unauthorized".into());
        }
        if p == "/.well-known/caldav" {
            return HttpResponse {
                status: 301,
                location: Some("/alex/".into()),
                ..Default::default()
            };
        }
        let path = req
            .url
            .strip_prefix(ORIGIN)
            .expect("only the fake's origin")
            .to_string();
        let mut cols = self.collections.lock().unwrap();
        // The account (principal and calendar home).
        if path == "/alex/" && req.method == "PROPFIND" {
            let mut body = String::from(
                r#"<?xml version="1.0"?><multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
            );
            body.push_str(r#"<response><href>/alex/</href><propstat><prop><resourcetype><collection/><principal/></resourcetype><current-user-principal><href>/alex/</href></current-user-principal><C:calendar-home-set><href>/alex/</href></C:calendar-home-set></prop><status>HTTP/1.1 200 OK</status></propstat></response>"#);
            if req.header("Depth") == Some("1") {
                for (p, c) in cols.iter() {
                    body.push_str(&collection_xml(p, c));
                }
            }
            body.push_str("</multistatus>");
            return resp(207, body);
        }
        let (col_path, name) = match path.rfind('/') {
            Some(i) if i + 1 < path.len() => (path[..=i].to_string(), Some(path[i + 1..].to_string())),
            _ => (path.clone(), None),
        };
        let Some(col) = cols.get_mut(&col_path) else {
            return resp(404, "Not Found".into());
        };
        if col.broken {
            return resp(500, "Internal Server Error".into());
        }
        match (req.method, name) {
            ("PROPFIND", None) => resp(
                207,
                format!(
                    r#"<?xml version="1.0"?><multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">{}</multistatus>"#,
                    collection_xml(&col_path, col)
                ),
            ),
            ("PROPFIND", Some(name)) => match col.objects.get(&name) {
                Some((etag, _)) => resp(
                    207,
                    format!(
                        r#"<?xml version="1.0"?><multistatus xmlns="DAV:"><response><href>{col_path}{name}</href><propstat><prop><getetag>{}</getetag></prop><status>HTTP/1.1 200 OK</status></propstat></response></multistatus>"#,
                        xml_escape(etag)
                    ),
                ),
                None => resp(404, "Not Found".into()),
            },
            ("REPORT", None) => {
                let mut body = String::from(
                    r#"<?xml version="1.0"?><multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">"#,
                );
                for (name, (etag, ics)) in &col.objects {
                    body.push_str(&format!(
                        r#"<response><href>{col_path}{name}</href><propstat><prop><getetag>{}</getetag><C:calendar-data>{}</C:calendar-data></prop><status>HTTP/1.1 200 OK</status></propstat></response>"#,
                        xml_escape(etag),
                        xml_escape(ics)
                    ));
                }
                body.push_str("</multistatus>");
                resp(207, body)
            }
            ("PUT", Some(name)) => {
                if col.broken_writes {
                    return resp(500, "Internal Server Error".into());
                }
                if !col.writable {
                    return resp(403, "Forbidden".into());
                }
                if req.header("If-None-Match") == Some("*") && col.objects.contains_key(&name) {
                    return resp(412, "Precondition Failed".into());
                }
                drop(cols);
                let etag = self.next_etag();
                let mut cols = self.collections.lock().unwrap();
                let col = cols.get_mut(&col_path).unwrap();
                col.objects
                    .insert(name, (etag.clone(), req.body.clone().unwrap_or_default()));
                HttpResponse {
                    status: 201,
                    etag: Some(etag),
                    ..Default::default()
                }
            }
            ("DELETE", Some(name)) => {
                if col.broken_writes {
                    return resp(500, "Internal Server Error".into());
                }
                let Some((etag, _)) = col.objects.get(&name) else {
                    return resp(404, "Not Found".into());
                };
                if req.header("If-Match").is_some_and(|m| m != etag) {
                    return resp(412, "Precondition Failed".into());
                }
                col.objects.remove(&name);
                resp(204, String::new())
            }
            _ => resp(405, "Method Not Allowed".into()),
        }
    }
}

fn collection_xml(path: &str, c: &Collection) -> String {
    let comps: String = c
        .components
        .iter()
        .map(|n| format!(r#"<C:comp name="{n}"/>"#))
        .collect();
    let privs = if c.writable {
        "<privilege><read/></privilege><privilege><write/></privilege>"
    } else {
        "<privilege><read/></privilege>"
    };
    format!(
        r#"<response><href>{path}</href><propstat><prop><resourcetype><collection/><C:calendar/></resourcetype><displayname>{}</displayname><current-user-privilege-set>{privs}</current-user-privilege-set><C:supported-calendar-component-set>{comps}</C:supported-calendar-component-set><current-user-principal><href>/alex/</href></current-user-principal></prop><status>HTTP/1.1 200 OK</status></propstat><propstat><prop><C:calendar-home-set/></prop><status>HTTP/1.1 404 Not Found</status></propstat></response>"#,
        xml_escape(&c.name)
    )
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[async_trait]
impl Transport for FakeDav {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        self.log.lock().unwrap().push(req.clone());
        Ok(self.handle(&req))
    }
}

/// Splits an iCalendar document into one document per `UID` (each with the calendar's
/// `VTIMEZONE`s), the way a CalDAV server stores it.
pub fn split_objects(ics: &str) -> Vec<(String, String)> {
    let text = ics.replace("\r\n", "\n");
    let mut header = Vec::new();
    let mut timezones = Vec::new();
    let mut events: Vec<(String, Vec<String>)> = Vec::new();
    let mut current: Option<(String, Vec<String>)> = None;
    let mut depth = 0;
    for line in text.lines() {
        if line == "BEGIN:VCALENDAR" || line == "END:VCALENDAR" {
            continue;
        }
        if depth == 0 && line.starts_with("BEGIN:") {
            current = Some((line[6..].to_string(), Vec::new()));
        }
        if line.starts_with("BEGIN:") {
            depth += 1;
        }
        match current.as_mut() {
            Some((_, lines)) => lines.push(line.to_string()),
            None => header.push(line.to_string()),
        }
        if line.starts_with("END:") {
            depth -= 1;
            if depth == 0
                && let Some((kind, lines)) = current.take()
            {
                if kind == "VTIMEZONE" {
                    timezones.extend(lines);
                } else {
                    let uid = lines
                        .iter()
                        .find_map(|l| l.strip_prefix("UID:"))
                        .unwrap_or("no-uid")
                        .to_string();
                    match events.iter_mut().find(|(u, _)| *u == uid) {
                        Some((_, ls)) => ls.extend(lines),
                        None => events.push((uid, lines)),
                    }
                }
            }
        }
    }
    events
        .into_iter()
        .map(|(uid, lines)| {
            let mut doc = vec!["BEGIN:VCALENDAR".to_string()];
            doc.extend(header.iter().cloned());
            doc.extend(timezones.iter().cloned());
            doc.extend(lines);
            doc.push("END:VCALENDAR".into());
            (uid, doc.join("\r\n") + "\r\n")
        })
        .collect()
}

/// One recorded request and its response. Only the parts the adapter depends on are kept.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Exchange {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_match: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_none_match: Option<String>,
    pub status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub body: String,
}

fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.find('/').map_or("/".into(), |i| rest[i..].to_string())
}

impl Exchange {
    pub fn of(req: &HttpRequest, res: &HttpResponse) -> Self {
        Exchange {
            method: req.method.into(),
            path: path_of(&req.url),
            depth: req.header("Depth").map(String::from),
            if_match: req.header("If-Match").map(String::from),
            if_none_match: req.header("If-None-Match").map(String::from),
            status: res.status,
            etag: res.etag.clone(),
            location: res.location.as_deref().map(path_of),
            body: res.body.clone(),
        }
    }

    fn answers(&self, req: &HttpRequest) -> bool {
        self.method == req.method
            && self.path == path_of(&req.url)
            && self.depth.as_deref() == req.header("Depth")
            && self.if_match.as_deref() == req.header("If-Match")
            && self.if_none_match.as_deref() == req.header("If-None-Match")
    }
}

/// Answers from a recording: each request takes the first unused exchange that matches it.
pub struct Replay {
    exchanges: Mutex<Vec<Option<Exchange>>>,
}

impl Replay {
    pub fn new(exchanges: Vec<Exchange>) -> Self {
        Replay {
            exchanges: Mutex::new(exchanges.into_iter().map(Some).collect()),
        }
    }

    pub fn unused(&self) -> usize {
        self.exchanges.lock().unwrap().iter().flatten().count()
    }
}

#[async_trait]
impl Transport for Replay {
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
        let mut ex = self.exchanges.lock().unwrap();
        let slot = ex
            .iter_mut()
            .find(|e| e.as_ref().is_some_and(|e| e.answers(&req)))
            .unwrap_or_else(|| panic!("no recorded answer for {} {}", req.method, path_of(&req.url)));
        let e = slot.take().unwrap();
        Ok(HttpResponse {
            status: e.status,
            etag: e.etag,
            location: e.location,
            body: e.body,
        })
    }
}
