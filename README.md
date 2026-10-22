<div align="center">

# cross-app

**Запускайте Windows-ігри та програми на Mac з Apple Silicon. Зі Steam, без віртуальних машин.**

![macOS 14+](https://img.shields.io/badge/macOS-14%2B-black?logo=apple)
![Apple Silicon](https://img.shields.io/badge/Apple%20Silicon-M1%E2%80%93M5-black)
![Rust](https://img.shields.io/badge/Rust-stable-orange?logo=rust)
![Status](https://img.shields.io/badge/status-v0.0.1%20%C2%B7%20early%20dev-yellow)
![License](https://img.shields.io/badge/license-MIT-blue)

[Встановлення](#встановлення) · [Перший запуск](#перший-запуск-за-10-хвилин) · [Рецепти](#рецепти) · [GUI](#gui) · [Довідка команд](#довідка-команд) · [Документація](#документація)

</div>

---

## Що це

cross-app — лаунчер, який бере на себе всю ручну роботу із запуску Windows-софту на Mac: сам завантажує й перевіряє Wine, створює ізольовані «пляшки», ставить потрібні бібліотеки, встановлює й патчить Steam, вмикає потрібний графічний бекенд.

Ви кажете *що* запустити, а cross-app знає *як*:

```bash
cross run demo ~/Downloads/setup.exe
```

Є дві форми: **CLI `cross`** для терміналу і **десктопний застосунок** (Tauri 2). Обидві працюють з тими самими даними.

> [!WARNING]
> **Рання розробка (v0.0.1), до публічної бети ще не дійшло.** Функціональність з [плану](SPRINT_PLAN.md) реалізована в коді, але багато що ще не перевірено на справжніх іграх, Steam і GPTK. Готових збірок (DMG, Homebrew) немає, cross-app збирається з коду. Див. [Обмеження](#обмеження).

### Як це працює

cross-app не емулює Windows сам. Він керує перевіреним стеком:

```text
  Windows-програма (.exe)
          │
          ▼
  Wine ─────────────────── перекладає виклики Windows API у macOS
          │
          ▼
  Rosetta 2 ────────────── виконує x86_64-код на ARM-процесорі
          │
          ▼
  DXVK + MoltenVK ──────── Direct3D 9/10/11 → Vulkan → Metal
  або D3DMetal (GPTK) ──── Direct3D 11/12 → Metal
          │
          ▼
        macOS
```

| Шар | Чим закрито |
|---|---|
| Windows API → macOS | [Wine](https://www.winehq.org/) (збірки [Gcenx](https://github.com/Gcenx/macOS_Wine_builds)) |
| x86_64 → ARM64 | Rosetta 2 |
| Direct3D 9/10/11 → Metal | DXVK + MoltenVK |
| Direct3D 11/12 → Metal | D3DMetal з Apple Game Porting Toolkit (GPTK імпортує користувач) |

### Три слова, які варто знати

| Термін | Що означає |
|---|---|
| **Двигун** (engine) | конкретна збірка Wine, наприклад `wine-devel-11.18`. Ставиться один раз, використовується всіма пляшками. |
| **Пляшка** (bottle) | окремий «маленький Windows»: свій диск `C:`, реєстр і встановлені програми. Пляшки не заважають одна одній: зламали одну — інші цілі. |
| **Verb** | залежність, яку можна доставити в пляшку однією командою, наприклад `vcrun2022` (Visual C++ runtime). |

---

## Встановлення

### Вимоги

- Mac з **Apple Silicon** (M1–M5). Intel не підтримується.
- **macOS 14+**, рекомендовано 15+.
- **Rosetta 2**: cross-app може поставити її сам.
- **Xcode Command Line Tools**: `xcode-select --install`.
- **Rust** (stable): потрібен лише для збирання з коду.
- Близько **1 ГБ** на двигун Wine, плюс місце під пляшки та ігри.

### Кроки

**1. Встановіть Rust**, якщо його немає:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

**2. Зберіть і встановіть `cross`** з теки проєкту. Бінарник потрапить у `~/.cargo/bin`, який rustup уже додав у `PATH`:

```bash
cargo install --locked --path crates/cross-cli
```

**3. Перевірте, чи готовий Mac:**

```bash
cross doctor
```

```text
[ok  ] architecture              arm64 (Apple Silicon)
[ok  ] chip                      Apple M1 Pro
[ok  ] macOS                     26.5.2
[ok  ] Rosetta 2                 installed
[ok  ] Xcode Command Line Tools  /Library/Developer/CommandLineTools
[warn] Game Porting Toolkit      not imported; DirectX 12 games need D3DMetal (see `cross engine import-gptk`)
```

- `[ok]` — усе гаразд.
- `[warn]` — працювати можна, але щось не налаштовано (GPTK потрібен лише для DirectX 12).
- `[FAIL]` — на цьому Mac cross-app не запрацює.

**4. Якщо бракує Rosetta 2:**

```bash
cross engine install-rosetta
```

Команда спитає підтвердження й запустить `softwareupdate --install-rosetta --agree-to-license`. Може знадобитися пароль адміністратора.

<details>
<summary>Як видалити cross-app</summary>

```bash
cargo uninstall cross-cli
rm -rf ~/Library/Application\ Support/cross-app   # двигуни, пляшки, кеш, логи
```

Друга команда стирає **всі** пляшки разом з іграми та збереженнями в них. Перед нею переконайтесь, що нічого звідти не потрібно.

</details>

---

## Перший запуск за 10 хвилин

Від чистого Mac до запущеної Windows-програми. Більшість часу забирає завантаження (~255 МБ). Докладна версія з перевірками на кожному кроці: [docs/quickstart.md](docs/quickstart.md).

**Крок 1. Поставте двигун Wine.**

```bash
cross engine list                              # що є в каталозі
cross engine install wine-devel-11.18 --dxvk   # Wine + DXVK + MoltenVK
```

Кожен архів перевіряється за SHA-256 і кешується, тож повторна установка мережі не потребує. `--dxvk` можна додати й пізніше.

**Крок 2. Створіть пляшку.**

```bash
cross bottle create demo      # Windows 11 за замовчуванням; інша: --windows 10
```

**Крок 3. Запустіть програму.**

```bash
cross run demo notepad                       # вбудований блокнот Wine: перевірка, що все працює
cross run demo ~/Downloads/setup.exe         # інсталятор, який ви завантажили
```

Після установки програма лежить усередині пляшки. Запускайте її за Windows-шляхом:

```bash
cross run demo 'C:\Program Files\App\app.exe'
```

Готово. Далі — рецепти для конкретних задач.

---

## Рецепти

<details open>
<summary><b>Запустити програму з аргументами чи з іншої теки</b></summary>

```bash
cross run demo ~/Games/App/app.exe -- --windowed      # усе після -- отримує сама програма
cross run demo game.exe --workdir ~/Games/App         # робоча тека (за замовчуванням — тека програми)
cross run demo setup.msi                              # .msi, .lnk, .bat і .cmd теж підходять
```

`cross` чекає, поки програма завершиться, і повертає її код виходу.

</details>

<details>
<summary><b>Програма пише, що бракує DLL чи Visual C++</b></summary>

```bash
cross deps list --bottle demo           # що можна поставити і що вже стоїть
cross deps install demo vcrun2022       # Visual C++ 2015–2022
```

Якщо verb немає в каталозі, `cross` запропонує системний `winetricks` (`brew install winetricks`).

</details>

<details>
<summary><b>Грати в ігри зі Steam</b></summary>

```bash
cross bottle create steam
cross steam install steam          # ставить Steam і накладає патчі
cross steam run steam              # відкриється вікно Steam: увійдіть і встановіть гру
cross steam games steam            # список встановлених ігор з їхніми appid
cross steam launch steam 413150    # запуск гри за appid (тут Stardew Valley)
```

Пароль і 2FA ви вводите у вікні Steam самі, cross-app їх не бачить. Докладно: [docs/steam.md](docs/steam.md).

</details>

<details>
<summary><b>Гра на DirectX 12</b></summary>

DirectX 12 потребує D3DMetal з Apple Game Porting Toolkit. Apple не дозволяє його поширювати, тому його імпортують з власної копії:

1. Завантажте `Game_Porting_Toolkit_*.dmg` з [developer.apple.com/games](https://developer.apple.com/games/) (потрібен безкоштовний Apple ID).
2. Імпортуйте:

   ```bash
   cross engine import-gptk ~/Downloads/Game_Porting_Toolkit_3.0.dmg
   cross engine d3dmetal enable wine-devel-11.18
   ```

3. Перемкніть пляшку (або одну Steam-гру) на D3DMetal:

   ```bash
   cross bottle graphics demo --backend d3dmetal
   cross bottle graphics steam --appid 413150 --backend d3dmetal
   ```

Якщо образ показує ліцензію, змонтуйте його один раз у Finder, прийміть її і передайте шлях до тому (`/Volumes/...`).

</details>

<details>
<summary><b>Гра гальмує або погано виглядає</b></summary>

```bash
cross bottle graphics demo                           # поточні налаштування
cross bottle graphics demo --backend dxvk            # спробувати інший бекенд
cross bottle graphics demo --metal-hud on            # показати FPS (Metal HUD)
cross bottle graphics demo --sync msync              # швидша синхронізація, якщо двигун її підтримує
cross run demo game.exe --dxvk-hud fps               # HUD DXVK лише для цього запуску
```

Віртуальний робочий стіл і масштаб для Retina:

```bash
cross bottle preset demo virtual-desktop 1920x1080
cross bottle preset demo dpi 192                     # 200 %
```

Докладно про бекенди, HUD і геймпади: [docs/graphics.md](docs/graphics.md).

</details>

<details>
<summary><b>Узяти готові налаштування для гри (профіль сумісності)</b></summary>

```bash
cross compat match --appid 413150              # чи є профіль для цієї гри
cross compat apply steam 413150                # записати його налаштування в пляшку
cross compat match --exe ~/Games/App/app.exe   # те саме для гри не зі Steam
```

Профіль містить рейтинг, бекенд, змінні оточення, залежності й відомі баги. Усі вбудовані профілі поки позначені як неперевірені. Як написати власний: [docs/compat-profiles.md](docs/compat-profiles.md).

</details>

<details>
<summary><b>Кілька ігор, які не мають заважати одна одній</b></summary>

Робіть окрему пляшку під кожну велику гру чи лаунчер. Щоб не ставити залежності щоразу, налаштуйте одну і клонуйте її:

```bash
cross bottle create base
cross deps install base vcrun2022
cross bottle clone base witcher      # на APFS миттєво й без зайвого місця
```

</details>

<details>
<summary><b>Щось зависло, впало чи не запускається</b></summary>

| Ситуація | Що зробити |
|---|---|
| Програма не закривається | `cross bottle kill demo`, а якщо не допомогло — `cross bottle kill demo --force` |
| Що запущено в пляшці | `cross bottle ps demo` |
| Подивитись, що писав Wine | `cross logs --bottle demo --wine` |
| Стежити за логом наживо | `cross logs --bottle demo --wine -f` |
| Детальніший вивід Wine | `cross run demo app.exe --wine-debug +err,+module` |
| Зібрати все для звіту про помилку | `cross diag -o report.tar.gz` |
| `cross` сам аварійно завершився | `cross crash show` (звіт локальний, нікуди не відправляється) |
| Гра з анти-чітом (EAC, BattlEye) | не запрацює, див. [FAQ](docs/faq.md) |

Повідомити про помилку чи результат гри: шаблони issue «Помилка» і «Звіт про сумісність гри» у [.github/ISSUE_TEMPLATE/](.github/ISSUE_TEMPLATE/). Додайте вивід `cross doctor --json`.

</details>

---

## GUI

Десктопний застосунок робить те саме, що й CLI, з тими самими даними:

- бібліотека ігор з обкладинками й значками сумісності;
- сторінка пляшки з налаштуваннями та процесами;
- двигуни й імпорт GPTK;
- живі логи;
- майстер першого запуску;
- drag-and-drop `.exe` у вікно.

```bash
cargo run -p cross-gui        # зібрати й відкрити вікно (Node/npm не потрібні)
```

Перша збірка довга (Tauri — це ~400 крейтів). Готового `.app` поки немає. Що в GUI перевірено, а що ні: [docs/gui.md](docs/gui.md).

---

## Довідка команд

Повний опис будь-якої команди: `cross <команда> --help`.

Глобальні прапорці: `-v` / `-vv` / `-vvv` — більше подробиць у терміналі, `-q` — лише помилки. Лог-файли завжди детальні.

<details>
<summary><code>cross doctor</code> — перевірка системи</summary>

`cross doctor [--json]` перевіряє чип, версію macOS, Rosetta 2, Xcode CLT і Game Porting Toolkit. Якщо система не підходить, повертає ненульовий код.

</details>

<details>
<summary><code>cross engine</code> — двигуни Wine, GPTK, Rosetta</summary>

| Команда | Що робить |
|---|---|
| `list [--installed] [--json]` | каталог двигунів і їхній стан, а також стан GPTK |
| `install <id> [--dxvk]` | завантажує, перевіряє й розпаковує двигун; `--dxvk` додає DXVK + MoltenVK |
| `remove <id>` | видаляє двигун (пляшки лишаються) |
| `import-gptk <dmg або тека> [--version V]` | імпортує D3DMetal з `Game_Porting_Toolkit_*.dmg` |
| `d3dmetal enable\|disable\|status <id>` | підключає імпортований D3DMetal до двигуна (APFS-копія двигуна) або прибирає його |
| `install-rosetta [--yes]` | встановлює Rosetta 2 |

</details>

<details>
<summary><code>cross bottle</code> — пляшки</summary>

| Команда | Що робить |
|---|---|
| `create <id> [--name N] [--engine E] [--windows 7\|8\|10\|11]` | створює 64-бітний префікс |
| `list [--json]` | список пляшок і їхній стан (running/stopped) |
| `delete <id> [--yes]` | видаляє пляшку з усім вмістом (питає підтвердження) |
| `clone <src> <dst> [--name N]` | копіює пляшку (на APFS миттєво й без зайвого місця) |
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
| `graphics <id> [--appid A] [--backend dxvk\|d3dmetal] [--metal-hud on\|off] [--metal-hud-log on\|off] [--dxvk-hud SPEC] [--sync esync\|msync\|none] [--cpu-count N]` | графічні налаштування пляшки чи окремої Steam-гри; без опцій показує поточні |
| `fps <лог-файл>` | зведення FPS із рядків Metal HUD (після `graphics --metal-hud-log on`) |

```bash
cross bottle reg demo set 'HKCU\Software\MyGame' Fullscreen 1 --type dword
```

Пляшку, у якій щось запущено, не можна клонувати чи перейменувати. Спершу виконайте `cross bottle kill`.

</details>

<details>
<summary><code>cross run</code> — запуск програми</summary>

`cross run <bottle> <program> [опції] [-- аргументи]` запускає програму і чекає на її завершення. Тип визначається за розширенням:

- `.exe` запускається напряму;
- `.msi` — через `msiexec /i`;
- `.lnk` — через `start /unix`;
- `.bat` і `.cmd` — через `cmd /c`.

`<program>` може бути шляхом на Mac, Windows-шляхом усередині пляшки (`'C:\...'`) або назвою вбудованої програми Wine (`notepad`).

| Опція | Що робить |
|---|---|
| `--workdir <тека>` | робоча тека (за замовчуванням — тека програми) |
| `--env K=V` | змінна оточення для цього запуску |
| `--dll-override d3d11=n,b` | DLL override для цього запуску |
| `--metal-hud`, `--dxvk-hud fps`, `--msync` | графічні перемикачі для цього запуску |
| `--wine-debug +err,+module` | канали `WINEDEBUG` лише для цього запуску |
| `--no-log` | залишити stderr Wine у терміналі замість `bottles/<id>/logs/run-*.log` |

Графічні налаштування пляшки застосовуються автоматично. Якщо програма завершилась з помилкою, `cross` друкує шлях до run-логу.

</details>

<details>
<summary><code>cross deps</code> — залежності (аналог winetricks)</summary>

| Команда | Що робить |
|---|---|
| `list [--bottle B] [--json]` | доступні verbs і що встановлено в пляшці |
| `install <bottle> <verb>... [--force] [--no-winetricks]` | ставить verbs разом з їхніми залежностями |

| Verb | Що ставить |
|---|---|
| `vcrun2022` | Visual C++ 2015–2022 runtime 14.36 (x86 + x64) |
| `dxvk` | DXVK 1.10.3 у `system32`/`syswow64` разом з overrides; потрібен двигун з `--dxvk` |

Установлені verbs записуються в `bottle.toml`, тож повторна установка пропускається без `--force`. Verbs поза каталогом передаються системному `winetricks` (з попередженням); `--no-winetricks` це вимикає.

</details>

<details>
<summary><code>cross steam</code> — Steam</summary>

| Команда | Що робить |
|---|---|
| `install <bottle> [--force]` | ставить Steam (тиха установка, патчі) |
| `run <bottle> [-- аргументи]` | запускає клієнт Steam і чекає, поки його закриють |
| `games <bottle> [--all] [--json]` | встановлені ігри: appid, стан, розмір |
| `launch <bottle> <appid>` | запускає гру через Steam у фоні; лог пишеться в `logs/steam.log` |
| `covers <bottle> [--force]` | завантажує обкладинки ігор у кеш |
| `profile <bottle> <appid> [--backend dxvk\|d3dmetal] [--env K=V] [--arg A] [--clear]` | налаштування окремої гри |
| `patches [--update [--url U] [--force]] [--reset]` | показує або оновлює набір патчів Steam |

</details>

<details>
<summary><code>cross compat</code> — профілі сумісності</summary>

| Команда | Що робить |
|---|---|
| `list` | усі профілі (вбудовані, community, локальні) |
| `show <id\|appid>` | один профіль |
| `match [--appid A] [--exe шлях]` | знайти профіль для гри чи програми |
| `apply <bottle> <id\|appid> [--exe шлях]` | записати налаштування профілю в пляшку |
| `sync`, `import`, `bundle` | community-набір через https, локальний профіль, збірка набору |

Формат і шари: [docs/compat.md](docs/compat.md).

</details>

<details>
<summary><code>cross crossover</code> — CrossOver як зовнішній двигун</summary>

Якщо CrossOver встановлений, cross-app може запускати програми через його Wine, нічого не копіюючи.

| Команда | Що робить |
|---|---|
| `status` | чи знайдено CrossOver і чи зареєстровано його як двигун (`cross engine install crossover`) |
| `bottles` | пляшки CrossOver |
| `import <пляшка> [--id ID]` | підключити пляшку CrossOver до cross-app (як посилання) |

На справжньому CrossOver це ще не перевірялось.

</details>

<details>
<summary><code>cross logs</code>, <code>cross diag</code>, <code>cross crash</code> — діагностика</summary>

| Команда | Що робить |
|---|---|
| `logs [--bottle B] [--wine\|--steam] [-n N] [-f] [--list] [--path]` | показує лог: загальний, пляшки, вивід Wine чи Steam |
| `diag [-o файл.tar.gz]` | архів для звіту про помилку: `doctor`, налаштування, записи двигунів і пляшок, останні логи |
| `crash list [--json]` | збережені звіти про аварії `cross`, найновіші першими |
| `crash show [id\|latest]` | друкує звіт (за замовчуванням найновіший) |
| `crash clear [--yes]` | видаляє всі звіти |

Значення змінних, схожих на секрети, у логах замасковані. Звіти про аварії **нікуди не відправляються**, зберігаються останні 20; вимкнути: `CROSS_NO_CRASH_REPORTS=1`. Деталі: [docs/logging.md](docs/logging.md).

</details>

<details>
<summary><code>cross telemetry</code> — телеметрія (вимкнена за замовчуванням)</summary>

Знеособлена статистика запусків. Записує лише версію cross-app, macOS major.minor, чип, двигун, бекенд і результат запуску. **Нічого не надсилається**: ендпоінта немає.

| Команда | Що робить |
|---|---|
| `status` | стан і де лежать файли |
| `enable` / `disable` | увімкнути / вимкнути (видаляє чергу) |
| `show` | показати записані події |

</details>

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
  logs/                       логи команд, run-*.log (вивід Wine), steam.log
cache/downloads/              перевірені завантаження (<sha256>-<файл>)
cache/covers/                 обкладинки ігор
logs/                         загальні логи
crashes/                      локальні звіти про аварії (cross crash)
compat/                       community- і локальні профілі сумісності
telemetry.toml, telemetry/    згода на телеметрію і локальна черга (лише якщо увімкнено)
catalog.json, verbs.toml,     необов'язкові локальні каталоги, що перекривають
steam-patches.toml            вбудовані
```

Файли програм у пляшці — у `bottles/<id>/prefix/drive_c/`, їх можна відкрити у Finder.

`CROSS_APP_HOME=/інша/тека` переносить усе в іншу теку: зручно для тестів чи окремих наборів пляшок.

<details>
<summary><code>config.toml</code> (усі поля необов'язкові)</summary>

```toml
default_engine = "wine-devel-11.18"   # двигун для нових пляшок
gptk_dir = "/шлях/до/gptk"            # якщо GPTK лежить не в стандартному місці
log_level = "debug"                   # деталізація лог-файлів; RUST_LOG має пріоритет
steam_patches_url = "https://..."     # звідки `cross steam patches --update` бере оновлення
```

Wine за замовчуванням працює тихо (`WINEDEBUG=-all`). Щоб побачити його діагностику, задайте `WINEDEBUG` самі, наприклад `WINEDEBUG=+err cross run ...`.

</details>

---

## Обмеження

Повний статус кожного пункту: [SPRINT_PLAN.md](SPRINT_PLAN.md).

- **Не все перевірено на реальних даних.** Лише тестами з підставними даними покриті: `.msi` і `.lnk` на справжніх файлах, справжній інсталятор `vcrun2022`, справжній `SteamSetup.exe` із входом і запуском гри, рендеринг через DXVK чи D3DMetal, справжні ігри, підпис і нотаризація, CrossOver.
  На реальному Wine 11.18 перевірено: двигуни, пляшки, запуск програм, реєстр, процеси, clone/rename/delete, verb `dxvk` і перемикач бекендів. Вікно GUI відкривається.
- **D3DMetal перевірено лише на синтетичному GPTK.** Без імпортованого GPTK бекенд `d3dmetal` переходить на вбудований Direct3D Wine (wined3d), і `cross` про це попереджає. Поточні двигуни Gcenx не позначені як сумісні з D3DMetal.
- **Rosetta 2.** Apple обіцяє її до macOS 27 включно, у macOS 28 лишиться лише підмножина. Нативних arm64-двигунів у каталозі поки немає, запасний варіант — CrossOver. Стратегія: [ADR 0003](docs/adr/0003-post-rosetta.md).
- **Анти-чіти** (EAC, BattlEye, Vanguard) під Wine не працюють. Це свідомий not-a-goal.
- **Steam:** оверлей вимкнено патчем. `corefonts` у каталозі ще немає, тому Store і Library можуть показуватись без тексту. Деталі: [docs/steam.md](docs/steam.md).
- **`cross bottle kill` без `--force`** зупиняє лише програми з вікнами; консольні процеси потребують `--force`.

---

## Документація

| Документ | Про що |
|---|---|
| [Швидкий старт](docs/quickstart.md) | покроково від чистого Mac до запущеної програми чи гри |
| [FAQ](docs/faq.md) | анти-чіти, 32-бітні ігри, Rosetta, ліцензія GPTK, звіти про аварії |
| [Steam](docs/steam.md) | як влаштовано Steam у пляшці, патчі, відомі обмеження |
| [Графіка](docs/graphics.md) | D3DMetal і DXVK, HUD, sync, Retina, геймпади |
| [Профілі ігор (CLI)](docs/compat.md) | `cross compat`: формат, шари, синхронізація |
| [Профілі сумісності](docs/compat-profiles.md) | як описати налаштування гри, щоб вона запускалась «з коробки» |
| [Логи та діагностика](docs/logging.md) | де логи, `-v`, вивід Wine, `cross logs`, `cross diag` |
| [GUI](docs/gui.md) | застосунок на Tauri 2: збірка, запуск, що перевірено |
| [Пакетування](docs/packaging.md) | DMG, підпис, нотаризація, Homebrew, release-workflow |
| [Телеметрія](docs/telemetry.md) | що записується (лише за згодою) і що ні |
| [Тестова матриця](docs/test-matrix.md) | шаблон прогону й формат запису результатів |
| [Чекліст релізу](docs/release-checklist.md) | що має бути готово до v0.1.0-beta |
| [ADR](docs/adr/) | архітектурні рішення: GUI-фреймворк, фронтенд, стратегія після Rosetta |
| [Сторонні компоненти](THIRD_PARTY_NOTICES.md) | ліцензії всього, що cross-app використовує чи завантажує |

---

## Розробка

```bash
cargo build                                   # збірка (без GUI)
cargo test                                    # юніт-тести без GUI (без мережі й Wine)
cargo test --workspace                        # разом із GUI (Tauri, ~400 крейтів)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

<details>
<summary>Інтеграційні тести на справжньому Wine</summary>

Позначені `#[ignore]` і потребують встановленого двигуна:

```bash
cross engine install wine-devel-11.18
export CROSS_TEST_ENGINES_HOME=~/Library/Application\ Support/cross-app
cargo test -p cross-bottle --test real_wine -- --ignored --test-threads=1
cargo test -p cross-deps --test real_graphics -- --ignored
cargo test -p cross-engine --test real_engine -- --ignored
cargo test -p cross-gui --test real_wine -- --ignored
```

</details>

### Структура workspace

| Крейт | Відповідальність |
|---|---|
| `cross-core` | доменна модель (пляшка, двигун, профіль), конфіги, логування, `doctor` |
| `cross-engine` | каталог двигунів, перевірені завантаження, імпорт GPTK і D3DMetal, політика перекладу, CrossOver |
| `cross-bottle` | пляшки, запуск програм, процеси, реєстр, пресети, графіка, run-логи |
| `cross-deps` | каталог verbs і їх установка, fallback на winetricks |
| `cross-steam` | Steam: установка, патчі, бібліотека (VDF), запуск, обкладинки |
| `cross-compat` | профілі сумісності ігор: формат, шари, синхронізація, API для GUI |
| `cross-telemetry` | opt-in телеметрія: згода, локальна черга |
| `cross-gui` | десктопний застосунок на Tauri 2 (поза `default-members`) |
| `cross-cli` | бінарник `cross` |

Головний принцип: ядро не знає про інтерфейс, тож усе, що робить GUI, доступне і з CLI. Рішення зафіксовано в [docs/adr/](docs/adr/).

---

## Ліцензії

Повний перелік із джерелами: [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

- **cross-app:** MIT (вказано в `Cargo.toml`; окремого файлу `LICENSE` поки немає).
- **DXVK** (zlib) і **MoltenVK** (Apache-2.0) завантажуються з їхніх офіційних релізів разом із двигуном (`--dxvk`).
- **Wine** — LGPL. Двигуни завантажуються з релізів Gcenx, cross-app їх не перепаковує.
- **D3DMetal / Game Porting Toolkit** належать Apple; cross-app їх не поширює, користувач імпортує свою копію.
- **Steam і ігри** належать своїм власникам. Вхід у Steam користувач виконує сам, cross-app не торкається паролів.
