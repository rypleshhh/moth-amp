//! Ядро moth-amp: API Яндекс Музыки, «Моя волна», кэш с локальным прокси,
//! эквалайзер, своя музыка в S3, хранение секретов. Им пользуются
//! Flutter-приложение (через мост `app/rust`) и консольный клиент `moth`.

pub mod auth;
pub mod cache;
pub mod dsp;
pub mod error;
mod fsutil;
pub mod model;
pub mod s3;
pub mod secrets;
pub mod yandex;

pub use error::{Error, Result};
