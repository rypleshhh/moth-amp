# moth-amp

Неофициальный клиент Яндекс Музыки для Windows и Android с упором на приватность.

## Структура

```
crates/
  core/   Rust-ядро: модели, Provider, токены, источник «Яндекс»
  cli/    консольный клиент `moth` для проверки ядра без UI
app/      Flutter-приложение (Windows, позже Android)
  lib/    UI и контроллер плеера (media_kit / libmpv)
  rust/   мост к ядру (flutter_rust_bridge)
```

## Приложение

Нужны Flutter SDK, Rust и включённый режим разработчика Windows.

```sh
cd app
flutter run -d windows
```

После изменения функций в `app/rust/src/api/` пересоздайте привязки:

```sh
cd app
flutter_rust_bridge_codegen generate
```

## Своё S3-хранилище на NAS

Музыка, кэш и настройки могут храниться в S3-совместимом хранилище. Как поднять его
на своём NAS (RustFS в Docker): [docs/s3-nas.md](docs/s3-nas.md), готовые файлы —
в `deploy/nas/`.

## Сборка

Нужен Rust (stable) с MSVC-тулчейном.

```sh
cargo test
cargo run -p moth-cli -- login        # вход по коду подтверждения
cargo run -p moth-cli -- status       # аккаунт и Плюс
cargo run -p moth-cli -- likes --limit 10
cargo run -p moth-cli -- playlists
cargo run -p moth-cli -- search "запрос"
cargo run -p moth-cli -- url <track_id>   # прямая ссылка на поток
cargo run -p moth-cli -- logout
```

Токен хранится в системном хранилище (Windows Credential Manager), запись `moth-amp / yandex`.
