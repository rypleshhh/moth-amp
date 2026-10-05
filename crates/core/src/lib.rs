//! Ядро клиента: модели, источники музыки, авторизация.
//!
//! UI-слой (Flutter) и CLI работают только через типы этого крейта и
//! трейт [`provider::Provider`], не зная деталей конкретного сервиса.

pub mod auth;
pub mod cache;
pub mod dsp;
pub mod error;
mod fsutil;
pub mod model;
pub mod provider;
pub mod s3;
pub mod secrets;
pub mod yandex;

pub use error::{Error, Result};
