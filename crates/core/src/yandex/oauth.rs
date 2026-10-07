//! Вход через OAuth Device Flow: пользователь вводит короткий код на сайте
//! Яндекса, пароль в клиент не попадает.
//!
//! `device_id` / `device_name` намеренно не передаются: они необязательны,
//! и без них Яндекс не получает лишнего идентификатора устройства.

use std::time::Duration;

use serde::Deserialize;

use crate::auth::{now_unix, TokenSet};
use crate::{Error, Result};

const OAUTH_BASE: &str = "https://oauth.yandex.ru";

/// Общий client_id сообщества (тот же, что в MarshalX/yandex-music-api и других
/// неофициальных клиентах). Не секрет: он вшит во все такие клиенты.
const CLIENT_ID: &str = "23cabbbdc6cd418abb4b39c32c41195d";
const CLIENT_SECRET: &str = "53bc75238f0c4d08a118e51fe9203300";

#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default = "default_expires_in")]
    pub expires_in: u64,
}

fn default_interval() -> u64 {
    5
}

fn default_expires_in() -> u64 {
    600
}

#[derive(Debug, PartialEq, Eq)]
pub enum PollResult {
    /// Пользователь ещё не подтвердил вход.
    Pending,
    /// Сервер просит опрашивать реже.
    SlowDown,
    Ready(TokenSet),
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct OAuthErrorBody {
    error: String,
    error_description: Option<String>,
}

#[derive(Clone)]
pub struct OAuthClient {
    http: reqwest::Client,
}

impl OAuthClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    pub async fn request_device_code(&self) -> Result<DeviceCode> {
        let resp = self
            .http
            .post(format!("{OAUTH_BASE}/device/code"))
            .form(&[("client_id", CLIENT_ID)])
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(oauth_error(&body, status));
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Один опрос: подтвердил ли пользователь код.
    pub async fn poll_token(&self, device_code: &str) -> Result<PollResult> {
        let resp = self
            .http
            .post(format!("{OAUTH_BASE}/token"))
            .form(&[
                ("grant_type", "device_code"),
                ("code", device_code),
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
            ])
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        parse_poll_response(status, &body)
    }

    /// Опрашивает сервер, пока пользователь не подтвердит вход или код не истечёт.
    pub async fn wait_for_token(&self, code: &DeviceCode) -> Result<TokenSet> {
        let deadline = now_unix() + code.expires_in;
        let mut interval = code.interval.max(1);
        loop {
            tokio::time::sleep(Duration::from_secs(interval)).await;
            if now_unix() > deadline {
                return Err(Error::DeviceCodeExpired);
            }
            match self.poll_token(&code.device_code).await? {
                PollResult::Ready(tokens) => return Ok(tokens),
                PollResult::Pending => {}
                PollResult::SlowDown => interval += 5,
            }
        }
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenSet> {
        let resp = self
            .http
            .post(format!("{OAUTH_BASE}/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", CLIENT_ID),
                ("client_secret", CLIENT_SECRET),
            ])
            .send()
            .await?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(oauth_error(&body, status));
        }
        let t: TokenResponse = serde_json::from_str(&body)?;
        let mut tokens = TokenSet::new(t.access_token, t.refresh_token, t.expires_in);
        // Если сервер не выдал новый refresh_token, продолжаем пользоваться старым.
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_owned());
        }
        Ok(tokens)
    }
}

fn parse_poll_response(status: reqwest::StatusCode, body: &str) -> Result<PollResult> {
    if status.is_success() {
        let t: TokenResponse = serde_json::from_str(body)?;
        return Ok(PollResult::Ready(TokenSet::new(
            t.access_token,
            t.refresh_token,
            t.expires_in,
        )));
    }
    match serde_json::from_str::<OAuthErrorBody>(body) {
        Ok(e) if e.error == "authorization_pending" => Ok(PollResult::Pending),
        Ok(e) if e.error == "slow_down" => Ok(PollResult::SlowDown),
        Ok(e) if e.error == "expired_token" => Err(Error::DeviceCodeExpired),
        _ => Err(oauth_error(body, status)),
    }
}

fn oauth_error(body: &str, status: reqwest::StatusCode) -> Error {
    match serde_json::from_str::<OAuthErrorBody>(body) {
        Ok(e) => Error::OAuth {
            code: e.error,
            description: e.error_description.unwrap_or_default(),
        },
        Err(_) => Error::Unexpected(format!("HTTP {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn device_code_defaults() {
        let c: DeviceCode = serde_json::from_str(
            r#"{"device_code":"d","user_code":"1234567","verification_url":"https://ya.ru/device"}"#,
        )
        .unwrap();
        assert_eq!(c.interval, 5);
        assert_eq!(c.expires_in, 600);
    }

    #[test]
    fn poll_pending_and_slow_down() {
        let pending = r#"{"error":"authorization_pending","error_description":"User has not yet authorized"}"#;
        assert_eq!(
            parse_poll_response(StatusCode::BAD_REQUEST, pending).unwrap(),
            PollResult::Pending
        );
        let slow = r#"{"error":"slow_down"}"#;
        assert_eq!(
            parse_poll_response(StatusCode::BAD_REQUEST, slow).unwrap(),
            PollResult::SlowDown
        );
    }

    #[test]
    fn poll_expired() {
        let body = r#"{"error":"expired_token"}"#;
        assert!(matches!(
            parse_poll_response(StatusCode::BAD_REQUEST, body),
            Err(Error::DeviceCodeExpired)
        ));
    }

    #[test]
    fn poll_ready() {
        let body = r#"{"token_type":"bearer","access_token":"AT","expires_in":31536000,"refresh_token":"RT"}"#;
        match parse_poll_response(StatusCode::OK, body).unwrap() {
            PollResult::Ready(t) => {
                assert_eq!(t.access_token, "AT");
                assert_eq!(t.refresh_token.as_deref(), Some("RT"));
                assert!(t.expires_at.is_some());
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn non_json_error() {
        let err = parse_poll_response(StatusCode::BAD_GATEWAY, "<html>").unwrap_err();
        assert!(matches!(err, Error::Unexpected(_)));
    }
}
