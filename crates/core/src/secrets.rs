//! Хранение секретов (токен Яндекса, ключи S3).
//!
//! - Windows/macOS/Linux — системное хранилище учётных данных ([`KeyringSecretStore`]).
//! - Android — файлы в приватной папке приложения ([`FileSecretStore`]): её
//!   закрывает песочница Android, а `keyring` там не умеет сохранять между
//!   запусками.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use crate::auth::{TokenSet, TokenStore};
use crate::fsutil::write_atomic;
use crate::{Error, Result};

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn set(&self, key: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str) -> Result<()>;
}

fn check_key(key: &str) -> Result<()> {
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        Ok(())
    } else {
        Err(Error::Storage(format!("недопустимое имя секрета: {key}")))
    }
}

/// Секреты в файлах `<dir>/<key>.secret`.
pub struct FileSecretStore {
    dir: PathBuf,
}

impl FileSecretStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn path(&self, key: &str) -> Result<PathBuf> {
        check_key(key)?;
        Ok(self.dir.join(format!("{key}.secret")))
    }
}

impl SecretStore for FileSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match fs::read_to_string(self.path(key)?) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        write_atomic(&self.path(key)?, value.as_bytes())
    }

    fn delete(&self, key: &str) -> Result<()> {
        match fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Секреты в системном хранилище: запись `<service>` / `<key>`.
#[cfg(feature = "keyring-store")]
pub struct KeyringSecretStore {
    service: String,
}

#[cfg(feature = "keyring-store")]
impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        check_key(key)?;
        keyring::Entry::new(&self.service, key).map_err(|e| Error::Storage(e.to_string()))
    }
}

#[cfg(feature = "keyring-store")]
impl SecretStore for KeyringSecretStore {
    fn get(&self, key: &str) -> Result<Option<String>> {
        match self.entry(key)?.get_password() {
            Ok(s) => Ok(Some(s)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Storage(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<()> {
        self.entry(key)?
            .set_password(value)
            .map_err(|e| Error::Storage(e.to_string()))
    }

    fn delete(&self, key: &str) -> Result<()> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Storage(e.to_string())),
        }
    }
}

/// Токен в хранилище секретов под ключом `key`.
pub struct SecretTokenStore {
    store: Arc<dyn SecretStore>,
    key: String,
}

impl SecretTokenStore {
    pub fn new(store: Arc<dyn SecretStore>, key: impl Into<String>) -> Self {
        Self {
            store,
            key: key.into(),
        }
    }
}

impl TokenStore for SecretTokenStore {
    fn load(&self) -> Result<Option<TokenSet>> {
        match self.store.get(&self.key)? {
            Some(json) => Ok(Some(serde_json::from_str(&json)?)),
            None => Ok(None),
        }
    }

    fn save(&self, tokens: &TokenSet) -> Result<()> {
        self.store.set(&self.key, &serde_json::to_string(tokens)?)
    }

    fn clear(&self) -> Result<()> {
        self.store.delete(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_roundtrip() {
        let dir = std::env::temp_dir().join(format!("moth-secrets-{}", std::process::id()));
        let store: Arc<dyn SecretStore> = Arc::new(FileSecretStore::new(&dir));
        assert_eq!(store.get("s3").unwrap(), None);
        store.set("s3", "{\"k\":1}").unwrap();
        assert_eq!(store.get("s3").unwrap().as_deref(), Some("{\"k\":1}"));
        assert!(store.set("../evil", "x").is_err());

        let tokens = SecretTokenStore::new(store.clone(), "yandex");
        let t = TokenSet::new("a".into(), Some("r".into()), Some(10));
        tokens.save(&t).unwrap();
        assert_eq!(tokens.load().unwrap(), Some(t));
        tokens.clear().unwrap();
        assert!(tokens.load().unwrap().is_none());

        store.delete("s3").unwrap();
        store.delete("s3").unwrap();
        std::fs::remove_dir_all(dir).unwrap();
    }
}
