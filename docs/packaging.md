# Пакетування та дистрибуція (Спринт 9)

Тут описано, як cross-app збирається в реліз: профіль збирання, будова `.app`, `cross` на `PATH`, підпис, нотаризація, DMG, Homebrew, GitHub Releases і автооновлення.

> **Статус (15.10.2026).** GUI (Спринт 6) у цій гілці ще немає, Apple Developer ID на машині розробки теж. Тому:
> - **перевірено наживо:** release-збирання CLI, справжній DMG (створення, `hdiutil verify`, монтування, запуск `cross` з образу, від'єднання), `link-cli.sh` (symlink, `--copy`, `--uninstall`, відмова лінкувати з `/Volumes`), рендер Homebrew-шаблонів і їх завантаження через DSL Homebrew 7.0.2, `plutil -lint` entitlements, dry-run усіх скриптів підпису й нотаризації;
> - **не перевірено:** справжній підпис Developer ID, нотаризація, stapling, Gatekeeper на чистому Mac, `release.yml` на GitHub (remote немає), `.app` від Tauri і його оновлення. Скрипти для цього написано за `man codesign`, `xcrun notarytool --help`, `xcrun stapler` і документацією Apple, але жодного разу не запущено по-справжньому. `shellcheck` на машині немає, тому скрипти перевірено лише `bash -n` (bash 3.2 з macOS) і прогонами.

## Файли

| Шлях | Що робить |
|---|---|
| `Cargo.toml` → `[profile.release]` | `lto = "thin"`, `codegen-units = 1`, `strip = "symbols"`: бінарник ~5,3 МБ проти ~33 МБ у debug |
| `packaging/macos/common.sh` | спільні функції: `DRY_RUN`, `DIST_DIR`, перевірка змінних |
| `packaging/macos/build-cli.sh` | `cargo build --release --locked -p cross-cli --target aarch64-apple-darwin`, копія в `dist/`, tar.gz; `--archive-only` перепаковує після підпису |
| `packaging/macos/entitlements.plist` | винятки hardened runtime для Wine |
| `packaging/macos/sign.sh` | `codesign` для бінарника, `.app` (зсередини назовні) і `.dmg` |
| `packaging/macos/notarize.sh` | `notarytool submit --wait`, `stapler staple/validate`, `spctl` для DMG |
| `packaging/macos/make-dmg.sh` | DMG через `hdiutil`; макет для CLI зараз, для `.app` через `APP_BUNDLE` |
| `packaging/macos/link-cli.sh` | кладе `cross` у `PATH`; їде всередині DMG |
| `packaging/macos/dmg-readme*.txt` | README у корені образу (для CLI і для GUI) |
| `packaging/homebrew/*.rb.in`, `render.sh` | шаблони formula і cask та їх заповнення |
| `.github/workflows/release.yml` | тег `v*` → перевірки, збирання, пакування, чернетка релізу |

Усі скрипти приймають `DRY_RUN=1`: тоді вони друкують команди замість виконання. `DIST_DIR` змінює теку результатів (за замовчуванням `dist/`, вона в `.gitignore`).

## Локальне збирання

```bash
packaging/macos/build-cli.sh        # dist/cross, dist/cross-0.0.1-aarch64-apple-darwin.tar.gz
packaging/macos/make-dmg.sh         # dist/cross-app-0.0.1-arm64.dmg
```

`make-dmg.sh` після створення монтує образ лише для читання (`-readonly -nobrowse`), перевіряє вміст, запускає `cross --version` з образу і від'єднує його. Від'єднання гарантує `trap`, навіть якщо перевірка впала. `VERIFY_MOUNT=0` вимикає монтування, `DMG_FORMAT=ULFO` перемикає стиснення (за замовчуванням `UDZO`).

Без GUI образ містить `cross`, `link-cli.sh` і `README.txt`. Перевірено 14–15.10.2026: образ ~3,2 МБ, `hdiutil verify` → VALID, після скрипта жодного змонтованого образу не лишається.

## Будова `.app` (план для Спринту 6)

```text
<App>.app/Contents/
  Info.plist
  MacOS/<gui-binary>      головний бінарник Tauri (не можна називати його `cross`)
  MacOS/cross             CLI: Tauri копіює сюди bundle.externalBin
  Resources/              фронтенд, іконки
```

Tauri кладе `externalBin` саме в `Contents/MacOS`, відрізає суфікс платформи (`cross-aarch64-apple-darwin` → `cross`) і підписує його разом з усім іншим, до підпису самого бандла. Це перевірено за кодом `tauri-bundler` (`bundle/macos/app.rs`). Файл має називатися з суфіксом цільової платформи: `binaries/cross-aarch64-apple-darwin`. Його готує `TAURI_SIDECAR_DIR=crates/cross-gui/src-tauri/binaries packaging/macos/build-cli.sh`.

Фрагмент `tauri.conf.json` для Спринту 6 (ключі звірено з довідником конфігурації Tauri 2):

```json
{
  "bundle": {
    "externalBin": ["binaries/cross"],
    "createUpdaterArtifacts": true,
    "macOS": {
      "minimumSystemVersion": "14.0",
      "hardenedRuntime": true,
      "entitlements": "../../../packaging/macos/entitlements.plist"
    }
  },
  "plugins": {
    "updater": {
      "pubkey": "<вміст публічного ключа>",
      "endpoints": ["https://github.com/<owner>/<repo>/releases/latest/download/latest.json"],
      "requireSignedVersion": true
    }
  }
}
```

Шлях до `entitlements` вказується відносно `src-tauri/`, тож його треба звірити, коли з'явиться реальна тека.

Symlink у `PATH` указує всередину `.app`. Автооновлення Tauri замінює бандл цілком, тому лінк і далі веде на свіжий CLI. Копія так не вміє.

## `cross` у PATH

```bash
link-cli.sh                 # ~/.local/bin/cross -> CLI всередині /Applications/*.app (без прав адміністратора)
link-cli.sh --copy          # копія замість лінка: для DMG без GUI
link-cli.sh --system        # /usr/local/bin: ПОТРІБНІ ПРАВА АДМІНІСТРАТОРА (sudo, лише якщо тека не доступна на запис)
link-cli.sh --uninstall     # прибирає лише symlink або бінарник cross, чужий файл не чіпає
```

За замовчуванням використовується `~/.local/bin`, бо туди можна писати без `sudo`. Якщо теки немає в `PATH`, скрипт друкує рядок для `~/.zprofile`. Скрипт попереджає, якщо `cross` уже знаходиться раніше в `PATH`, наприклад `~/.cargo/bin/cross` після `cargo install`. Лінкувати з `/Volumes/...` скрипт відмовляється, бо образ зникне після від'єднання.

GUI зможе робити те саме пунктом меню «Install command line tool», тим самим скриптом або його Rust-аналогом.

## Підпис і hardened runtime

Ключі `entitlements.plist` звірено з документацією Apple (Bundle Resources → Entitlements):

| Ключ | Навіщо Wine |
|---|---|
| `com.apple.security.cs.allow-jit` | пам'ять W+X через `MAP_JIT` |
| `com.apple.security.cs.allow-unsigned-executable-memory` | W+X без `MAP_JIT`: Wine мапить PE-образи як записувані й виконувані |
| `com.apple.security.cs.disable-library-validation` | завантаження DXVK/MoltenVK/D3DMetal і бібліотек Wine, які підписані не нами |

Важливе застереження: **сам `cross` цих винятків не потребує**. Він лише запускає Wine, а hardened runtime діє на процес з підписаним бінарником і не успадковується дочірніми процесами. Двигуни Wine завантажуються під час роботи в `~/Library/Application Support/cross-app/engines` і зберігають власні підписи. Тож entitlements почнуть реально впливати тоді, коли Wine їхатиме всередині нашого підписаного бандла. Для `cross` тоді варто розглянути мінімальний набір без винятків. Кандидати, які ще не додано і які треба перевіряти на реальному Wine: `com.apple.security.cs.allow-dyld-environment-variables` і `com.apple.security.cs.disable-executable-page-protection` (обидва ключі існують у документації Apple).

```bash
SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" packaging/macos/sign.sh dist/cross
SIGN_IDENTITY="..." packaging/macos/sign.sh dist/cross-app-0.0.1-arm64.dmg
SIGN_IDENTITY=- packaging/macos/sign.sh dist/cross     # ad-hoc: лише для локальної перевірки
```

Для коду скрипт викликає `codesign --force --sign … --timestamp --options runtime --entitlements …`, після чого `codesign --verify --strict --verbose=2`. DMG підписується без runtime і entitlements. У `.app` спершу підписуються вкладені Mach-O з `Contents/MacOS` (крім головного) і `Contents/Frameworks`, потім сам бандл, бо `--deep` для підпису застарів (`man codesign`). Додаткові змінні: `SIGN_KEYCHAIN` (CI), `CLI_IDENTIFIER` (`codesign -i`; без нього ідентифікатором стає ім'я файлу `cross`).

Зараз `dist/cross` має лише ad-hoc-підпис лінкера (`flags=0x20002(adhoc,linker-signed)`). Gatekeeper такий бінарник із завантаженого DMG заблокує.

## Нотаризація і stapling

```bash
xcrun notarytool store-credentials cross-app-notary   # один раз, інтерактивно (дія користувача)
NOTARY_PROFILE=cross-app-notary packaging/macos/notarize.sh dist/cross-app-0.0.1-arm64.dmg
```

Облікові дані перевіряються в такому порядку. Імена змінних ті самі, що читає бандлер Tauri:

1. `NOTARY_PROFILE`: профіль у Keychain;
2. `APPLE_API_KEY_PATH` + `APPLE_API_KEY` + `APPLE_API_ISSUER`: ключ App Store Connect API (рекомендовано для CI);
3. `APPLE_ID` + `APPLE_PASSWORD` (пароль застосунку) + `APPLE_TEAM_ID`.

Скрипт виконує `notarytool submit --wait --timeout 1h --output-format json`, читає `status` з JSON через `plutil`, при `Invalid` забирає `notarytool log`, потім робить `stapler staple` і `stapler validate`, а для DMG ще `spctl --assess --type open --context context:primary-signature -v`. У dry-run пароль замінюється на `<redacted>`.

Обмеження з документації інструментів: голий Mach-O не можна ні подати, ні «степлувати» (`stapler` підтримує лише UDIF-образи, підписані бандли і flat-пакети). Тому CLI поширюється всередині нотаризованого DMG. Нотаризованим вважається й сам бінарник, бо квиток покриває вміст образу, але для копії поза образом Gatekeeper перевіряє квиток онлайн.

**Не перевірено:** точні JSON-ключі `id`/`status` у виводі `notarytool` (взято з документації; `plutil -extract` на такому JSON перевірено). Прапорець `--context` для `spctl` відсутній у локальному `man spctl`, але `spctl` на macOS 26.5 його приймає і відхиляє непідписаний образ з `source=no usable signature`.

## Homebrew

- `packaging/homebrew/cross-app-cli.rb.in`: formula, що збирає CLI з вихідного архіву тегу (`depends_on "rust" => :build`, `cargo install *std_cargo_args(path: "crates/cross-cli")`). Тест працює без мережі й Wine: `cross --version` і `cross bottle list --json` → `[]`.
- `packaging/homebrew/cross-app.rb.in`: cask для DMG з GUI (`app`, `binary` на `Contents/MacOS/cross`, `zap`); має сенс лише після Спринту 6.

```bash
packaging/homebrew/render.sh packaging/homebrew/cross-app-cli.rb.in dist/cross-app-cli.rb \
  VERSION=0.1.0 URL=<архів тегу> SHA256=<sha256 архіву> HOMEPAGE=<репо> HEAD_URL=<репо>.git
```

`render.sh` вимагає заповнити всі плейсхолдери, відхиляє лапки й зворотні слеші в значеннях і перевіряє результат через `ruby -c`. Обидва шаблони після рендеру завантажено через DSL Homebrew 7.0.2 (formula: залежність `rust`, вимоги arm64/macOS 14; cask: `app`, `binary`, `zap`). `brew audit`/`brew style` не запускались: вони потребують tap і доустановлення гемів.

Обидва варіанти ставлять команду `cross`. Cask у Homebrew 7 вміє `conflicts_with` лише для cask, тож конфлікт formula/cask описано коментарем. Публікувати варто у власний tap: офіційний `homebrew/cask` вимагає, щоб застосунок проходив Gatekeeper. URL репозиторію ще немає, тому в шаблонах лише плейсхолдери.

## GitHub Actions: `release.yml`

На тег `v*`:

1. перевіряє, що тег збігається з версією в `Cargo.toml`;
2. запускає `fmt`, `clippy`, `test`;
3. збирає CLI;
4. **якщо є секрети підпису**: створює тимчасовий keychain (кроки з документації Tauri, прапорці `security` звірено з `security help`), підписує CLI і перепаковує tar.gz;
5. збирає DMG; без секретів він називається `…-unsigned.dmg` і в лозі з'являється warning;
6. **якщо є секрети підпису**: підписує DMG; **якщо є ще й секрети нотаризації**: нотаризує й «степлує» його;
7. рахує `SHA256SUMS` після stapling, бо stapling змінює файл;
8. рендерить formula з архіву тегу (`https://github.com/<repo>/archive/refs/tags/<tag>.tar.gz`);
9. вивантажує артефакти і створює **чернетку** релізу (`gh release create --draft --verify-tag --generate-notes`). Публікує її людина;
10. видаляє keychain (`if: always()`).

`workflow_dispatch` проганяє те саме з будь-якої гілки без релізу. Секрети не можна читати в `if:` кроку, тож їхня наявність передається через `env` (`HAVE_SIGNING`, `HAVE_NOTARY`). Список секретів наведено в заголовку файлу. YAML перевірено парсером Ruby; `actionlint` на машині немає, а сам workflow ще не запускався (remote немає). Гілку без підпису (build → DMG `-unsigned` → `SHA256SUMS`) прогнано локально тими самими командами.

## Автооновлення (`tauri-plugin-updater`)

Звірено з документацією Tauri 2 (v2.tauri.app/plugin/updater) і з кодом плагіна в гілці `v2` (`tauri-plugin-updater` 2.14.0, `src/updater.rs`, `src/config.rs`). Коли Спринт 6 закріпить версію, звірити ще раз.

**Ключі** (дія користувача; приватний ключ не потрапляє в репозиторій):

```bash
cargo tauri signer generate -w ~/.tauri/cross-app.key    # потрібен tauri-cli 2.x
```

Команда створює пару ключів minisign: `~/.tauri/cross-app.key` (приватний, з паролем) і `~/.tauri/cross-app.key.pub`. Вміст `.pub` вставляється в `plugins.updater.pubkey`; шлях до файлу там не працює. Під час `tauri build` потрібні `TAURI_SIGNING_PRIVATE_KEY` (шлях або вміст) і `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; `.env`-файли не підходять. Якщо ключ загубиться, наявні встановлення більше не зможуть оновитися.

**Артефакти** з `bundle.createUpdaterArtifacts: true` на macOS: `<App>.app.tar.gz` і `<App>.app.tar.gz.sig`.

**Статичний маніфест** (`latest.json`, кладеться в GitHub Release):

```json
{
  "version": "0.1.0",
  "notes": "…",
  "pub_date": "2026-11-01T12:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "url": "https://github.com/<owner>/<repo>/releases/download/v0.1.0/<App>.app.tar.gz",
      "signature": "<вміст файлу .app.tar.gz.sig>"
    }
  }
}
```

- `version` у SemVer, можна з префіксом `v`; `pub_date` у RFC 3339; `notes` і `pub_date` необов'язкові.
- `signature` містить сам вміст `.sig`, а не шлях і не URL. Перед перевіркою підпис і `pubkey` декодуються з base64.
- Плагін шукає ключ `darwin-aarch64-app` (`{os}-{arch}-{bundle_type}`), потім `darwin-aarch64`. Маніфест валідується цілком, тож кожен запис платформи має бути повним.
- В URL ендпоінтів підставляються `{{current_version}}`, `{{target}}` (`darwin`), `{{arch}}` (`aarch64`), а в 2.14 ще й `{{bundle_type}}`. Ендпоінти мусять бути HTTPS.
- `requireSignedVersion: true` змушує перевіряти версію з довіреного коментаря підпису. Без цього змінений маніфест може видати старий, але валідно підписаний пакет за новий. Працює лише з підписами Tauri CLI, що записують версію (`--app-version`); інакше маємо `MissingSignedVersion`. Під час інтеграції перевірити, що закріплена версія CLI це вміє.

**Інтеграція, коли з'явиться Спринт 6:**

1. `cargo add tauri-plugin-updater --target 'cfg(any(target_os = "macos", windows, target_os = "linux"))'` у `crates/cross-gui/src-tauri`, JS-пакет `@tauri-apps/plugin-updater`.
2. У `setup`: `app.handle().plugin(tauri_plugin_updater::Builder::new().build())` під `#[cfg(desktop)]`.
3. У `capabilities/default.json` додати `"updater:default"`.
4. У `tauri.conf.json` вказати конфіг вище (`createUpdaterArtifacts`, `pubkey`, `endpoints`, `requireSignedVersion`).
5. У `release.yml` на місці коментаря «Sprint 6 hook» зібрати `.app` з `TAURI_SIGNING_PRIVATE_KEY*` і `APPLE_*`, передати його в `make-dmg.sh` через `APP_BUNDLE`, згенерувати `latest.json` з `.sig` і додати `latest.json`, `.app.tar.gz`, `.app.tar.gz.sig` до релізу.
6. Порядок має значення: `.app.tar.gz` треба пакувати з уже підписаного й нотаризованого `.app`, з квитком. Бандлер Tauri робить це сам, якщо підпис і нотаризація йдуть через нього (змінні `APPLE_*`).

## Що потребує дії користувача

- Apple Developer Program і сертифікат **Developer ID Application** (експорт у `.p12`).
- `xcrun notarytool store-credentials` або ключ App Store Connect API для нотаризації.
- Перший справжній підпис, нотаризація і перевірка DMG на чистому Mac (Gatekeeper без попереджень: це deliverable спринту).
- Створити GitHub-репозиторій і remote, додати секрети з заголовка `release.yml`, поставити й запушити перший тег.
- Згенерувати ключ оновлень Tauri і зберегти приватний ключ та пароль у сховищі секретів.
- Створити Homebrew tap і опублікувати відрендерену formula, а після GUI ще й cask.
- Вирішити питання ендпоінта телеметрії і політики приватності, перш ніж щось надсилати (див. [telemetry.md](telemetry.md)).
