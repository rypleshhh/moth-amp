use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("сетевая ошибка: {0}")]
    Http(#[from] reqwest::Error),

    #[error("не удалось разобрать ответ: {0}")]
    Decode(#[from] serde_json::Error),

    #[error("ошибка API {name}: {message}")]
    Api { name: String, message: String },

    #[error("ошибка OAuth {code}: {description}")]
    OAuth { code: String, description: String },

    #[error("нет авторизации: войдите в аккаунт")]
    Unauthorized,

    #[error("код подтверждения истёк, начните вход заново")]
    DeviceCodeExpired,

    #[error("для трека {0} нет доступного потока")]
    NoStream(String),

    #[error("ошибка хранилища токена: {0}")]
    Storage(String),

    #[error("неожиданный ответ сервера: {0}")]
    Unexpected(String),
}

pub type Result<T> = std::result::Result<T, Error>;
