//! Ядро клиента: модели, источники музыки, авторизация.
//!
//! UI-слой (Flutter) и CLI работают только через типы этого крейта и
//! трейт [`provider::Provider`], не зная деталей конкретного сервиса.

pub mod auth;
pub mod error;
pub mod model;
pub mod provider;
pub mod yandex;

pub use error::{Error, Result};
