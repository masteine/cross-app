# GUI cross-app

Десктопний застосунок на Tauri 2 (крейт `crates/cross-gui`, бінарник `cross-app`). Він робить те саме, що й CLI `cross`, через ті самі крейти, і зберігає дані в тій самій теці (`~/Library/Application Support/cross-app/` або `CROSS_APP_HOME`).

> **Статус: Спринт 6, рання версія.** Командний шар перевірено тестами, зокрема на справжньому Wine 11.18. Саме вікно (WKWebView, меню, Dock, нотифікації, drag-and-drop) ще не запускалося. Деталі в розділі [Що перевірено](#що-перевірено).

## Збирання і запуск

GUI входить у workspace, але не в `default-members`. Tauri тягне близько 400 крейтів, тож звичайні `cargo build` і `cargo test` його пропускають. Команди з `--workspace` або `-p cross-gui` його збирають.

```bash
cargo build -p cross-gui                 # збірка (Node і npm не потрібні)
cargo test -p cross-gui                  # тести командного шару, вікно не відкривається
cargo run -p cross-gui                   # відкриває вікно cross-app
CROSS_APP_HOME=/tmp/cross-test cargo run -p cross-gui   # окремий набір даних
```

Крейт вимагає Rust 1.90+ (MSRV Tauri 2.12), решта workspace — 1.80.

Інтеграційний тест на справжньому Wine (як у `cross-bottle`). Він створює пляшку, додає `.bat`-ярлик, запускає його, читає лог, змінює версію Windows, зупиняє й видаляє пляшку. Вікна не відкриває:

```bash
CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app \
  cargo test -p cross-gui --test real_wine -- --ignored --test-threads=1
```

`.app`-бандл, підпис і DMG — це Спринт 9. Для них знадобиться `tauri-cli` (`cargo install tauri-cli --version "^2" --locked`), іконка `.icns` і справжній bundle identifier. Зараз у `tauri.conf.json` стоїть `"bundle": { "active": false }` і тимчасовий identifier `dev.cross-app.gui`.

## Що вміє

| Екран | Що є |
|---|---|
| **Бібліотека** | картки ігор Steam з усіх пляшок (обкладинки з `cache/covers`, на першому показі завантажуються з CDN Steam) і ярлики програм; кнопка «Play»: гра через `steam.exe -applaunch`, Steam-клієнт з патчами, програма через `wine`; фільтр; бейдж сумісності |
| **Пляшки** | список зі станом (running/stopped); створення: назва, id, двигун, Windows 7–11 |
| **Сторінка пляшки** | двигун, версія Windows (реєстр + `bottle.toml`), ярлики (запуск, видалення, «Add program…»), установка й запуск Steam, залежності з каталогу, процеси (`wine tasklist`), Stop / Force stop, «Run program…», winecfg / regedit / taskmgr / cmd, видалення пляшки з підтвердженням |
| **Двигуни** | каталог і встановлені двигуни, установка з прогресом, «Add DXVK», двигун за замовчуванням, видалення; імпорт GPTK з `.dmg` або змонтованого тому |
| **Логи** | файли з `bottles/<id>/logs/` або глобальних `logs/`, оновлення раз на секунду (як `tail -f`) |
| **Майстер першого запуску** | відкривається, поки немає жодної пляшки. Кроки: перевірка системи (`cross doctor`) з установкою Rosetta 2 після підтвердження, вибір і установка двигуна, необов'язковий імпорт GPTK, перша пляшка. Його можна пропустити, а потім відкрити знову через Help → First-Run Setup |
| **Drag-and-drop** | `.exe`/`.msi`/`.lnk`/`.bat`/`.cmd`, кинутий на вікно, стає ярликом (`[[apps]]` у `bottle.toml`) у вибраній пляшці. Файли з `drive_c` пляшки зберігаються як `C:\...`, тож ярлик переживає clone/rename |

Довгі операції (двигуни, Steam, залежності, створення пляшки) показують прогрес у куті вікна та на іконці в Dock. Якщо вікно не у фокусі, після завершення з'являється нотифікація.

Програми, запущені з GUI, пишуть stdout/stderr у `bottles/<id>/logs/<назва>.log`, тож їх видно на сторінці логів. Власний лог GUI лежить у глобальних `logs/` (`cross.log.<дата>`), як і лог CLI.

### Меню macOS

| Меню | Пункти |
|---|---|
| cross-app | About, Engines and Settings… (⌘,), Services, Hide, Quit |
| File | New Bottle… (⌘N), Add Program to Bottle… (⌘O), Close Window |
| Edit | Undo, Redo, Cut, Copy, Paste, Select All (без них не працює копіювання в полях вводу) |
| View | Library (⌘1), Bottles (⌘2), Engines (⌘3), Logs (⌘4), Refresh (⌘R), Full Screen |
| Window | Minimize, Zoom |
| Help | First-Run Setup… |

## Будова

```text
crates/cross-gui/
  src/api/       командний шар: звичайний Rust над cross-*, серіалізовані «view»-структури, тести
  src/app/       Tauri: команди (тонкі обгортки над api), події, меню, Dock, нотифікації
  ui/            фронтенд: index.html, styles.css, js/ (ES-модулі, без збірки; ADR 0002)
  capabilities/  дозволи вікна: лише core:default
  tauri.conf.json
```

- Кожна команда виконується поза головним потоком (`spawn_blocking`).
- Довгі команди отримують від сторінки `task`-id і шлють події:
  - `task://started` `{task, title}`;
  - `task://progress` — з `cross_engine::Progress`: `{task, label, stage, done, total, percent}`, не частіше ніж раз на 150 мс на крок;
  - `task://finished` `{task, title, ok, error}`.
- Інші події:
  - `process://exited` — коли завершилась програма, запущена з GUI;
  - `menu` — id пункту меню;
  - вбудовані `tauri://drag-enter|drag-leave|drag-drop` Tauri.
- **Сумісність.** Трейт `CompatSource` (`src/api/compat.rs`) з рейтингами green / yellow / red / unknown. Поки працює заглушка `NoCompatData`, тож усі бейджі показують «Unknown». Крейт `cross-compat` (Спринт 7) підключається реалізацією трейту в `Gui::compat` (`src/app/mod.rs`).

## Що перевірено

Станом на 15.10.2026:

- **Тести.** 29 юніт-тестів командного шару, кожен з тимчасовим `CROSS_APP_HOME` і підставним wine: бібліотека, ярлики, запуск у фоні з логом, логи, двигуни, GPTK з теки, пляшки, прогрес, Dock-стан. Ще 1 інтеграційний тест на справжньому Wine 11.18 (див. вище) пройшов за 49 с.
- **Конфігурація Tauri** звірена з кодом 2.12.4:
  - `app.withGlobalTauri`;
  - `Window::set_progress_bar` — на macOS це індикатор на плитці Dock (`tao` 0.37, `platform_impl/macos/progress_bar.rs`), `Indeterminate` там показується як звичайний;
  - події drag-and-drop `tauri://drag-*` з полем `paths`;
  - `core:default`.
- **Фронтенд** переглянуто у вбудованому браузері (Chromium, не WKWebView) з підставним `window.__TAURI__`:
  - усі сторінки, майстер, прогрес задачі, діалог drop, світла й темна тема;
  - знайдено й виправлено дві помилки рендеру.

**Не перевірено:**

- запуск самого вікна (`cargo run -p cross-gui`): WKWebView, меню, ⌘-скорочення, нативні діалоги (вибір файлу, підтвердження), справжній drag-and-drop з Finder;
- **нотифікації**: `tauri-plugin-notification` на macOS використовує bundle identifier. Для не-бандленого бінарника (`cargo run`) вони можуть не з'являтися, тож імовірно запрацюють лише з `.app` (Спринт 9);
- **прогрес у Dock** візуально;
- установка Rosetta з GUI: `softwareupdate` запускається без терміналу. Якщо він попросить пароль адміністратора, GUI покаже команду для Terminal. На цій машині Rosetta вже стоїть;
- завантаження обкладинок із CDN у вікні, запуск Steam і ігор (як і в CLI, див. `docs/steam.md`).

## Відомі обмеження

- **Вивід Steam-клієнта.** Кнопка «Start Steam» викликає `cross_steam::launch::run_client`, який успадковує stdio, тому вивід клієнта не потрапляє в логи. Ігри, запущені через «Play», пишуть у `logs/steam.log`. Для повноцінного логу потрібна фонова функція запуску клієнта в `cross-steam`.
- **winetricks** для verbs поза каталогом у GUI вимкнено, лишається тільки CLI. Попередження про сторонній скрипт ще не має UI.
- **Профілі ігор** (`cross steam profile`) у GUI лише враховуються (`has_profile`), редагувати їх можна тільки через CLI.
- **Логи GUI** пишуться в глобальні `logs/`, а не в лог пляшки. Підписка tracing одна на процес.
- **Залежність `toml`.** Через GUI весь workspace фіксує `toml` 0.8.2 замість 0.8.23. Ланцюжок `tauri → gtk 0.18 → glib-macros → proc-macro-crate 2.0.2` вимагає `toml_datetime =0.6.3`, а `Cargo.lock` спільний для всіх цілей, хоча gtk потрібен лише на Linux. Усі тести workspace проходять з 0.8.2.
