//! Sharing a photo: the link a phone opens to see it in Immich.
//!
//! `HQ_IMMICH_PUBLIC_URL` is the address people's phones reach Immich on, which is often not the one
//! the backend uses (a cluster-internal Service). The link is a plain `<url>/photos/<assetId>`:
//! it carries no key or grant, so whoever scans it still signs in to Immich with their own login.
//! The frame's QR opens the Immich phone app by default, with `immich://asset?id=<assetId>`
//! (handled by the app on Android and iOS, in the server it's signed in to), and can switch to
//! the web link for phones without the app.

use anyhow::bail;
use serde_json::Value;

/// The public Immich address, validated and without a trailing slash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImmichPublicUrl(String);

impl ImmichPublicUrl {
    /// `None` for an empty value (sharing off); an error for anything that isn't a plain
    /// `http(s)://host[/path]` URL.
    pub fn parse(raw: &str) -> anyhow::Result<Option<Self>> {
        let url = raw.trim().trim_end_matches('/');
        if url.is_empty() {
            return Ok(None);
        }
        let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else {
            bail!("HQ_IMMICH_PUBLIC_URL must start with https:// or http://, got {raw:?}");
        };
        let host = rest.split('/').next().unwrap_or("");
        if host.is_empty()
            || host.contains('@')
            || url.contains(['?', '#'])
            || url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
        {
            bail!("HQ_IMMICH_PUBLIC_URL must be a plain URL such as https://photos.example.com, got {raw:?}");
        }
        Ok(Some(Self(url.to_string())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `<url>/photos/<assetId>`, or `None` when the ID doesn't look like an Immich asset ID.
    pub fn photo_url(&self, asset_id: &str) -> Option<String> {
        valid_asset_id(asset_id).then(|| format!("{}/photos/{asset_id}", self.0))
    }
}

/// The Immich phone app's link to an asset, `immich://asset?id=<assetId>`, or `None` when the ID
/// doesn't look like an Immich asset ID.
pub fn app_url(asset_id: &str) -> Option<String> {
    valid_asset_id(asset_id).then(|| format!("immich://asset?id={asset_id}"))
}

/// A UUID in practice; anything of letters, digits and dashes is accepted, so the ID can't change
/// a link's path or query.
fn valid_asset_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
}

/// Sets `shareUrl` (web) and `shareAppUrl` (phone app) on each photo that came from Immich (has
/// an `immichId`) and removes them from the rest. Without a public URL no photo gets them, which
/// hides the frame's Share button.
pub fn annotate(photos: &mut Value, public: Option<&ImmichPublicUrl>) {
    for p in photos.as_array_mut().into_iter().flatten() {
        let id = p["immichId"].as_str().unwrap_or("");
        let links = public.and_then(|u| u.photo_url(id)).zip(app_url(id));
        if let Some(obj) = p.as_object_mut() {
            match links {
                Some((web, app)) => {
                    obj.insert("shareUrl".into(), web.into());
                    obj.insert("shareAppUrl".into(), app.into());
                }
                None => {
                    obj.remove("shareUrl");
                    obj.remove("shareAppUrl");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const ASSET: &str = "9b2d8a4e-3f1c-4e7a-b0d5-6c8f1e2a3b4c";

    fn url(s: &str) -> ImmichPublicUrl {
        ImmichPublicUrl::parse(s).unwrap().unwrap()
    }

    #[test]
    fn builds_the_photo_link() {
        assert_eq!(
            url("https://photos.example.com").photo_url(ASSET).unwrap(),
            format!("https://photos.example.com/photos/{ASSET}")
        );
    }

    #[test]
    fn builds_the_app_link() {
        assert_eq!(app_url(ASSET).unwrap(), format!("immich://asset?id={ASSET}"));
        for bad in ["", "a&b=c", "a/b", "a b"] {
            assert_eq!(app_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn trailing_slashes_and_paths() {
        assert_eq!(
            url(" https://photos.example.com/ ").as_str(),
            "https://photos.example.com"
        );
        assert_eq!(
            url("http://nas.lan:2283/immich//").photo_url("abc").unwrap(),
            "http://nas.lan:2283/immich/photos/abc"
        );
    }

    #[test]
    fn empty_means_off() {
        assert_eq!(ImmichPublicUrl::parse("").unwrap(), None);
        assert_eq!(ImmichPublicUrl::parse("  /").unwrap(), None);
    }

    #[test]
    fn rejects_urls_that_are_not_plain() {
        for bad in [
            "photos.example.com",
            "ftp://photos.example.com",
            "https://",
            "https:///photos",
            "https://user:pass@photos.example.com",
            "https://photos.example.com/?key=abc",
            "https://photos.example.com/#x",
            "https://photos example.com",
            "javascript:alert(1)",
        ] {
            assert!(ImmichPublicUrl::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rejects_asset_ids_that_would_change_the_path() {
        let u = url("https://photos.example.com");
        for bad in ["", "../api/keys", "a/b", "a?b", "a b", &"a".repeat(65)] {
            assert_eq!(u.photo_url(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_immich_photos_get_a_share_url_and_only_when_configured() {
        let photos = || {
            json!([
                { "id": "p1", "immichId": ASSET },
                { "id": "demo0" },
                { "id": "p2", "immichId": "../x" },
                { "id": "p3", "shareUrl": "https://stale.example.com/photos/x", "shareAppUrl": "immich://asset?id=x" },
            ])
        };

        let mut on = photos();
        annotate(&mut on, Some(&url("https://photos.example.com/")));
        assert_eq!(on[0]["shareUrl"], format!("https://photos.example.com/photos/{ASSET}"));
        assert_eq!(on[0]["shareAppUrl"], format!("immich://asset?id={ASSET}"));
        for p in &on.as_array().unwrap()[1..] {
            assert!(p.get("shareUrl").is_none() && p.get("shareAppUrl").is_none(), "{p}");
        }

        let mut off = photos();
        annotate(&mut off, None);
        assert!(
            off.as_array()
                .unwrap()
                .iter()
                .all(|p| p.get("shareUrl").is_none() && p.get("shareAppUrl").is_none())
        );
    }
}
