//! The WebDAV/CalDAV requests the adapter needs, written by hand over an injected HTTP transport.
//!
//! Only four requests are used: `PROPFIND` (discovery and ETags), a `calendar-query` `REPORT`
//! with a time range, `PUT` of a new object, and `DELETE` with `If-Match`. Errors never carry a
//! URL: calendar URLs are secret and errors end up in `/api/calendar` and the logs.

use std::{fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};

const DAV: &str = "DAV:";
const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";

/// One HTTP request, as plain data so a test transport can check it and answer from a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub etag: Option<String>,
    pub body: String,
}

/// The HTTP client, injected so tests never touch the network.
#[async_trait]
pub trait Transport: Send + Sync {
    /// Sends the request. `Err` is a failure to get any response (DNS, TLS, timeout), worded
    /// without the URL.
    async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DavError {
    /// No response at all.
    Unreachable(String),
    /// 401/403: wrong or missing credentials, or no permission.
    Unauthorized,
    /// 412: the object changed on the server since we last saw it (ETag mismatch), or it
    /// already exists when we tried to create it.
    Conflict,
    NotFound,
    /// Any other unexpected status.
    Status(u16),
    /// A response we couldn't make sense of.
    BadResponse(String),
    /// The configured URL is not a calendar collection, and discovery found `found` event
    /// calendars under it (so it couldn't pick one).
    NotACalendar {
        found: usize,
    },
    /// The calendar exists but doesn't hold events (a task list, for example).
    NoEvents,
}

impl fmt::Display for DavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The cause (DNS, TLS, timeout) is in the logs: too long for a toast.
            DavError::Unreachable(_) => f.write_str("calendar server not reachable"),
            DavError::Unauthorized => f.write_str("the calendar server refused the credentials"),
            DavError::Conflict => f.write_str("the event changed on the calendar server, try again"),
            DavError::NotFound => f.write_str("calendar not found on the server"),
            DavError::Status(s @ 300..400) => {
                write!(f, "the calendar moved (HTTP {s}): use its final URL in HQ_CALENDARS")
            }
            DavError::Status(s) => write!(f, "calendar server error (HTTP {s})"),
            DavError::BadResponse(e) => write!(f, "unexpected answer from the calendar server ({e})"),
            DavError::NotACalendar { found: 0 } => {
                f.write_str("the URL is not a calendar and has no calendars under it")
            }
            DavError::NotACalendar { found } => {
                write!(
                    f,
                    "the URL is not a calendar but has {found} calendars under it: use one of their URLs"
                )
            }
            DavError::NoEvents => f.write_str("the calendar doesn't hold events"),
        }
    }
}

impl std::error::Error for DavError {}

fn check(res: &HttpResponse, ok: &[u16]) -> Result<(), DavError> {
    match res.status {
        s if ok.contains(&s) => Ok(()),
        401 | 403 => Err(DavError::Unauthorized),
        404 | 410 => Err(DavError::NotFound),
        409 | 412 => Err(DavError::Conflict),
        s => Err(DavError::Status(s)),
    }
}

/// One `<response>` of a multistatus, keeping only the properties found with a 2xx propstat.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DavResource {
    pub href: String,
    pub etag: Option<String>,
    pub calendar_data: Option<String>,
    pub displayname: Option<String>,
    /// `resourcetype` contains `C:calendar`.
    pub is_calendar: bool,
    /// `supported-calendar-component-set`, e.g. `["VEVENT", "VTODO"]`. `None` when not reported.
    pub components: Option<Vec<String>>,
    /// `current-user-privilege-set`, local names such as `read`, `write`, `all`. `None` when not
    /// reported.
    pub privileges: Option<Vec<String>>,
    pub principal: Option<String>,
    pub calendar_home: Option<String>,
}

impl DavResource {
    pub fn holds_events(&self) -> bool {
        self.components
            .as_ref()
            .is_none_or(|c| c.iter().any(|c| c.eq_ignore_ascii_case("VEVENT")))
    }

    /// Whether new objects can be added. Servers that don't report privileges are assumed to
    /// allow it; a failed write then reports its own error.
    pub fn writable(&self) -> bool {
        self.privileges.as_ref().is_none_or(|p| {
            p.iter()
                .any(|p| matches!(p.as_str(), "all" | "write" | "write-content" | "bind"))
        })
    }
}

/// Parses a `207 Multi-Status` body.
pub fn parse_multistatus(xml: &str) -> Result<Vec<DavResource>, DavError> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| DavError::BadResponse(format!("not XML: {e}")))?;
    let root = doc.root_element();
    if !is(root, DAV, "multistatus") {
        return Err(DavError::BadResponse("not a multistatus".into()));
    }
    let mut out = Vec::new();
    for resp in root.children().filter(|n| is(*n, DAV, "response")) {
        let Some(href) = child(resp, DAV, "href").map(text) else {
            continue;
        };
        let mut r = DavResource {
            href,
            ..Default::default()
        };
        for ps in resp.children().filter(|n| is(*n, DAV, "propstat")) {
            let ok = child(ps, DAV, "status").map(text).is_none_or(|s| status_ok(&s));
            let Some(prop) = child(ps, DAV, "prop").filter(|_| ok) else {
                continue;
            };
            for p in prop.children().filter(|n| n.is_element()) {
                let ns = p.tag_name().namespace();
                match (ns, p.tag_name().name()) {
                    (Some(DAV), "getetag") => r.etag = Some(text(p)).filter(|s| !s.is_empty()),
                    (Some(DAV), "displayname") => r.displayname = Some(text(p)).filter(|s| !s.is_empty()),
                    (Some(DAV), "resourcetype") => r.is_calendar = child(p, CALDAV, "calendar").is_some(),
                    (Some(DAV), "current-user-principal") => r.principal = child(p, DAV, "href").map(text),
                    (Some(DAV), "current-user-privilege-set") => {
                        r.privileges = Some(
                            p.descendants()
                                .filter(|n| is(*n, DAV, "privilege"))
                                .flat_map(|n| n.children().filter(|c| c.is_element()))
                                .map(|n| n.tag_name().name().to_string())
                                .collect(),
                        )
                    }
                    (Some(CALDAV), "calendar-data") => r.calendar_data = Some(text_raw(p)),
                    (Some(CALDAV), "calendar-home-set") => r.calendar_home = child(p, DAV, "href").map(text),
                    (Some(CALDAV), "supported-calendar-component-set") => {
                        r.components = Some(
                            p.children()
                                .filter(|n| is(*n, CALDAV, "comp"))
                                .filter_map(|n| n.attribute("name").map(String::from))
                                .collect(),
                        )
                    }
                    _ => {}
                }
            }
        }
        out.push(r);
    }
    Ok(out)
}

fn status_ok(line: &str) -> bool {
    // "HTTP/1.1 200 OK"
    line.split_whitespace().nth(1).is_some_and(|code| code.starts_with('2'))
}

fn is(n: roxmltree::Node, ns: &str, name: &str) -> bool {
    n.is_element() && n.tag_name().name() == name && n.tag_name().namespace() == Some(ns)
}

fn child<'a, 'i>(n: roxmltree::Node<'a, 'i>, ns: &str, name: &str) -> Option<roxmltree::Node<'a, 'i>> {
    n.children().find(|c| is(*c, ns, name))
}

fn text(n: roxmltree::Node) -> String {
    text_raw(n).trim().to_string()
}

fn text_raw(n: roxmltree::Node) -> String {
    n.descendants()
        .filter(|d| d.is_text())
        .filter_map(|d| d.text())
        .collect()
}

/// Resolves an `href` from a multistatus (normally an absolute path) against the URL it came from.
pub fn resolve(base: &str, href: &str) -> String {
    if href.contains("://") {
        return href.to_string();
    }
    let origin_end = base
        .find("://")
        .map(|i| i + 3)
        .and_then(|start| base[start..].find('/').map(|j| start + j))
        .unwrap_or(base.len());
    if href.starts_with('/') {
        format!("{}{href}", &base[..origin_end])
    } else {
        let dir = base
            .rfind('/')
            .filter(|&i| i >= origin_end)
            .map_or(base.len(), |i| i + 1);
        format!("{}{href}", &base[..dir])
    }
}

/// `https://dav.example/a/b/` → `/a/b/`, to compare with a multistatus `href`.
fn path_of(url: &str) -> &str {
    match url.find("://") {
        Some(i) => url[i + 3..].find('/').map_or("/", |j| &url[i + 3 + j..]),
        None => url,
    }
}

fn same_resource(a: &str, b: &str) -> bool {
    let a = path_of(a).trim_end_matches('/');
    let b = path_of(b).trim_end_matches('/');
    a == b || percent_decode(a) == percent_decode(b)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    (c as char).to_digit(16).map(|d| d as u8)
}

/// A calendar object (one `.ics` resource) from a calendar-query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarObject {
    /// Absolute URL of the object.
    pub url: String,
    pub etag: Option<String>,
    pub ics: String,
}

/// A calendar collection found by discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    /// Absolute URL of the collection, ending in `/`.
    pub url: String,
    pub name: Option<String>,
    pub writable: bool,
}

const PROPFIND_COLLECTION: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:resourcetype/>
    <d:displayname/>
    <d:current-user-principal/>
    <d:current-user-privilege-set/>
    <c:supported-calendar-component-set/>
    <c:calendar-home-set/>
  </d:prop>
</d:propfind>"#;

const PROPFIND_HOME: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <c:calendar-home-set/>
  </d:prop>
</d:propfind>"#;

const PROPFIND_ETAG: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:getetag/>
  </d:prop>
</d:propfind>"#;

fn calendar_query(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let f = "%Y%m%dT%H%M%SZ";
    format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop>
    <d:getetag/>
    <c:calendar-data/>
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{}" end="{}"/>
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#,
        start.format(f),
        end.format(f)
    )
}

/// A CalDAV client for one account: the transport plus the `Authorization` header, if any.
#[derive(Clone)]
pub struct DavClient {
    transport: Arc<dyn Transport>,
    authorization: Option<String>,
}

impl DavClient {
    pub fn new(transport: Arc<dyn Transport>, username: Option<&str>, password: Option<&str>) -> Self {
        let authorization = username.map(|u| format!("Basic {}", base64(&format!("{u}:{}", password.unwrap_or("")))));
        DavClient {
            transport,
            authorization,
        }
    }

    async fn send(
        &self,
        method: &'static str,
        url: &str,
        mut headers: Vec<(&'static str, String)>,
        body: Option<String>,
    ) -> Result<HttpResponse, DavError> {
        if let Some(a) = &self.authorization {
            headers.push(("Authorization", a.clone()));
        }
        self.transport
            .send(HttpRequest {
                method,
                url: url.to_string(),
                headers,
                body,
            })
            .await
            .map_err(DavError::Unreachable)
    }

    async fn propfind(&self, url: &str, depth: &str, body: &str) -> Result<Vec<DavResource>, DavError> {
        let res = self
            .send(
                "PROPFIND",
                url,
                vec![
                    ("Depth", depth.into()),
                    ("Content-Type", "application/xml; charset=utf-8".into()),
                ],
                Some(body.into()),
            )
            .await?;
        check(&res, &[207])?;
        parse_multistatus(&res.body)
    }

    /// Finds the calendar a configured URL means: the URL itself when it is a calendar, or the
    /// only event calendar in the account's calendar home when it points at the account.
    pub async fn discover(&self, url: &str) -> Result<CalendarInfo, DavError> {
        let url = with_slash(url);
        let found = self.propfind(&url, "0", PROPFIND_COLLECTION).await?;
        let me = found
            .iter()
            .find(|r| same_resource(&resolve(&url, &r.href), &url))
            .or(found.first())
            .ok_or_else(|| DavError::BadResponse("empty multistatus".into()))?;
        if me.is_calendar {
            if !me.holds_events() {
                return Err(DavError::NoEvents);
            }
            return Ok(CalendarInfo {
                url,
                name: me.displayname.clone(),
                writable: me.writable(),
            });
        }
        let home = match (&me.calendar_home, &me.principal) {
            (Some(h), _) => resolve(&url, h),
            (None, Some(p)) => {
                let principal = resolve(&url, p);
                let r = self.propfind(&principal, "0", PROPFIND_HOME).await?;
                let h = r.iter().find_map(|r| r.calendar_home.clone());
                resolve(&principal, &h.ok_or(DavError::NotACalendar { found: 0 })?)
            }
            (None, None) => return Err(DavError::NotACalendar { found: 0 }),
        };
        let cals: Vec<_> = self
            .propfind(&home, "1", PROPFIND_COLLECTION)
            .await?
            .into_iter()
            .filter(|r| r.is_calendar && r.holds_events())
            .collect();
        match cals.as_slice() {
            [one] => Ok(CalendarInfo {
                url: with_slash(&resolve(&home, &one.href)),
                name: one.displayname.clone(),
                writable: one.writable(),
            }),
            _ => Err(DavError::NotACalendar { found: cals.len() }),
        }
    }

    /// Events overlapping `[start, end)`: masters and their edited instances, unexpanded, so
    /// the calendar engine handles recurrence the same way as for ICS feeds.
    pub async fn query(
        &self,
        calendar: &str,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> Result<Vec<CalendarObject>, DavError> {
        let res = self
            .send(
                "REPORT",
                calendar,
                vec![
                    ("Depth", "1".into()),
                    ("Content-Type", "application/xml; charset=utf-8".into()),
                ],
                Some(calendar_query(start, end)),
            )
            .await?;
        check(&res, &[207])?;
        Ok(parse_multistatus(&res.body)?
            .into_iter()
            .filter_map(|r| {
                Some(CalendarObject {
                    url: resolve(calendar, &r.href),
                    etag: r.etag,
                    ics: r.calendar_data?,
                })
            })
            .collect())
    }

    /// Creates a new object; fails with `Conflict` if one already exists at `url`. Returns its
    /// ETag when the server says it.
    pub async fn create(&self, url: &str, ics: String) -> Result<Option<String>, DavError> {
        let res = self
            .send(
                "PUT",
                url,
                vec![
                    ("Content-Type", "text/calendar; charset=utf-8".into()),
                    ("If-None-Match", "*".into()),
                ],
                Some(ics),
            )
            .await?;
        check(&res, &[200, 201, 204])?;
        Ok(res.etag)
    }

    /// The current ETag of an object.
    pub async fn etag(&self, url: &str) -> Result<Option<String>, DavError> {
        let r = self.propfind(url, "0", PROPFIND_ETAG).await?;
        Ok(r.into_iter().find_map(|r| r.etag))
    }

    /// Deletes an object only if it still has `etag`. An object that is already gone counts as
    /// deleted.
    pub async fn delete(&self, url: &str, etag: &str) -> Result<(), DavError> {
        let res = self.send("DELETE", url, vec![("If-Match", etag.into())], None).await?;
        match check(&res, &[200, 202, 204]) {
            Err(DavError::NotFound) => Ok(()),
            r => r,
        }
    }
}

fn with_slash(url: &str) -> String {
    if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{url}/")
    }
}

fn base64(s: &str) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let b = s.as_bytes();
    let mut out = String::with_capacity(b.len().div_ceil(3) * 4);
    for chunk in b.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        for (i, o) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("Aladdin:open sesame", "QWxhZGRpbjpvcGVuIHNlc2FtZQ=="),
        ] {
            assert_eq!(base64(i), o, "{i}");
        }
    }

    #[test]
    fn resolves_hrefs() {
        let base = "https://dav.example:5232/alex/family/";
        assert_eq!(
            resolve(base, "/alex/family/x.ics"),
            "https://dav.example:5232/alex/family/x.ics"
        );
        assert_eq!(resolve(base, "x.ics"), "https://dav.example:5232/alex/family/x.ics");
        assert_eq!(resolve(base, "https://other/x.ics"), "https://other/x.ics");
        assert_eq!(resolve("https://dav.example", "/a/"), "https://dav.example/a/");
    }

    #[test]
    fn same_resource_ignores_slash_and_encoding() {
        assert!(same_resource("https://h/a%20b/", "/a b"));
        assert!(!same_resource("https://h/a/", "/a/b/"));
    }

    #[test]
    fn skips_failed_propstats() {
        let xml = r#"<?xml version="1.0"?>
<multistatus xmlns="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav">
  <response><href>/a/</href>
    <propstat><prop><resourcetype><collection/><C:calendar/></resourcetype></prop><status>HTTP/1.1 200 OK</status></propstat>
    <propstat><prop><displayname>nope</displayname></prop><status>HTTP/1.1 404 Not Found</status></propstat>
  </response>
</multistatus>"#;
        let r = parse_multistatus(xml).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].is_calendar);
        assert_eq!(r[0].displayname, None);
        assert!(r[0].writable() && r[0].holds_events());
    }

    #[test]
    fn rejects_html() {
        assert!(matches!(
            parse_multistatus("<html><body>login</body></html>"),
            Err(DavError::BadResponse(_))
        ));
    }

    #[test]
    fn errors_never_show_urls() {
        for e in [
            DavError::Unreachable("timed out".into()),
            DavError::Unauthorized,
            DavError::Conflict,
            DavError::Status(500),
            DavError::NotACalendar { found: 2 },
        ] {
            assert!(!e.to_string().contains("://"));
        }
    }
}
