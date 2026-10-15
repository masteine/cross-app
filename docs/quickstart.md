# Швидкий старт

Покроковий шлях від чистого Mac до запущеної Windows-програми чи гри зі Steam. Для бети потрібен термінал: GUI (Спринт 6) ще не випущено.

Займе приблизно 15–20 хвилин, більшу частину з них забирають завантаження (Wine приблизно 255 МБ, Steam і гра окремо).

## 0. Що потрібно

- Mac з Apple Silicon (M1–M5). Intel-Mac не підтримується.
- macOS 14 або новіша, рекомендовано 15+.
- Близько 1 ГБ на двигун Wine, плюс місце під пляшки й ігри.
- Xcode Command Line Tools: `xcode-select --install`.
- Rust (stable), доки немає готових збірок: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`.

## 1. Встановити cross

```bash
git clone <адреса репозиторію> cross-app && cd cross-app
cargo install --locked --path crates/cross-cli
cross --version
```

Бінарник опиниться в `~/.cargo/bin/cross`.

## 2. Перевірити систему

```bash
cross doctor
```

Рядки `[FAIL]` означають, що на цьому Mac cross-app працювати не буде. `[warn]` — попередження: наприклад, GPTK не імпортовано (потрібен лише для DirectX 12, див. крок 6).

Якщо Rosetta 2 не встановлена:

```bash
cross engine install-rosetta     # питає підтвердження, може попросити пароль адміністратора
```

## 3. Поставити двигун Wine

```bash
cross engine list
cross engine install wine-devel-11.18 --dxvk
```

`--dxvk` додає DXVK і MoltenVK (Direct3D 9/10/11 через Vulkan → Metal). Кожен архів перевіряється за SHA-256 і зберігається в кеші, тому повторна установка не потребує мережі.

## 4. Створити пляшку

Пляшка — окремий Wine-префікс зі своїм «диском C:», реєстром і програмами. Під кожну велику гру або лаунчер зручно мати окрему.

```bash
cross bottle create demo          # Windows 11; інша версія: --windows 10
cross bottle list
```

## 5. Запустити Windows-програму

```bash
cross run demo ~/Downloads/npp.8.6.Installer.x64.exe      # інсталятор
cross run demo 'C:\Program Files\Notepad++\notepad++.exe'  # встановлена програма
cross run demo notepad                                      # вбудований блокнот Wine
```

Програмі передається шлях до файлу (`.exe`, `.msi`, `.lnk`, `.bat`), а не до теки. Аргументи програми пишуться після `--`. Код виходу програми стає кодом виходу `cross`.

Якщо програмі бракує runtime-бібліотек:

```bash
cross deps install demo vcrun2022     # Visual C++ 2015–2022
cross deps install demo dxvk          # DXVK у пляшку (потрібен двигун з --dxvk)
```

## 6. DirectX 12 і Game Porting Toolkit (необов'язково)

D3DMetal від Apple не можна поширювати разом із cross-app, тому його імпортують з власної копії:

1. Завантажте `Game_Porting_Toolkit_*.dmg` з [developer.apple.com/games](https://developer.apple.com/games/) (потрібен безкоштовний Apple ID).
2. Імпортуйте його: `cross engine import-gptk ~/Downloads/Game_Porting_Toolkit_3.0.dmg`.

> **Обмеження бети:** імпорт працює, але `--backend d3dmetal` поки означає вбудований Direct3D Wine. Перемикач бекендів з'явиться в Спринті 7.

## 7. Steam

```bash
cross bottle create steam
cross steam install steam          # закріплений SteamSetup.exe + патчі
cross steam run steam              # відкриється вікно Steam: увійдіть і встановіть гру
cross steam games steam            # список ігор з appid
cross steam launch steam 413150    # запуск гри за appid (тут Stardew Valley)
```

Вхід у Steam (пароль, 2FA) виконуєте ви у вікні Steam; cross-app паролів не бачить. Налаштування окремої гри (бекенд, змінні, аргументи) описано в [steam.md](steam.md) і [compat-profiles.md](compat-profiles.md).

## 8. Якщо щось пішло не так

| Симптом | Що зробити |
|---|---|
| Програма «зависла» або не закривається | `cross bottle kill demo`, а якщо не допомогло, то `cross bottle kill demo --force` |
| Треба бачити помилки Wine | `WINEDEBUG=+err cross run demo app.exe` |
| Де логи | `~/Library/Application Support/cross-app/bottles/<id>/logs/` (команди пляшки), `…/cross-app/logs/` (решта) |
| `cross` аварійно завершився (panic) | звіт збережено локально; `cross crash show` надрукує останній. Він нікуди не відправляється |
| Гра з анти-чітом не запускається | очікувано, див. [FAQ](faq.md#чи-запрацюють-онлайн-ігри-з-анти-чітом) |

Повідомити про проблему або результат гри: [шаблони issue](../.github/ISSUE_TEMPLATE/) (опис помилки та звіт про сумісність гри). До звіту варто додати вивід `cross doctor --json`.

## Що далі

- [FAQ](faq.md): анти-чіти, 32-бітні ігри, Rosetta, ліцензія GPTK.
- [Профілі сумісності](compat-profiles.md): як описати налаштування гри, щоб вона запускалась «з коробки».
- [Довідка команд](../README.md#довідка-команд) у README.
