# Сборка moth-amp

Как подготовить машину, собрать и запустить moth-amp для Windows и Android.
Команды ниже — для Windows (PowerShell или Git Bash).

## Что установить

| Инструмент | Версия | Зачем |
|---|---|---|
| [Rust](https://rustup.rs) | stable, MSVC-тулчейн | ядро и мост |
| Visual Studio Build Tools 2022 | компонент «Desktop development with C++» | линковка Rust и сборка под Windows |
| [Flutter](https://docs.flutter.dev/get-started/install/windows) | stable 3.47+ | интерфейс |
| `flutter_rust_bridge_codegen` | 2.13.0 | генерация привязок Rust ↔ Dart |
| JDK | 17 | только для Android |
| Android SDK | платформа 36, build-tools 36 | только для Android |

Установка по шагам:

```powershell
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
rustup default stable-msvc
cargo install flutter_rust_bridge_codegen --version 2.13.0 --locked

git clone --depth 1 -b stable https://github.com/flutter/flutter.git C:\src\flutter
# добавить C:\src\flutter\bin в PATH пользователя
flutter --disable-analytics
```

**Режим разработчика Windows** обязателен: без него Flutter не собирает плагины
(нужны символические ссылки). «Параметры → Для разработчиков → Режим разработчика».

После установки перезапустите терминал и VS Code, чтобы подхватился новый `PATH`.
Проверка: `flutter doctor` — пункты Flutter, Windows и Visual Studio должны быть с ✓.

### Android (по желанию)

1. JDK 17 — например, переносной zip от Microsoft
   (`https://aka.ms/download-jdk/microsoft-jdk-17-windows-x64.zip`) в `C:\Android\jdk17`.
2. [Command-line tools](https://developer.android.com/studio#command-tools) в
   `C:\Android\sdk\cmdline-tools\latest`, затем:

   ```powershell
   $env:JAVA_HOME = 'C:\Android\jdk17'
   & C:\Android\sdk\cmdline-tools\latest\bin\sdkmanager.bat --sdk_root=C:\Android\sdk --licenses
   & C:\Android\sdk\cmdline-tools\latest\bin\sdkmanager.bat --sdk_root=C:\Android\sdk "platform-tools" "platforms;android-36" "build-tools;36.0.0"
   flutter config --android-sdk C:\Android\sdk --jdk-dir C:\Android\jdk17
   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android
   ```

3. NDK Gradle скачает сам при первой сборке (несколько минут).

## Структура репозитория

```
crates/core/   ядро на Rust: Яндекс, волна, кэш и прокси, эквалайзер, S3, секреты
crates/cli/    консольный клиент moth — проверка ядра без интерфейса
app/           Flutter-приложение
  lib/         интерфейс и плеер (media_kit / libmpv)
  rust/        мост к ядру (flutter_rust_bridge)
deploy/nas/    docker-compose для своего S3 на NAS
docs/          документация
scripts/       сборка тестовых экземпляров
```

## Запуск для разработки

```powershell
cd app
flutter run -d windows
```

Отладочная сборка медленнее и тяжелее релизной (около 270 МБ памяти против ~110 МБ).

### Если меняли функции моста

Всё, что лежит в `app/rust/src/api/`, видно из Dart через сгенерированные привязки.
После изменения сигнатур:

```powershell
cd app
flutter_rust_bridge_codegen generate
```

## Тесты и проверки

```powershell
cargo test --workspace                 # тесты ядра
cargo clippy --workspace --all-targets
cd app; flutter analyze
```

**Сквозные тесты S3** (нужен Docker):

```powershell
docker run -d --rm --name moth-s3-test -p 9000:9000 `
  -e RUSTFS_ACCESS_KEY=moth -e RUSTFS_SECRET_KEY=mothtest123 rustfs/rustfs
$env:MOTH_S3_TEST = 'http://127.0.0.1:9000'
cargo test -p moth-core s3_ -- --ignored
docker stop moth-s3-test
```

## Консольный клиент

Удобен, чтобы проверить API без интерфейса. Токен общий с приложением.

```powershell
cargo run -p moth-cli -- login                  # вход по коду подтверждения
cargo run -p moth-cli -- status                 # аккаунт и Плюс
cargo run -p moth-cli -- likes --limit 10
cargo run -p moth-cli -- playlists
cargo run -p moth-cli -- search "запрос"
cargo run -p moth-cli -- wave                   # «Моя волна», тихий режим
cargo run -p moth-cli -- download <id>          # скачать трек с тегами во временную папку
cargo run -p moth-cli -- url <id>               # прямая ссылка на поток
cargo run -p moth-cli -- logout
```

## Тестовые сборки

```powershell
powershell -ExecutionPolicy Bypass -File scripts\release.ps1               # всё
powershell -ExecutionPolicy Bypass -File scripts\release.ps1 -SkipAndroid  # только Windows
```

Результат — в `dist\`:

| Файл | Что это |
|---|---|
| `moth-amp-<версия>-windows-x64\` | папка: распаковал и запустил `moth_amp.exe`, установка не нужна |
| `moth-amp-<версия>-windows-x64.zip` | то же в архиве |
| `moth-amp-<версия>-android-arm64-v8a.apk` | для современных телефонов |
| `…-armeabi-v7a.apk` | старые 32-битные телефоны |
| `…-x86_64.apk` | эмуляторы и x86-планшеты |

Первая сборка под Android долгая (NDK, Rust под четыре архитектуры), следующие — пара минут.

**Подпись APK.** Тестовые APK подписаны отладочным ключом Flutter. Для публикации
нужен свой ключ: создать keystore (`keytool -genkey …`), описать его в
`app/android/key.properties` и подключить в `app/android/app/build.gradle.kts`.

### Установка на телефон

- Скопировать APK на телефон и открыть; разрешить установку из этого источника.
- Или по USB с включённой отладкой: `C:\Android\sdk\platform-tools\adb.exe install -r dist\moth-amp-<версия>-android-arm64-v8a.apk`.

## Частые ошибки

| Что видно | Решение |
|---|---|
| `cargo`/`flutter` не найдены | Перезапустить VS Code целиком (терминал берёт `PATH` при запуске редактора). |
| `Building with plugins requires symlink support` | Включить режим разработчика Windows. |
| `cannot open file … .exe` / `MSB3073` при сборке | Закрыть запущенный moth-amp — сборка не может перезаписать файлы. |
| Ошибки компиляции в `frb_generated.rs` | Пересоздать привязки: `flutter_rust_bridge_codegen generate`. |
| Android: `SDK location not found` | `flutter config --android-sdk C:\Android\sdk`. |
