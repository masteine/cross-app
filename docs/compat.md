# Профілі сумісності (`cross-compat`)

Профіль описує одну гру: як її впізнати, з якими налаштуваннями запускати, що в ній відомо зламане і якого статусу очікувати (зелений, жовтий, червоний).

> **Усі вбудовані профілі не перевірені.** Їх написано з публічних джерел, указаних у кожному файлі. Жодну гру з cross-app не запускали. Бейдж завжди показує `(unverified)`, доки в профілі немає секції `[verified]` з датою, двигуном і машиною реального запуску.

## Команди

```bash
cross compat list                         # усі профілі з бейджами і походженням
cross compat show hollow-knight           # або за appid: cross compat show 367520
cross compat match --appid 1086940        # або --exe ~/Games/Game.exe (SHA-256, потім ім'я)
cross compat apply games baldurs-gate-3   # записати налаштування у гру в bottle.toml
cross compat apply games my-game --exe 'C:\Games\Game.exe'   # гра не зі Steam
cross compat import my-game.toml          # локальний профіль/перевизначення
cross compat sync --url https://…/bundle.toml               # community-набір
cross compat bundle path/to/profiles --revision 4 > bundle.toml
```

`apply` записує бекенд, змінні середовища, DLL overrides, аргументи й режим синхронізації в запис гри (`[[apps]]` з `steam_appid` або з `path`). Потім друкує команду для встановлення залежностей, яких бракує (`cross deps install …`), і застереження: неперевірений профіль, анти-чіт, відомі баги. Підсумок можна подивитися командою `cross bottle graphics <пляшка> --appid <appid>`.

## Формат профілю

Один TOML-файл на гру, ім'я файлу збігається з `id`:

```toml
schema = 1
id = "baldurs-gate-3"                 # [a-z0-9-], до 64 символів
name = "Baldur's Gate 3"
steam_appid = 1086940                 # за ним впізнається гра зі Steam
exe_names = ["bg3_dx11.exe", "bg3.exe"]   # слабке впізнавання за ім'ям
exe_sha256 = []                       # точне впізнавання конкретної збірки
rating = "yellow"                     # green | yellow | red
backend = "d3dmetal"                  # d3dmetal | dxvk (необов'язково)
sync = "esync"                        # none | esync | msync (необов'язково)
deps = ["vcrun2022"]                  # verbs для cross deps install
env = {}
dll_overrides = {}
args = []                             # лише прості -прапорці
anticheat = "EasyAntiCheat"           # необов'язково
known_bugs = ["…"]
notes = ["…"]

[source]                              # обов'язково: звідки налаштування
name = "…"
url = "https://…"
reported_rating = "playable"          # як оцінює джерело
reported_on = "Whisky 3.0.0, Wine 10.0, macOS 15.3"

# [verified]                          # лише після реального запуску з cross-app
# date = "2026-11-01"
# engine = "wine-devel-11.18"
# macos = "26.1"
# chip = "M3"
```

Значення рейтингу: **green** — запускається без ручних дій і грається; **yellow** — потрібні обхідні шляхи або гру ніхто не підтвердив; **red** — не запускається (анти-чіт, DRM, падіння).

Обмеження для файлів зі сторони (community, local), як і для `steam-patches.toml`: заборонені змінні, що перенаправляють префікс, завантажувач чи шляхи (`WINEPREFIX`, `WINEARCH`, `WINE`, `WINESERVER`, `WINELOADER`, `WINEDLLPATH`, `WINEDEBUG`, `PATH`, `HOME`, `TMPDIR`, `DYLD_*`, `LD_*`). Імена змінних мають бути у верхньому регістрі, режими DLL override — зі списку `n`, `b`, `n,b`, `b,n`, `native`, `builtin`, `native,builtin`, `builtin,native`, `d`, `disabled`, `""`. Аргументи можуть бути лише простими `-прапорцями`, URL джерела — лише https.

## Звідки беруться профілі

Порядок пріоритету (за `id`, наступний шар повністю заміняє профіль):

1. вбудовані (`crates/cross-compat/profiles/*.toml`, вшиті в бінарник);
2. community: `<дані>/compat/community/bundle.toml`, завантажений `cross compat sync`. Приймається лише більша `revision` (або `--force`);
3. локальні: `<дані>/compat/local/<id>.toml` (`cross compat import` або вручну).

Пошкоджений community- чи локальний файл не ламає завантаження: його пропускають, а CLI показує `warning: skipped …`.

### Структура community-репозиторію

```
cross-compat-profiles/
├── README.md
├── profiles/<id>.toml     # джерело правди, один файл на гру, зміни через PR з рев'ю
└── bundle.toml            # згенерований: cross compat bundle profiles --revision N
```

`bundle.toml` містить `schema`, `revision` і всі профілі як `[[profile]]`. Генератор перевіряє кожен файл, відповідність імені файлу до `id` і унікальність `id`. Публічного репозиторію й URL ще немає (як і для каналу патчів Steam).

## API для GUI (Спринт 6)

```rust
use cross_compat::{CompatDb, Rating};

let db = CompatDb::load(&dirs)?;                 // вбудовані + community + local
if let Some(entry) = db.by_appid(appid) {        // або db.by_exe(path)? -> Match { entry, kind }
    let p = &entry.profile;
    let colour = match p.rating { Rating::Green => …, Rating::Yellow => …, Rating::Red => … };
    p.badge_text();       // "yellow (unverified)"
    p.is_verified();      // false для всіх вбудованих
    p.known_bugs; p.anticheat; p.source.url; entry.layer; // Builtin | Community | Local
}
db.problems;              // файли, які пропущено, з причиною

// Застосувати: змінює Bottle у пам'яті, зберігає викликач.
let applied = cross_compat::apply_to_bottle(&mut bottle, p, None)?;
applied.missing_deps;     // встановити через cross-deps
```

`Entry`, `GameProfile` і `Rating` серіалізуються в JSON (`serde`), тож їх можна напряму віддавати у фронтенд Tauri.

## 10 вбудованих профілів

Джерело всіх: спільнотна база ігор [frankea/Whisky `GameDB.json`](https://github.com/frankea/Whisky/blob/main/WhiskyKit/Sources/WhiskyKit/GameDatabase/Resources/GameDB.json). Із 10 вибраних записів вісім позначено як «community-reference» без посилання на звіт, два (Skyrim SE, AoE II DE) посилаються на issues Whisky. Appid перевірено через Steam Store API (15.10.2026).

| Профіль | Appid | Рейтинг | Бекенд | Примітка |
|---|---|---|---|---|
| Hollow Knight | 367520 | green | d3dmetal | |
| Stardew Valley | 413150 | green | d3dmetal | SMAPI може не стартувати з першого разу |
| Terraria | 105600 | green | d3dmetal | |
| The Witcher 3 | 292030 | yellow | d3dmetal | next-gen = DX12, лише D3DMetal |
| Skyrim SE | 489830 | green | d3dmetal | ENB-пресети ламаються |
| Cyberpunk 2077 | 1091500 | yellow | d3dmetal | DX12, лише D3DMetal |
| Elden Ring | 1245620 | yellow | d3dmetal | лише офлайн (EAC) |
| Baldur's Gate 3 | 1086940 | yellow | d3dmetal | DX11-рендерер `bg3_dx11.exe` |
| Age of Empires II: DE | 813780 | yellow | dxvk | замість 32-бітної AoE II HD з тестової матриці; `dotnet48`, `xact` лише через winetricks |
| PUBG: Battlegrounds | 578080 | red | — | BattlEye |

Рейтинги взято з джерела й перекладено в нашу шкалу (`works` → green, `playable` або `unverified` → yellow, `broken` → red). На поточному двигуні каталогу (upstream Wine) D3DMetal не працює, тому `d3dmetal`-профілі там фактично йдуть через wined3d, а `sync` пропускається (див. [graphics.md](graphics.md)). Ці рейтинги — очікування з чужих машин, а не результат на нашому стеку. Мета Спринту 7 «10 ігор запустились з коробки» **не досягнута**, бо ігри не запускались.
