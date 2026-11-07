//! Nextcloud's Login Flow v2, as Nextcloud and RustiCal serve it: Home HQ asks the server for a
//! login page, a person opens it on their phone (signing in there, with OIDC if the server uses
//! it) and approves, and Home HQ collects an app password made for it. No password is typed on
//! the frame.
//!
//! 1. `POST <server>/index.php/login/v2` → `{ "login": "<page>", "poll": { "token", "endpoint" } }`
//! 2. `POST <endpoint>` with the form body `token=…`: 404 while waiting, then
//!    `{ "server", "loginName", "appPassword" }`; 401 or 404 forever once it has expired.
//!
//! The server names the app password after our `User-Agent`. RustiCal builds `login` and
//! `endpoint` from the `Host` header with `https://`; the poll goes to the endpoint's path on the
//! origin we started with, so an internal service address still works for polling.

use serde::Deserialize;

use super::dav::{DavError, HttpRequest, Transport, origin_of};

/// A started flow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flow {
    /// The page to open on a phone.
    pub login: String,
    token: String,
    /// Where to poll, on the origin the flow was started on.
    endpoint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    Waiting,
    /// Approved: the user to sign in as and the app password made for Home HQ.
    Approved {
        login_name: String,
        app_password: String,
    },
    /// Expired, declined or already collected.
    Gone,
}

#[derive(Deserialize)]
struct StartBody {
    login: String,
    poll: PollInfo,
}

#[derive(Deserialize)]
struct PollInfo {
    token: String,
    endpoint: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Done {
    login_name: String,
    app_password: String,
}

fn form_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Starts a flow on `origin` (`https://dav.example`, no path).
pub async fn start(t: &dyn Transport, origin: &str) -> Result<Flow, DavError> {
    let origin = origin.trim_end_matches('/');
    let res = t
        .send(HttpRequest {
            method: "POST",
            url: format!("{origin}/index.php/login/v2"),
            headers: vec![("Accept", "application/json".into())],
            body: None,
        })
        .await
        .map_err(DavError::Unreachable)?;
    match res.status {
        200 => {}
        404 | 405 => return Err(DavError::NotFound),
        401 | 403 => return Err(DavError::Unauthorized),
        s => return Err(DavError::Status(s)),
    }
    let body: StartBody = serde_json::from_str(&res.body)
        .map_err(|_| DavError::BadResponse("not a Nextcloud login flow answer".into()))?;
    let path = &body.poll.endpoint[origin_of(&body.poll.endpoint).len()..];
    if !path.starts_with('/') || !(body.login.starts_with("https://") || body.login.starts_with("http://")) {
        return Err(DavError::BadResponse("not a Nextcloud login flow answer".into()));
    }
    Ok(Flow {
        login: body.login,
        token: body.poll.token,
        endpoint: format!("{origin}{path}"),
    })
}

/// Checks a flow once.
pub async fn poll(t: &dyn Transport, flow: &Flow) -> Result<Poll, DavError> {
    let res = t
        .send(HttpRequest {
            method: "POST",
            url: flow.endpoint.clone(),
            headers: vec![
                ("Content-Type", "application/x-www-form-urlencoded".into()),
                ("Accept", "application/json".into()),
            ],
            body: Some(format!("token={}", form_escape(&flow.token))),
        })
        .await
        .map_err(DavError::Unreachable)?;
    match res.status {
        200 => {
            let d: Done = serde_json::from_str(&res.body)
                .map_err(|_| DavError::BadResponse("not a Nextcloud login flow answer".into()))?;
            if d.login_name.is_empty() || d.app_password.is_empty() {
                return Err(DavError::BadResponse("empty app password".into()));
            }
            Ok(Poll::Approved {
                login_name: d.login_name,
                app_password: d.app_password,
            })
        }
        // Nextcloud and RustiCal answer 404 while waiting.
        404 => Ok(Poll::Waiting),
        401 | 403 | 410 => Ok(Poll::Gone),
        s => Err(DavError::Status(s)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;

    use super::super::dav::HttpResponse;
    use super::*;

    struct Script(Mutex<Vec<(u16, &'static str)>>, Mutex<Vec<HttpRequest>>);

    #[async_trait]
    impl Transport for Script {
        async fn send(&self, req: HttpRequest) -> Result<HttpResponse, String> {
            self.1.lock().unwrap().push(req);
            let (status, body) = self.0.lock().unwrap().remove(0);
            Ok(HttpResponse {
                status,
                body: body.into(),
                ..Default::default()
            })
        }
    }

    #[tokio::test]
    async fn runs_the_flow_and_polls_on_our_origin() {
        // RustiCal reached on an internal address answers with its public https host.
        let t = Script(
            Mutex::new(vec![
                (
                    200,
                    r#"{"login":"https://dav.example/index.php/login/v2/flow/f1","poll":{"token":"t/k+n","endpoint":"https://dav.example/index.php/login/v2/poll/f1"}}"#,
                ),
                (404, ""),
                (
                    200,
                    r#"{"server":"https://dav.example","loginName":"alex","appPassword":"secret-app-pw"}"#,
                ),
                (401, ""),
            ]),
            Mutex::new(Vec::new()),
        );
        let flow = start(&t, "http://rustical.home.svc:4000/").await.unwrap();
        assert_eq!(flow.login, "https://dav.example/index.php/login/v2/flow/f1");
        assert_eq!(poll(&t, &flow).await.unwrap(), Poll::Waiting);
        assert_eq!(
            poll(&t, &flow).await.unwrap(),
            Poll::Approved {
                login_name: "alex".into(),
                app_password: "secret-app-pw".into()
            }
        );
        assert_eq!(poll(&t, &flow).await.unwrap(), Poll::Gone);
        let reqs = t.1.lock().unwrap();
        assert_eq!(reqs[0].url, "http://rustical.home.svc:4000/index.php/login/v2");
        assert_eq!(reqs[1].url, "http://rustical.home.svc:4000/index.php/login/v2/poll/f1");
        assert_eq!(reqs[1].body.as_deref(), Some("token=t%2Fk%2Bn"));
        assert_eq!(
            reqs[1].header("Content-Type"),
            Some("application/x-www-form-urlencoded")
        );
    }

    #[tokio::test]
    async fn servers_without_the_flow_say_not_found() {
        let t = Script(Mutex::new(vec![(404, "Not Found")]), Mutex::new(Vec::new()));
        assert_eq!(start(&t, "https://radicale.example").await, Err(DavError::NotFound));
        let t = Script(Mutex::new(vec![(200, "<html>login</html>")]), Mutex::new(Vec::new()));
        assert!(matches!(
            start(&t, "https://x.example").await,
            Err(DavError::BadResponse(_))
        ));
    }
}
