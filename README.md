# moth-amp

Неофициальный клиент Яндекс Музыки для Windows и Android: без аналитики и
трекеров, с эквалайзером, кэшем в обычных mp3/flac и своей музыкой в S3.

- **Работа с приложением:** [docs/usage.md](docs/usage.md)
- **Сборка:** [docs/build.md](docs/build.md)
- **S3-хранилище на своём NAS:** [docs/s3-nas.md](docs/s3-nas.md)

## Коротко о сборке

```powershell
cd app
flutter run -d windows                                                     # разработка
powershell -ExecutionPolicy Bypass -File ..\scripts\release.ps1            # тестовые сборки в dist\
```

Нужны Rust (MSVC), Visual Studio Build Tools с C++, Flutter и режим разработчика
Windows — подробно в [docs/build.md](docs/build.md).

## Структура

```
crates/core/   ядро на Rust: Яндекс, волна, кэш и прокси, эквалайзер, S3, секреты
crates/cli/    консольный клиент moth
app/           Flutter-приложение (lib/ — интерфейс, rust/ — мост к ядру)
deploy/nas/    docker-compose для S3 на NAS
docs/          документация
scripts/       сборка тестовых экземпляров
```
