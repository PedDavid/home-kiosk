//! Sharing a photo: the link a phone opens to see it in Immich.
//!
//! `IMMICH_PUBLIC_URL` is the address people's phones reach Immich on, which is often not the one
//! the backend uses (a cluster-internal Service). The link is a plain `<url>/photos/<assetId>`:
//! it carries no key or grant, so whoever scans it still signs in to Immich with their own login.

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
            bail!("IMMICH_PUBLIC_URL must start with https:// or http://, got {raw:?}");
        };
        let host = rest.split('/').next().unwrap_or("");
        if host.is_empty()
            || host.contains('@')
            || url.contains(['?', '#'])
            || url.chars().any(|c| c.is_whitespace() || c.is_control() || c == '"')
        {
            bail!("IMMICH_PUBLIC_URL must be a plain URL such as https://photos.example.com, got {raw:?}");
        }
        Ok(Some(Self(url.to_string())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `<url>/photos/<assetId>`, or `None` when the ID doesn't look like an Immich asset ID
    /// (a UUID; anything of letters, digits and dashes is accepted).
    pub fn photo_url(&self, asset_id: &str) -> Option<String> {
        let valid =
            (1..=64).contains(&asset_id.len()) && asset_id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-');
        valid.then(|| format!("{}/photos/{asset_id}", self.0))
    }
}

/// Sets `shareUrl` on each photo that came from Immich (has an `immichId`) and removes it from the
/// rest. Without a public URL no photo gets one, which hides the frame's Share button.
pub fn annotate(photos: &mut Value, public: Option<&ImmichPublicUrl>) {
    for p in photos.as_array_mut().into_iter().flatten() {
        let url = public.zip(p["immichId"].as_str()).and_then(|(u, id)| u.photo_url(id));
        if let Some(obj) = p.as_object_mut() {
            match url {
                Some(u) => obj.insert("shareUrl".into(), u.into()),
                None => obj.remove("shareUrl"),
            };
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
                { "id": "p3", "shareUrl": "https://stale.example.com/photos/x" },
            ])
        };

        let mut on = photos();
        annotate(&mut on, Some(&url("https://photos.example.com/")));
        assert_eq!(on[0]["shareUrl"], format!("https://photos.example.com/photos/{ASSET}"));
        for p in &on.as_array().unwrap()[1..] {
            assert!(p.get("shareUrl").is_none(), "{p}");
        }

        let mut off = photos();
        annotate(&mut off, None);
        assert!(off.as_array().unwrap().iter().all(|p| p.get("shareUrl").is_none()));
    }
}
