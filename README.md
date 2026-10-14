# cross-app

Лаунчер для Windows-ігор і програм на Mac з Apple Silicon (M1–M5), з підтримкою Steam. Написаний на Rust.

cross-app не емулює Windows сам. Він керує перевіреним стеком і забирає з нього ручну роботу: завантаження й перевірку двигунів, створення «пляшок», залежності, патчі Steam.

| Шар | Чим закрито |
|---|---|
| Windows API → macOS | [Wine](https://www.winehq.org/) (збірки [Gcenx](https://github.com/Gcenx/macOS_Wine_builds)) |
| x86_64 → ARM64 | Rosetta 2 |
| Direct3D 9/10/11 → Metal | DXVK + MoltenVK |
| Direct3D 11/12 → Metal | D3DMetal з Apple Game Porting Toolkit (імпорт є, підключення до запуску ще в роботі) |

> **Статус: рання розробка (v0.0.1), лише CLI.** Готові Спринти 1–5 з [плану](SPRINT_PLAN.md): двигуни, пляшки, запуск програм, залежності, Steam. GUI, перемикання графічних бекендів і пакетування ще попереду. Що перевірено, а що ні, описано в [Обмеженнях](#обмеження).

---

## Вимоги

- Mac з **Apple Silicon** (Intel не підтримується)
- **macOS 14+**, рекомендовано 15+
- **Rosetta 2**: потрібна поточним двигунам Wine. cross-app може її поставити.
- **Xcode Command Line Tools**: `xcode-select --install`
- **Rust** (stable): потрібен лише для збирання з коду
- Близько 1 ГБ вільного місця на двигун Wine, плюс місце під пляшки та ігри

## Встановлення

Готових збірок (DMG, Homebrew) ще немає, тож cross-app збирається з коду.

1. Встановіть Rust, якщо його немає:

   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. Зберіть і встановіть `cross` з теки проєкту. Бінарник потрапить у `~/.cargo/bin`, який rustup додає в `PATH`:

   ```bash
   cargo install --locked --path crates/cross-cli
   ```

3. Перевірте систему:

   ```bash
   cross doctor
   ```

   Приклад виводу:

   ```text
   [ok  ] architecture              arm64 (Apple Silicon)
   [ok  ] chip                      Apple M1 Pro
   [ok  ] macOS                     26.5.2
   [ok  ] Rosetta 2                 installed
   [ok  ] Xcode Command Line Tools  /Library/Developer/CommandLineTools
   [warn] Game Porting Toolkit      not imported; DirectX 12 games need D3DMetal (see `cross engine import-gptk`)
   ```

4. Якщо Rosetta 2 не встановлена:

   ```bash
   cross engine install-rosetta
   ```

   Команда питає підтвердження і запускає `softwareupdate --install-rosetta --agree-to-license`. Може знадобитися пароль адміністратора.

Видалення: `cargo uninstall cross-cli`. Дані лежать окремо (див. [Де що зберігається](#де-що-зберігається)).

---

## Швидкий старт

### 1. Двигун Wine

```bash
cross engine list                              # каталог і що встановлено
cross engine install wine-devel-11.18 --dxvk   # ~255 МБ: Wine + DXVK + MoltenVK
```

Кожен архів перевіряється за SHA-256 і кешується, тож повторна установка йде без мережі. `--dxvk` можна додати й пізніше до вже встановленого двигуна.

### 2. Пляшка

Пляшка — це окремий Wine-префікс зі своїм «диском C:», реєстром і програмами.

```bash
cross bottle create demo                 # Windows 11 за замовчуванням; --windows 7|8|10|11
cross bottle list
```

### 3. Запуск програми

```bash
cross run demo ~/Downloads/setup.exe                 # .exe / .msi / .lnk / .bat
cross run demo ~/Games/App/app.exe -- --windowed     # аргументи програми після --
cross run demo 'C:\Program Files\App\app.exe'        # Windows-шлях усередині пляшки
cross run demo notepad                               # вбудовані програми Wine
```

Корисні прапорці:

```bash
cross run demo game.exe --metal-hud --msync --dxvk-hud fps \
  --env MY_VAR=1 --dll-override d3d11=n,b --workdir ~/Games/App
```

Код виходу програми стає кодом виходу `cross`.

### 4. Залежності (аналог winetricks)

```bash
cross deps list --bottle demo
cross deps install demo vcrun2022 dxvk
```

Verbs, яких немає в каталозі, передаються системному `winetricks`, якщо він встановлений (`brew install winetricks`). Перед цим CLI попереджає. Вимкнути таку поведінку можна прапорцем `--no-winetricks`.

### 5. Steam

```bash
cross bottle create steam
cross steam install steam        # закріплений SteamSetup.exe + патчі
cross steam run steam            # вікно Steam: увійдіть і встановіть гру
cross steam games steam          # список встановлених ігор
cross steam launch steam 220     # запуск гри за appid
```

Детальніше у [docs/steam.md](docs/steam.md).

---

## Довідка команд

Повний опис будь-якої команди дає `cross <команда> --help`.

### `cross doctor [--json]`

Перевіряє чип, версію macOS, Rosetta 2, Xcode CLT і Game Porting Toolkit. Якщо система не підходить, повертає ненульовий код.

### `cross engine`

| Команда | Що робить |
|---|---|
| `list [--installed] [--json]` | каталог двигунів і їхній стан, а також стан GPTK |
| `install <id> [--dxvk]` | завантажує, перевіряє й розпаковує двигун; `--dxvk` додає DXVK + MoltenVK |
| `remove <id>` | видаляє двигун (пляшки лишаються) |
| `import-gptk <dmg або тека> [--version V]` | імпортує D3DMetal з `Game_Porting_Toolkit_*.dmg` |
| `install-rosetta [--yes]` | встановлює Rosetta 2 |

**Game Porting Toolkit** не можна поширювати разом із cross-app. Завантажте його самі з [developer.apple.com](https://developer.apple.com/games/) (потрібен Apple ID) і передайте шлях:

```bash
cross engine import-gptk ~/Downloads/Game_Porting_Toolkit_3.0.dmg
```

Якщо образ показує ліцензію, змонтуйте його один раз у Finder, прийміть ліцензію і передайте шлях до змонтованого тому (`/Volumes/...`).

### `cross bottle`

| Команда | Що робить |
|---|---|
| `create <id> [--name N] [--engine E] [--windows 7\|8\|10\|11]` | створює 64-бітний префікс |
| `list [--json]` | список пляшок і їхній стан (running/stopped) |
| `delete <id> [--yes]` | видаляє пляшку з усім вмістом (питає підтвердження) |
| `clone <src> <dst> [--name N]` | копіює пляшку (на APFS це миттєво й без зайвого місця) |
| `rename <id> <new-id>` | змінює id пляшки |
| `ps <id> [--json]` | Windows-процеси в пляшці |
| `kill <id> [--force]` | просить програми завершитись; `--force` вбиває все |
| `reg <id> get <KEY> <NAME>` | читає значення реєстру |
| `reg <id> set <KEY> <NAME> <DATA> [--type sz\|dword\|...]` | записує значення реєстру |
| `reg <id> delete <KEY> <NAME>` | видаляє значення реєстру |
| `preset <id> windows <7\|8\|10\|11>` | змінює версію Windows |
| `preset <id> dpi <96..480>` | змінює масштаб: 96 = 100 %, 144 = 150 %, 192 = 200 % |
| `preset <id> virtual-desktop <WxH\|off>` | вмикає чи вимикає віртуальний робочий стіл, наприклад `1920x1080` |
| `tool <id> winecfg\|regedit\|cmd\|taskmgr` | відкриває інструмент Wine |

Приклади:

```bash
cross bottle reg demo set 'HKCU\Software\MyGame' Fullscreen 1 --type dword
cross bottle preset demo virtual-desktop 1920x1080
cross bottle kill demo --force
```

Пляшку, у якій щось запущено, не можна клонувати чи перейменувати. Спершу виконайте `cross bottle kill`.

### `cross run <bottle> <program> [опції] [-- аргументи]`

Запускає програму і чекає на її завершення. Тип визначається за розширенням:

- `.exe` запускається напряму;
- `.msi` йде через `msiexec /i`;
- `.lnk` йде через `start /unix`;
- `.bat` і `.cmd` йдуть через `cmd /c`.

Робоча тека за замовчуванням — тека самої програми.

### `cross deps`

| Команда | Що робить |
|---|---|
| `list [--bottle B] [--json]` | доступні verbs і що встановлено в пляшці |
| `install <bottle> <verb>... [--force] [--no-winetricks]` | ставить verbs разом з їхніми залежностями |

Зараз у каталозі є:

| Verb | Що ставить |
|---|---|
| `vcrun2022` | Visual C++ 2015–2022 runtime 14.36 (x86 + x64) |
| `dxvk` | DXVK 1.10.3 у `system32`/`syswow64` разом з overrides; потрібен двигун з `--dxvk` |

Установлені verbs записуються в `bottle.toml`, тож повторна установка пропускається без `--force`.

### `cross steam`

| Команда | Що робить |
|---|---|
| `install <bottle> [--force]` | ставить Steam (тиха установка, патчі) |
| `run <bottle> [-- аргументи]` | запускає клієнт Steam і чекає, поки його закриють |
| `games <bottle> [--all] [--json]` | встановлені ігри: appid, стан, розмір |
| `launch <bottle> <appid>` | запускає гру через Steam у фоні; лог пишеться в `logs/steam.log` |
| `covers <bottle> [--force]` | завантажує обкладинки ігор у кеш |
| `profile <bottle> <appid> [--backend dxvk\|d3dmetal] [--env K=V] [--arg A] [--clear]` | налаштування окремої гри |
| `patches [--update [--url U] [--force]] [--reset]` | показує або оновлює набір патчів Steam |

---

## Де що зберігається

Усе лежить у `~/Library/Application Support/cross-app/`:

```text
config.toml                   глобальні налаштування
engines/<id>/                 двигуни Wine (engine.toml, components/ з DXVK і MoltenVK)
gptk/                         імпортований Game Porting Toolkit (lib/external, VERSION)
bottles/<id>/
  bottle.toml                 налаштування пляшки, залежності, профілі ігор, стан Steam
  prefix/                     сам Wine-префікс (drive_c, реєстр)
  logs/                       логи команд для цієї пляшки, steam.log
cache/downloads/              перевірені завантаження (<sha256>-<файл>)
cache/covers/                 обкладинки ігор
logs/                         загальні логи
catalog.json, verbs.toml,     необов'язкові локальні каталоги, що перекривають
steam-patches.toml            вбудовані
```

Змінна `CROSS_APP_HOME=/інша/тека` переносить усе в іншу теку. Це зручно для тестів і окремих наборів пляшок.

`config.toml` (усі поля необов'язкові):

```toml
default_engine = "wine-devel-11.18"   # двигун для нових пляшок
gptk_dir = "/шлях/до/gptk"            # якщо GPTK лежить не в стандартному місці
log_level = "info"                    # або debug, trace; змінна RUST_LOG має пріоритет
steam_patches_url = "https://..."     # звідки `cross steam patches --update` бере оновлення
```

Wine за замовчуванням працює тихо (`WINEDEBUG=-all`). Щоб побачити його діагностику, задайте `WINEDEBUG` самостійно, наприклад `WINEDEBUG=+err cross run ...`.

---

## Обмеження

Повний статус кожного пункту ведеться в [SPRINT_PLAN.md](SPRINT_PLAN.md).

- **Не все перевірено на реальних даних:**
  - запуск `.msi` і `.lnk` на справжніх файлах;
  - установка `vcrun2022` справжнім інсталятором Microsoft;
  - справжній `SteamSetup.exe`, вхід у Steam і запуск гри;
  - рендеринг через DXVK.

  Усе це покрито лише тестами з підставним wine. Перевірено на реальному Wine 11.18: двигуни, створення пляшок, запуск програм, реєстр, процеси, clone/rename/delete і verb `dxvk`.
- **D3DMetal ще не підключено.** Імпорт GPTK працює, але `--backend d3dmetal` поки означає вбудований Direct3D Wine. Перемикач бекендів запланований на Спринт 7.
- **Rosetta 2.** Apple обіцяє її до macOS 27 включно, у macOS 28 лишиться лише підмножина. Двигуни без Rosetta запланові на Спринт 8, `cross doctor` про це попереджає.
- **Анти-чіти** (EAC, BattlEye, Vanguard) і онлайн-ігри з ними під Wine не працюють. Це свідомий not-a-goal.
- **Steam:** оверлей вимкнено патчем. winetricks додатково ставить для Steam `corefonts`, а в нашому каталозі його ще немає, тому Store і Library можуть показуватись без тексту. Деталі у [docs/steam.md](docs/steam.md).
- **Graceful-зупинка** (`cross bottle kill` без `--force`) просить завершитись лише програми з вікнами. Консольні процеси її ігнорують, для них потрібен `--force`.

---

## Розробка

```bash
cargo build                                   # збірка
cargo test --workspace                        # юніт-тести (без мережі й Wine)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Інтеграційні тести на справжньому Wine позначені `#[ignore]`. Для них потрібен встановлений двигун:

```bash
cross engine install wine-devel-11.18
CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app \
  cargo test -p cross-bottle --test real_wine -- --ignored --test-threads=1
```

Структура workspace:

| Крейт | Відповідальність |
|---|---|
| `cross-core` | доменна модель (пляшка, двигун, профіль), конфіги, логування, `doctor` |
| `cross-engine` | каталог двигунів, перевірені завантаження, імпорт GPTK, Rosetta |
| `cross-bottle` | пляшки, запуск програм, процеси, реєстр, пресети |
| `cross-deps` | каталог verbs і їх установка, fallback на winetricks |
| `cross-steam` | Steam: установка, патчі, бібліотека (VDF), запуск, обкладинки |
| `cross-cli` | бінарник `cross` |

Головний принцип: ядро не знає про інтерфейс. Усе, що робитиме GUI, вже доступне з CLI. Рішення щодо GUI зафіксовано в [docs/adr/0001-gui-framework.md](docs/adr/0001-gui-framework.md).

## Ліцензії

- **cross-app:** у `Cargo.toml` вказано MIT; окремого файлу `LICENSE` поки немає.
- **Wine:** ліцензія LGPL. Двигуни завантажуються з релізів Gcenx, cross-app їх не перепакує.
- **D3DMetal / Game Porting Toolkit:** належать Apple, cross-app їх не поширює. Користувач імпортує свою копію.
- **Steam і ігри:** належать своїм власникам. Вхід у Steam користувач виконує сам, cross-app не торкається паролів.
