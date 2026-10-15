# cross-app — план спринтів

> Rust-лаунчер для запуску Windows-ігор та програм на Mac з процесорами Apple M-серії, з підтримкою Steam.
> Дата складання: 14.10.2026. Спринт = 2 тижні. Оцінка розрахована на одного розробника (full-time) або двох part-time.

---

## 0. Чесна рамка проєкту

**Що реально.** Жоден проєкт не переписує Windows API з нуля. Усі робочі рішення на Mac (CrossOver, Whisky, Kegworks, Heroic) — це оркестратори над **Wine**. Rust тут — мова для **ядра, CLI та GUI лаунчера**, а не для емуляції Windows.

**Стек перекладу, який ми оркеструємо:**

| Шар | Сьогодні (2026) | Завтра (2027+) |
|---|---|---|
| Win32 API → macOS | Wine 11.x (GPTK-сумісний білд) | Wine 12 + ARM64EC |
| x86_64 → ARM64 | Rosetta 2 | FEX / власний x86-емулятор у Wine (підхід Hangover, CrossOver ARM64 preview) |
| DirectX 11/12 → Metal | D3DMetal з Apple Game Porting Toolkit 3/4 | D3DMetal + Metal 4 / MetalFX |
| DirectX 9/10/11 → Metal | DXVK → MoltenVK (Vulkan → Metal) | те саме |
| Steam | Windows-клієнт Steam усередині Wine-«пляшки» | те саме |

**Ключові ризики, які формують план:**

1. **Rosetta 2 deprecation.** Apple: Rosetta доступна до macOS 27 включно, у macOS 28 (осінь 2027) залишиться лише підмножина для «старих непідтримуваних ігор». Уся індустрія (CrossOver 27, ARM64-прев'ю серпня 2026) рухається до Wine без Rosetta. Наш план закладає цей перехід у Спринт 8.
2. **Ліцензія GPTK.** D3DMetal не можна редистрибутити. Користувач сам завантажує GPTK з developer.apple.com, а ми імпортуємо бібліотеки (так робили Whisky і його форки).
3. **Анти-чіти.** EAC, BattlEye, Vanguard, Denuvo-online не працюють під Wine на macOS. Це not-a-goal; у GUI треба чесно показувати статус сумісності.
4. **32-бітні ігри.** Wine на macOS працює через WoW64 (без 32-бітного macOS); частина старих ігор лягає. CrossOver 27 узагалі відмовився від 32-біт.
5. **Steam під Wine.** Клієнт на CEF ламається кожні кілька місяців (логін, оверлей, web-helper). Потрібен шар «патчів Steam», який можна оновлювати окремо від релізу лаунчера.
6. **Нотаризація та Gatekeeper.** Wine-бінарі + запуск сторонніх .exe — потрібен Developer ID, hardened runtime з винятками, нотаризація.

**Not a goal (v1):** віртуалізація Windows (Parallels/UTM), DX12 без D3DMetal, онлайн-ігри з анти-чітом, Intel-Mac.

---

## 1. Архітектура (Cargo workspace)

```
cross-app/
├── Cargo.toml                 # workspace
├── crates/
│   ├── cross-core/            # домен: Bottle, Engine, App, Profile; без UI
│   ├── cross-engine/          # завантаження/перевірка Wine-білдів, GPTK import, DXVK/MoltenVK
│   ├── cross-bottle/          # створення prefix, wineboot, реєстр, env, DLL overrides, запуск процесів
│   ├── cross-deps/            # залежності (vcrun, dotnet, fonts, DXVK) — аналог winetricks
│   ├── cross-steam/           # інсталяція Steam, патчі, парсинг VDF/ACF, бібліотека ігр
│   ├── cross-compat/          # база сумісності (профілі на гру, рейтинги), YAML/JSON + sync
│   ├── cross-cli/             # бінар `cross` (clap)
│   └── cross-gui/             # Tauri 2 (Rust backend + веб-фронт) або iced — рішення у Спринті 1
├── engines/                   # кеш завантажених двигунів (~/Library/Application Support/cross-app/)
├── docs/
└── SPRINT_PLAN.md
```

**Основні крейти:** `tokio`, `clap`, `serde` + `toml`, `reqwest`, `tracing`, `thiserror`/`anyhow`, `directories`, `sha2`, `keyvalues-parser` (Steam VDF), `plist`, `notify`, `tauri` 2 + `tauri-plugin-updater`, `sysinfo`.

**Принцип:** ядро не знає про GUI; усе, що робить GUI, можна зробити з CLI. Це дає тестованість і скриптування.

---

## 2. Спринти

### Спринт 0 — Дослідження та ручний PoC (тижні 1–2)
**Мета:** вручну, без жодного нашого коду, пройти весь шлях і зафіксувати відтворюваний рецепт.

- [ ] Встановити Rosetta 2, Homebrew (x86_64 і arm64), зібрати/завантажити GPTK-сумісний Wine (Gcenx wine-crossover або форк Whisky «Wine Libraries»).
- [ ] Імпортувати D3DMetal з GPTK 3/4 dmg вручну (`hdiutil` + `ditto`), задокументувати шляхи бібліотек.
- [ ] Створити prefix, запустити Windows-клієнт Steam, увійти, встановити 1 гру DX11 і 1 гру DX12, запустити обидві.
- [ ] Повторити з DXVK + MoltenVK замість D3DMetal для DX11-гри. Порівняти FPS (Metal HUD).
- [ ] Перевірити CrossOver 27 ARM64 preview: чи працює без Rosetta, на якій підмножині ігор. Записати висновки для Спринту 8.
- [ ] Записати всі env vars, DLL overrides, прапорці Steam (`-noverifyfiles`, `-no-cef-sandbox` тощо), версії.

**Deliverable:** `docs/poc-recipe.md` + відео/скріншоти. **Gate:** якщо DX12 через D3DMetal не запускається на нашій машині — план переглядається.

---

### Спринт 1 — Каркас проєкту та CLI (тижні 3–4)
- [x] Cargo workspace, `cross-core` з доменною моделлю (`Bottle`, `Engine`, `App`, `LaunchProfile`), `cross-cli`.
- [x] Конфіг у TOML: `~/Library/Application Support/cross-app/config.toml`, кожна пляшка — своя `bottle.toml`.
- [x] Логування через `tracing` з файловим апендером на пляшку.
- [x] Детекція середовища: чип (M1–M5), версія macOS, Rosetta встановлена/ні, Xcode CLT, доступний GPTK.
- [x] Рішення по GUI-фреймворку (Tauri 2 за замовчуванням; iced — якщо хочемо pure-Rust). Зафіксувати ADR у `docs/adr/`.
- [ ] CI (workflow написано в `.github/workflows/ci.yml`, ще не запускався — немає git-репо/remote): GitHub Actions на `macos-15`/`macos-26` arm64: fmt, clippy, test.

**Deliverable:** `cross doctor` показує стан системи. `cross --help` описує майбутні команди.

---

### Спринт 2 — Менеджер двигунів (тижні 5–6)
- [x] `cross-engine`: каталог двигунів (JSON-маніфест у репо: назва, версія Wine, URL, SHA-256, вимагає Rosetta: так/ні).
- [x] Завантаження з прогресом, перевірка хешу, розпакування у `engines/<id>/`. _Перевірено наживо 14.10.2026: `cross engine install wine-devel-11.18 --dxvk` (~50 с, SHA-256 збігаються, `wine --version` → `wine-11.18` під Rosetta, повторна установка з кешу, `already installed`, `remove`)._
- [x] Імпорт D3DMetal: користувач вказує `Game_Porting_Toolkit_*.dmg`, ми монтуємо, витягаємо `lib/external/`, зберігаємо версію GPTK.
- [x] Встановлення Rosetta 2 (`softwareupdate --install-rosetta --agree-to-license`) з UI-підтвердженням. _Команда й підтвердження реалізовані; реальний запуск `softwareupdate` не виконувався (Rosetta вже встановлена на цій машині)._
- [x] Варіант DXVK + MoltenVK як окремий «графічний бекенд» двигуна.
- [x] Команди: `cross engine list|install|remove|import-gptk`.

**Deliverable:** будь-який двигун з каталогу ставиться однією командою; GPTK імпортується. _GPTK-збірку Wine від Gcenx свідомо не додано в каталог: вона постачається разом із D3DMetal від Apple (ліцензійне рішення — Спринт 10)._

---

### Спринт 3 — Пляшки та запуск програм (тижні 7–8)
- [x] `cross-bottle`: `create` (wineboot, вибір Windows-версії 10/11, 64-біт), `delete`, `clone`, `rename`.
- [x] Запуск `.exe` / `.msi` / `.lnk` з параметрами, робочою директорією, env (`WINEESYNC`, `WINEMSYNC`, `MTL_HUD_ENABLED`, `WINEDLLOVERRIDES`, `DXVK_HUD`).
- [x] Керування процесами: список процесів пляшки, `kill`, graceful-зупинка. _Відхилення від плану: `wineboot -k` у Wine означає «kill without cleanup», тому graceful = `wineboot -e -s` (просить завершитись програми з вікнами; консольні процеси його ігнорують), `--force` = `wineserver -k`. На macOS `ps` бачить `WINEPREFIX` лише у wineserver, тож «запущена» визначається по ньому, а список процесів — через `wine tasklist`._
- [x] Реєстр: читання/запис ключів через `reg.exe`; пресети (DPI, VirtualDesktop). _Пресет DXVA не зроблено: перевіреного ключа реєстру Wine для нього немає — переноситься в Спринт 7._ _DPI записується в обидва місця (`HKLM\\…\\Hardware Profiles\\Current\\Software\\Fonts` і `HKCU\\Control Panel\\Desktop`); що Wine реально масштабує вікна — не перевірено візуально._
- [x] Доступ до `winecfg`, `regedit`, `cmd`, `taskmgr` однією командою.
- [x] Інтеграційні тести на реальному Wine (позначені `#[ignore]`, запускаються локально та в nightly CI).

**Deliverable:** `cross bottle create demo && cross run demo ~/Downloads/app.exe` працює. _Перевірено наживо на Wine 11.18 (14.10.2026): create, `.bat`/`.exe` за шляхом на хості, Windows-шлях, `--workdir`, env і прапорці, код виходу, ps/kill, пресети, реєстр, clone, rename, delete; 2 інтеграційні тести `cargo test -p cross-bottle --test real_wine -- --ignored`. **Не перевірено на справжніх файлах:** `.msi` і `.lnk` (покрито лише перевіркою побудови команди), nightly-workflow (`.github/workflows/nightly.yml`) ще не запускався._

---

### Спринт 4 — Залежності (аналог winetricks) (тижні 9–10)
- [x] `cross-deps`: декларативні «verbs» у TOML (`crates/cross-deps/verbs.toml`) з URL, хешем, кроками (`run`, `copy`, `override`, `registry`), `requires`/`conflicts`; локальний `verbs.toml` у data-теці перекриває вбудований. _У каталозі поки лише **`vcrun2022`** і **`dxvk`**; `dotnet48`, `corefonts`, `d3dcompiler_47`, `xact` не додано:_
  - _`d3dcompiler_47`: winetricks не дає хешів для його cab-файлів, а вигадувати їх не можна (потрібне власне завантаження й закріплення хешу);_
  - _`corefonts`, `xact`: потребують розпаковки cab (у Wine є `extrac32`/`expand`, крок `extract` ще не написано) і реєстрації шрифтів;_
  - _`dotnet48`: ~120 МБ і відомо крихкий під Wine — без перевірки на справжньому інсталяторі не додаю._
- [x] Кеш завантажених інсталяторів (спільний із двигунами: `cache/downloads/<sha256>-<файл>`), повторна установка офлайн.
- [x] Fallback: системний `winetricks -q <verb>` для verbs поза каталогом, якщо він є в `PATH`/Homebrew (CLI попереджає перед запуском; `--no-winetricks` вимикає). _Перевірено лише з підставним скриптом: справжній winetricks на цій машині не встановлено._
- [x] Стан встановлених залежностей у `bottle.toml` (`[deps.<verb>]`).
- [x] Побічно закрито прогалину Спринту 2: `cross engine install <id> --dxvk` для вже встановленого двигуна доустановлює DXVK + MoltenVK.

**Deliverable:** `cross deps install demo vcrun2022 dxvk` ставить і реєструє залежності. _Перевірено наживо (14.10.2026): `dxvk` (DLL у `system32`/`syswow64` з тими самими SHA-256, що й у компоненті; overrides; стан; повтор; помилки). Що Wine справді малює через DXVK — не перевірялось (Спринт 7). **`vcrun2022` на справжніх інсталяторах не запускався** (потрібно завантажити ~39 МБ з Microsoft) — перевірено лише на підставному wine; pinned-версія 14.36 (2023) старіша за актуальну._
  - _Знахідки під час перевірки: (1) Windows-код виходу під Unix обрізається до 8 біт (3010 → 194, 1638 → 102), тому `ok_exit` порівнюється за модулем 256; (2) «builtin»-збірка DXVK не містить `dxgi.dll`, тож `dxgi` не перевизначається._

---

### Спринт 5 — Steam (тижні 11–12)
- [x] `cross steam install <bottle>`: завантаження `SteamSetup.exe` (закріплений SHA-256, кеш), тиха інсталяція `/S`, застосування патчів зі `steam-patches.toml`. _Патчі взято з verb `steam` у winetricks (Wine bugs 49839, 22053, 44985 + `ShowCrashDialog`); HTTP/2-фікс не додано — не знайшов перевіреного джерела. Якщо інсталятор сам запускає Steam, той зупиняється._
- [x] Окремий канал оновлення `steam-patches.toml` (`cross steam patches --update`, URL у `config.toml` або `--url`; лише https, лише більша `revision`; оновлені registry-патчі доходять до пляшки при наступному запуску). _Межі безпеки для файлу з мережі: реєстр лише в `HKCU\\Software\\{Wine,Valve}`, аргументи — прості `-прапорці`, без `WINEPREFIX`/`PATH`/`DYLD_*`. Публічного URL для каналу ще немає (немає репозиторію)._
- [x] Парсинг `libraryfolders.vdf` (новий і старий формат) та `appmanifest_*.acf` → ігри з appid, назвою, розміром, станом (`cross steam games`). _Власний VDF-парсер замість `keyvalues-parser`: ~200 рядків з тестами, без зовнішньої залежності._
- [x] Запуск гри: `steam.exe -applaunch <appid>` (`cross steam launch`, у фоні, лог у `logs/steam.log`); обкладинки з CDN Steam за appid з кешем (`cross steam covers`).
- [x] Профіль гри: бекенд/env/аргументи в `bottle.toml` (`cross steam profile`). _`dxvk` → native `d3d11`/`d3d10core` (вимагає `cross deps install … dxvk`); `d3dmetal` поки означає вбудований Direct3D Wine — справжнє підключення D3DMetal у Спринті 7. Налаштування не діють, якщо Steam уже запущений — CLI попереджає._
- [x] Відомі проблеми задокументовано в `docs/steam.md` (оверлей, Steam Input, Cloud, CEF/шрифти). _Майже все там позначено «не перевірено»._

**Deliverable:** Steam встановлюється й логіниться з GUI/CLI; бібліотека показується; гра запускається. _Станом на 14.10.2026 перевірено: 26 тестів `cross-steam` (VDF, бібліотеки, патчі й оновлення, установка з підставним wine, профілі, обкладинки) і CLI на справжньому префіксі з синтетичною бібліотекою (два диски через `dosdevices`). **Не перевірено:** справжній `SteamSetup.exe` під Wine, запуск клієнта, логін, запуск гри, завантаження обкладинок (URL перевірено HEAD-запитами). Ризик: winetricks ставить для Steam `corefonts`, якого в нашому каталозі ще немає._

---

### Спринт 6 — GUI (тижні 13–14)
- [ ] Tauri 2: Rust-команди над `cross-core`; фронт — бібліотека ігор/програм (картки з обкладинками), сторінка пляшки, налаштування двигуна, логи в реальному часі.
- [ ] Майстер «перший запуск»: перевірка Rosetta, вибір двигуна, імпорт GPTK, створення першої пляшки.
- [ ] Drag-and-drop `.exe` на вікно → створити ярлик у пляшці.
- [ ] Статуси сумісності (зелений/жовтий/червоний) з `cross-compat`.
- [ ] Нативні дрібниці: меню macOS, dark mode, нотифікації, іконка в Dock з прогресом.

**Deliverable:** користувач без терміналу проходить шлях «встановив → запустив Steam → запустив гру».

---

### Спринт 7 — Графіка, продуктивність, ввід (тижні 15–16)
- [ ] Перемикач бекенду на пляшку/гру: D3DMetal ↔ DXVK/MoltenVK; MetalFX upscaling і Metal 4 encoding як прапорці.
- [ ] Metal HUD / DXVK HUD одним кліком; збір FPS у логи для бенчмарків.
- [ ] `msync`/`esync` вибір; обмеження CPU-ядер для старих ігор.
- [ ] Retina/HiDPI: DPI у реєстрі, режим «ретина» (Wine `RetinaMode`), безрамкове вікно.
- [ ] Геймпади: Xbox/PlayStation/Switch Pro через macOS Game Controller → Wine (SDL/hidraw); тест 3 контролерів.
- [ ] `cross-compat`: формат профілю гри (appid або хеш exe → env, deps, бекенд, відомі баги), community-репозиторій, локальний override.

**Deliverable:** 10 ігор з тестової матриці мають готові профілі «запустилось з коробки».

---

### Спринт 8 — Шлях без Rosetta (тижні 17–18) — стратегічний
**Мета:** не залишитись без двигуна на macOS 28.

- [ ] Дослідити стан Wine ARM64EC на macOS та x86-емуляторів (FEX, Box64, підхід Hangover; що зробив CodeWeavers у ARM64 preview).
- [ ] Спробувати зібрати Wine ARM64EC + FEX на macOS; задокументувати блокери (JIT, W^X, `MAP_JIT`, TSO-режим).
- [ ] Абстракція `Engine.translation = Rosetta | Native(FEX) | External(CrossOver)` у `cross-engine`, щоб нові двигуни додавалися маніфестом.
- [ ] Якщо власний білд нереальний у строки — підтримати CrossOver (встановлений користувачем) як зовнішній двигун: детекція, запуск через його `wine`, імпорт пляшок.
- [ ] Перевірка: Apple «legacy games» режим Rosetta у macOS 27 beta — чи ловить він Wine-процеси (ймовірно ні).

**Deliverable:** ADR «Стратегія після Rosetta» з рішенням і прототипом хоча б одного не-Rosetta запуску (власний або через CrossOver).

---

### Спринт 9 — Пакетування та дистрибуція (тижні 19–20)
- [ ] `.app` bundle через Tauri; вбудований `cross` CLI з символьним лінком у `/usr/local/bin`.
- [ ] Apple Developer ID, hardened runtime з entitlements (`com.apple.security.cs.allow-jit`, `disable-library-validation`, `allow-unsigned-executable-memory` для Wine), нотаризація, stapling.
- [ ] Автооновлення (`tauri-plugin-updater`, підписані маніфести).
- [ ] DMG + Homebrew cask; GitHub Releases з CI.
- [ ] Телеметрія лише opt-in і анонімна (версія macOS, чип, двигун, успіх/провал запуску) для бази сумісності.

**Deliverable:** підписаний DMG ставиться на чистий Mac без попереджень Gatekeeper.

---

### Спринт 10 — Стабілізація та публічна бета (тижні 21–22)
- [ ] Прогнати повну тестову матрицю (розділ 3) на M1, M2/M3 і M4/M5; зафіксувати результати в `cross-compat`. _Не виконано: потрібні справжні ігри, Steam-акаунт власника і Mac кількох поколінь. Підготовлено шаблон [`docs/test-matrix.md`](docs/test-matrix.md) (критерії, процедура, 18 рядків матриці зі статусом «не тестувалось», формат запису, що збігається з шаблоном issue «Звіт про сумісність гри»)._
- [ ] Виправлення top-20 багів бети. _Бету ще не випущено, тож звітів немає. Замість цього проведено полювання на баги до бети (15.10.2026): `cargo test --workspace` (134 тести), 2 інтеграційні тести на Wine 11.18 і ручний прогін CLI на тимчасовому `CROSS_APP_HOME` (~60 сценаріїв: помилки вводу, відсутні пляшки, пошкоджені TOML, реальні create/run/reg/preset/ps/kill/deps/profile/clone/rename/delete). Знайдено й виправлено:_
  - _`cross run <тека>` (зокрема `.app` або тека з назвою `*.exe`) відкривав файловий менеджер Wine (`explorer.exe`) і блокувався до закриття вікна. Тепер CLI відмовляє з поясненням (`cross-cli/src/bottle.rs`). Ту саму перевірку варто перенести в `cross_bottle::launch::build_command` для GUI (drag-and-drop теки)._
  - _`cross deps list --bottle`, `cross steam games|covers|profile` для неіснуючої пляшки показували сиру I/O-помилку замість «bottle does not exist»._
  - _Один пошкоджений `bottle.toml` ламав `cross bottle list` для всіх пляшок; тепер такий файл пропускається з попередженням (`cross-core/src/config.rs`). Те саме з `engine.toml` у `EngineManager::list` **не виправлено** (файл змінює Спринт 8)._
  - _Помилки з причиною друкувались двічі (`I/O error at X: No such file…: No such file…`, а TOML-помилки цілим блоком двічі): наші enum-и і показують `source` у `Display`, і повертають його з `source()`, а `{:#}` в `main.rs` іде по ланцюгу. Додано `cross_core::chain_message`, який пропускає вже надруковані причини. **Діє після заміни одного рядка в `main.rs`**._
  - _Помічено, не виправлено: Wine використовує власну `libMoltenVK` 1.4.0 з архіву Gcenx, а не компонент `moltenvk-1.4.2` (видно з `[mvk-info]` у stderr) — для Спринту 7. Рядок `launching cmd=…` на INFO друкує в термінал повну команду з оточенням — для логування._
- [x] Crash-reporting (локально; відправки немає, тож згода потрібна лише на ручну публікацію звіту). _`cross_core::crash`: panic hook пише `<data>/crashes/crash-<unix>-<pid>.txt` (версія, macOS, чип, Rosetta, потік, місце, повідомлення, backtrace, останні 50 рядків поточного логу), зберігаються 20 останніх, `CROSS_NO_CRASH_REPORTS=1` вимикає. `cross crash list|show|clear` у `crates/cross-cli/src/crash.rs`. Перевірено юніт-тестами (зокрема справжній panic через hook) і наживо в бінарнику з тимчасовим panic. **Не зареєстровано в `main.rs`** (його змінює лід): потрібні `mod crash;`, варіант `Command::Crash`, виклик `cross_core::crash::install(...)` після `logging::init`. Рядки лога, які non-blocking writer ще не скинув, у звіт можуть не потрапити._
- [x] Документація: README, «Швидкий старт», FAQ (анти-чіт, 32-біт, Rosetta), гайд з профілів сумісності. _[`docs/quickstart.md`](docs/quickstart.md), [`docs/faq.md`](docs/faq.md) (також ліцензія GPTK, звіти про аварії, Steam ToS), [`docs/compat-profiles.md`](docs/compat-profiles.md) (концепція; місця, які треба звірити з форматом Спринту 7, позначено «[узгодити зі Спринтом 7]»), README з посиланнями та `cross crash`. Швидкий старт «з нуля» на чистому Mac не проходили._
- [x] Ліцензійна чистота: LGPL-повідомлення Wine, відсутність D3DMetal у дистрибутиві, Steam ToS. _[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md): ліцензії всього, що завантажується (Wine LGPL-2.1+, DXVK zlib, MoltenVK Apache-2.0, SteamSetup — SSA, VC++ — Microsoft) з посиланнями на upstream, 166 Rust-крейтів бінарника з `cargo metadata`. Перевірено, що в репозиторії немає бінарників і адрес Apple, а GPTK лише копіюється з образу користувача. **Відкрито:** файлу `LICENSE` немає (вибір власника); архіви Gcenx не містять текстів ліцензій; ліцензії бібліотек MacPorts у збірці Wine поштучно не звірено; тексти ліцензій Rust-крейтів треба вкласти в `.app` (Спринт 9)._
- [ ] Реліз v0.1.0-beta, канал зворотного зв'язку (GitHub Discussions / Discord). _Не виконано (дії власника: репозиторій, тег, публікація, Discussions). Підготовлено [`docs/release-checklist.md`](docs/release-checklist.md) і шаблони issue у `.github/ISSUE_TEMPLATE/` (помилка, звіт про сумісність; `config.yml` чекає на посилання Discussions)._

**Deliverable:** публічна бета, 5–10 зовнішніх тестувальників, перші community-профілі.

---

### Backlog (після v0.1)
- Інші лаунчери: GOG Galaxy, Epic (через Legendary), Battle.net, Ubisoft Connect.
- Імпорт пляшок з Whisky/CrossOver/Kegworks.
- Оверлей з FPS/темп. (аналог MangoHud) на Metal.
- Steam Cloud / синхронізація сейвів між пляшками.
- Автопідбір профілю за хешем exe + ML-подібні евристики по логу помилок.
- Локалізація (uk, en, de, …).

---

## 3. Тестова матриця (мінімум для v0.1)

| Категорія | Приклади | API | Очікування |
|---|---|---|---|
| DX11, Steam, легка | Hollow Knight, Stardew Valley (Win-білд), Terraria | DX11/DX9 | зелений |
| DX11, Steam, важка | The Witcher 3 (DX11), Skyrim SE, GTA V | DX11 | зелений/жовтий |
| DX12, Steam | Cyberpunk 2077, Elden Ring (офлайн), Baldur's Gate 3 (DX11/Vulkan) | DX12/Vulkan | жовтий |
| Не-Steam програми | Notepad++, 7-Zip, Paint.NET (dotnet), старий інсталятор .msi | Win32/.NET | зелений |
| Анти-чіт | Fortnite, Valorant, Apex | EAC/BE/Vanguard | червоний (очікувано, показати в UI) |
| 32-біт | Age of Empires II HD, старі інді | DX9 32-bit | жовтий/червоний |

Критерій «зелений»: запуск без ручних дій, стабільні 30+ FPS на M1 у 1080p середніх налаштуваннях.

---

## 4. Метрики успіху v0.1
- 80% тестової матриці (без анти-чіту) — зелений/жовтий.
- Шлях «чистий Mac → гра запущена» ≤ 15 хвилин без терміналу.
- Нуль обов'язкових ручних правок реєстру/env для ігор з профілями.
- Є робочий або прототипний не-Rosetta шлях до релізу macOS 28.

---

## 5. Джерела, на яких базуються припущення (жовтень 2026)
- Apple: Rosetta доступна до macOS 27, далі лише підмножина для старих ігор; macOS 26.4 вже показує попередження.
- Game Porting Toolkit 4 (WWDC 2026) у беті, GPTK 3 — поточний реліз; Metal 4 + MetalFX для D3DMetal.
- Whisky закрито у квітні 2025; активний форк (frankea/Whisky) постачає «Wine Libraries» на Wine 11.16 з GPTK-підтримкою.
- CrossOver 27 (червень 2026): тільки Apple Silicon, без 32-біт; ARM64-прев'ю без Rosetta — серпень 2026.
- Wine 11.x dev, Wine 12 очікується на початку 2027.
