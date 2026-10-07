//! Токены и их хранение.

use std::fmt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::Result;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    /// Unix-время истечения в секундах.
    pub expires_at: Option<u64>,
}

impl TokenSet {
    pub fn new(access_token: String, refresh_token: Option<String>, expires_in: Option<u64>) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at: expires_in.map(|s| now_unix() + s),
        }
    }

    /// Истекает ли токен в ближайшие `secs` секунд.
    pub fn expires_within(&self, secs: u64) -> bool {
        self.expires_at
            .is_some_and(|at| at <= now_unix().saturating_add(secs))
    }
}

// Не печатаем токены в логи.
impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "<redacted>"))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Где хранится токен: в приложении и CLI — [`crate::secrets::SecretTokenStore`],
/// в тестах — [`MemoryTokenStore`].
pub trait TokenStore: Send + Sync {
    fn load(&self) -> Result<Option<TokenSet>>;
    fn save(&self, tokens: &TokenSet) -> Result<()>;
    fn clear(&self) -> Result<()>;
}

#[derive(Default)]
pub struct MemoryTokenStore {
    inner: Mutex<Option<TokenSet>>,
}

impl MemoryTokenStore {
    pub fn new(tokens: Option<TokenSet>) -> Self {
        Self {
            inner: Mutex::new(tokens),
        }
    }
}

impl TokenStore for MemoryTokenStore {
    fn load(&self) -> Result<Option<TokenSet>> {
        Ok(self.inner.lock().unwrap().clone())
    }

    fn save(&self, tokens: &TokenSet) -> Result<()> {
        *self.inner.lock().unwrap() = Some(tokens.clone());
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        *self.inner.lock().unwrap() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expiry_check() {
        let t = TokenSet::new("a".into(), None, Some(100));
        assert!(!t.expires_within(10));
        assert!(t.expires_within(1000));

        let no_expiry = TokenSet::new("a".into(), None, None);
        assert!(!no_expiry.expires_within(u64::MAX));
    }

    #[test]
    fn debug_hides_tokens() {
        let t = TokenSet::new("secret-access".into(), Some("secret-refresh".into()), None);
        let s = format!("{t:?}");
        assert!(!s.contains("secret"));
    }

    #[test]
    fn memory_store_roundtrip() {
        let store = MemoryTokenStore::default();
        assert!(store.load().unwrap().is_none());
        let t = TokenSet::new("a".into(), Some("r".into()), Some(10));
        store.save(&t).unwrap();
        assert_eq!(store.load().unwrap(), Some(t));
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
    }
}
