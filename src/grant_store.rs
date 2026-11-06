//! Encrypted on-disk storage for the Immich device grant, so it survives restarts.
//!
//! The file holds `MAGIC || nonce || AES-256-GCM(record) || tag`. The key is derived from
//! `HQ_KEY` with HKDF-SHA256, and the broker and Immich URLs are bound in as associated data,
//! so a grant is only reloaded by an instance configured for the same servers.
use chrono::{DateTime, Utc};
use ring::{
    aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey},
    hkdf,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

const MAGIC: &[u8; 4] = b"HQG1";
const SALT: &[u8] = b"home-hq grant store v1";
const INFO: &[u8] = b"immich device grant";
pub const FILE_NAME: &str = "immich-grant.bin";

/// What is persisted: the issued key (which is also the broker's handle for `/device/revoke`),
/// its expiry, scope, and the local deadline computed when it was issued.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct StoredGrant {
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub deadline: DateTime<Utc>,
    pub scope: String,
}

pub struct GrantStore {
    path: PathBuf,
    key: LessSafeKey,
    aad: Vec<u8>,
    rng: SystemRandom,
}

/// Why a stored grant was not loaded. Every variant except `Missing` means the file is unusable.
#[derive(Debug, PartialEq)]
pub enum LoadError {
    Missing,
    Unreadable,
    Undecryptable,
}

impl GrantStore {
    /// `None` when there is no dashboard key: the grant is never written unencrypted.
    pub fn new(dir: &Path, hq_key: Option<&str>, broker: &str, immich: &str) -> Option<Self> {
        let hq_key = hq_key.filter(|k| !k.is_empty())?;
        let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, SALT).extract(hq_key.as_bytes());
        let okm = prk
            .expand(&[INFO], &AES_256_GCM)
            .expect("HKDF output length matches AES-256-GCM");
        let key = LessSafeKey::new(UnboundKey::from(okm));
        let mut aad = MAGIC.to_vec();
        for part in [broker, immich] {
            aad.extend_from_slice(&(part.len() as u64).to_be_bytes());
            aad.extend_from_slice(part.as_bytes());
        }
        Some(Self {
            path: dir.join(FILE_NAME),
            key,
            aad,
            rng: SystemRandom::new(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<StoredGrant, LoadError> {
        let data = match std::fs::read(&self.path) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(LoadError::Missing),
            Err(_) => return Err(LoadError::Unreadable),
        };
        let Some(rest) = data.strip_prefix(MAGIC.as_slice()) else {
            return Err(LoadError::Unreadable);
        };
        if rest.len() < NONCE_LEN + AES_256_GCM.tag_len() {
            return Err(LoadError::Unreadable);
        }
        let (nonce, sealed) = rest.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).map_err(|_| LoadError::Unreadable)?;
        let mut sealed = sealed.to_vec();
        let plain = self
            .key
            .open_in_place(nonce, Aad::from(&self.aad), &mut sealed)
            .map_err(|_| LoadError::Undecryptable)?;
        serde_json::from_slice(plain).map_err(|_| LoadError::Undecryptable)
    }

    /// Writes atomically (temporary file, fsync, rename), readable by the owner only.
    pub fn save(&self, grant: &StoredGrant) -> anyhow::Result<()> {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("no randomness for the grant nonce"))?;
        let mut sealed = serde_json::to_vec(grant)?;
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(&self.aad), &mut sealed)
            .map_err(|_| anyhow::anyhow!("could not encrypt the grant"))?;
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

    pub fn delete(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh, empty directory under the system temp dir, removed on drop.
    pub struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("home-hq-test-{}-{n}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn grant() -> StoredGrant {
        StoredGrant {
            token: "private-issued-key".into(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            deadline: Utc::now() + chrono::Duration::hours(1),
            scope: "asset.read".into(),
        }
    }

    #[test]
    fn round_trips_without_leaking_the_key() {
        let dir = TempDir::new();
        let store = GrantStore::new(&dir.0, Some("k1"), "https://b/", "https://i/api").unwrap();
        assert_eq!(store.load(), Err(LoadError::Missing));
        let saved = grant();
        store.save(&saved).unwrap();
        let raw = std::fs::read(store.path()).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("private-issued-key"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(store.path()).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(store.load().unwrap(), saved);
        // A fresh nonce each time: the same grant never encrypts to the same bytes.
        store.save(&saved).unwrap();
        assert_ne!(std::fs::read(store.path()).unwrap(), raw);
        store.delete().unwrap();
        store.delete().unwrap();
        assert_eq!(store.load(), Err(LoadError::Missing));
    }

    #[test]
    fn needs_hq_key_and_rejects_other_keys_servers_and_tampering() {
        let dir = TempDir::new();
        assert!(GrantStore::new(&dir.0, None, "https://b/", "https://i/api").is_none());
        assert!(GrantStore::new(&dir.0, Some(""), "https://b/", "https://i/api").is_none());
        let store = GrantStore::new(&dir.0, Some("k1"), "https://b/", "https://i/api").unwrap();
        store.save(&grant()).unwrap();
        let other_key = GrantStore::new(&dir.0, Some("k2"), "https://b/", "https://i/api").unwrap();
        assert_eq!(other_key.load(), Err(LoadError::Undecryptable));
        let other_immich = GrantStore::new(&dir.0, Some("k1"), "https://b/", "https://evil/api").unwrap();
        assert_eq!(other_immich.load(), Err(LoadError::Undecryptable));
        let mut raw = std::fs::read(store.path()).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 1;
        std::fs::write(store.path(), &raw).unwrap();
        assert_eq!(store.load(), Err(LoadError::Undecryptable));
        std::fs::write(store.path(), b"garbage").unwrap();
        assert_eq!(store.load(), Err(LoadError::Unreadable));
    }
}
