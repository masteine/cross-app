# Сторонні компоненти та ліцензії

Цей файл описує, чий код і які дані використовує cross-app, під якими ліцензіями, і що з цього cross-app **поширює** сам, а що лише **завантажує на Mac користувача** з офіційних джерел під час роботи.

> Складено 15.10.2026 для v0.1.0-beta (Спринт 10). Це інженерний перелік, а не юридичний висновок. Перед публічним релізом його варто показати юристу (див. [docs/release-checklist.md](docs/release-checklist.md)).

## 1. Ліцензія самого cross-app

У `Cargo.toml` (`[workspace.package] license = "MIT"`) вказано MIT, але **файлу `LICENSE` у репозиторії немає**. Вибір і додавання ліцензії залишаються за власником проєкту. Доки файлу немає, формально діє лише запис у `Cargo.toml`.

## 2. Що входить у бінарник `cross` (поширюється разом із cross-app)

`cross` статично збирається з Rust-крейтів із `Cargo.lock`. Перелік нижче отримано командою `cargo metadata --offline --filter-platform aarch64-apple-darwin` для залежностей `cross-cli` (без dev-залежностей) 15.10.2026: **166 сторонніх крейтів**.

| Ліцензія (SPDX з `Cargo.toml` крейта) | Кількість |
|---|---|
| `MIT OR Apache-2.0` (та еквівалентні записи `Apache-2.0 OR MIT`, `MIT/Apache-2.0`) | 103 |
| `MIT` | 29 |
| `Unicode-3.0` | 18 |
| інші, перелічені нижче | 16 |

Крейти з ліцензіями, відмінними від MIT / Apache-2.0:

| Крейт | Версія | Ліцензія |
|---|---|---|
| aho-corasick, memchr | 1.1.5, 2.8.3 | `Unlicense OR MIT` |
| hyper-rustls, rustls | 0.27.10, 0.23.46 | `Apache-2.0 OR ISC OR MIT` |
| icu_collections, icu_locale_core, icu_normalizer, icu_normalizer_data, icu_properties, icu_properties_data, icu_provider, litemap, potential_utf, tinystr, writeable, yoke, yoke-derive, zerofrom, zerofrom-derive, zerotrie, zerovec, zerovec-derive | 2.x / 0.x | `Unicode-3.0` |
| unicode-ident | 1.0.27 | `(MIT OR Apache-2.0) AND Unicode-3.0` |
| lru-slab | 0.1.3 | `MIT OR Apache-2.0 OR Zlib` |
| tinyvec | 1.13.3 | `Zlib OR Apache-2.0 OR MIT` |
| option-ext | 0.2.0 | `MPL-2.0` |
| ring | 0.17.14 | `Apache-2.0 AND ISC` |
| rustix | 1.1.5 | `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT` |
| rustls-webpki, untrusted | 0.103.15, 0.9.0 | `ISC` |
| ryu | 1.0.23 | `Apache-2.0 OR BSL-1.0` |
| subtle | 2.6.1 | `BSD-3-Clause` |
| sync_wrapper | 1.0.2 | `Apache-2.0` |
| webpki-roots | 1.0.9 | `CDLA-Permissive-2.0` |

Що з цього випливає:

- Усі ліцензії дозвільні; копілефту, що поширювався б на код cross-app, немає. `MPL-2.0` (option-ext) — файловий копілефт: вимагає відкривати лише зміни в самих файлах цього крейта, яких ми не робимо.
- MIT, Apache-2.0, BSD, ISC, Unicode-3.0 і CDLA вимагають класти **тексти ліцензій і copyright-рядки** поряд із бінарником. Зараз вони не збираються. **До релізу:** згенерувати повний файл ліцензій (наприклад `cargo about generate` або `cargo deny check licenses` у CI, Спринт 9) і покласти його в `.app`/DMG. Пункт є в [чеклісті релізу](docs/release-checklist.md).
- Перелік треба перегенеровувати при кожній зміні `Cargo.lock` (GUI зі Спринту 6 додасть Tauri і JS-залежності; їхні ліцензії сюди ще не входять).

## 3. Що cross-app завантажує під час роботи (не входить у дистрибутив)

Ці файли cross-app **не перепаковує і не розповсюджує**. Їх завантажує сам користувач командою `cross ...` з офіційних адрес, закріплених SHA-256 у каталогах репозиторію (`crates/cross-engine/catalog.json`, `crates/cross-deps/verbs.toml`, `crates/cross-steam/steam-patches.toml`, `crates/cross-steam/src/covers.rs`). Інших мережевих адрес у коді немає.

| Компонент | Що саме і звідки | Ліцензія | Джерело відомостей про ліцензію |
|---|---|---|---|
| **Wine** 11.18 (devel і staging) | збірки Gcenx: `github.com/Gcenx/macOS_Wine_builds/releases/download/11.18/wine-{devel,staging}-11.18-osx64.tar.xz` (`cross engine install`) | LGPL-2.1-or-later | файл `LICENSE` у репозиторії Wine: «GNU Lesser General Public License … version 2.1 … or any later version» ([wine-mirror/wine](https://github.com/wine-mirror/wine/blob/master/LICENSE)) |
| ↳ wine-mono 11.3.0 | вкладено в архів Gcenx (`share/wine/mono`) | суміш: переважно MIT і LGPL; окремі частини GPL з винятком, BSD, MS-PL, zlib | `COPYING` у [madewokherd/wine-mono](https://github.com/madewokherd/wine-mono/blob/master/COPYING) |
| ↳ wine-gecko 2.47.4 | вкладено в архів Gcenx (`share/wine/gecko`) | MPL (версію за upstream-файлом не звірено; пакунки дистрибутивів для 2.47.4 вказують MPL-2.0) | пакунки Arch/Fedora; upstream `LICENSE` не отримано |
| ↳ бібліотеки MacPorts | вкладено в архів Gcenx (`lib/`): FreeType, GnuTLS (+ nettle, hogweed, gmp, libtasn1, p11-kit, libidn2, libunistring), SDL2, MoltenVK, Vulkan loader, libinotify, ICU, libxml2, libxslt, libpng, zlib, bzip2, xz/lzma, zstd, lz4, brotli, libffi, libiconv, libintl, libpcap, MacPorts legacy-support | кожна своя; **поштучно не звірено** | склад — за вмістом встановленого архіву і README [Gcenx/macOS_Wine_builds](https://github.com/Gcenx/macOS_Wine_builds) |
| **DXVK** 1.10.3 для macOS | `github.com/Gcenx/DXVK-macOS/releases/download/v1.10.3-20230507-repack/dxvk-macOS-async-v1.10.3-20230507-repack-builtin.tar.gz` (`--dxvk`) | zlib | `LICENSE` у [doitsujin/dxvk](https://github.com/doitsujin/dxvk/blob/master/LICENSE); форк [Gcenx/DXVK-macOS](https://github.com/Gcenx/DXVK-macOS) позначений як «Zlib license» |
| **MoltenVK** 1.4.2 | `github.com/KhronosGroup/MoltenVK/releases/download/v1.4.2/MoltenVK-macos.tar` (`--dxvk`) | Apache-2.0 | `LICENSE` у [KhronosGroup/MoltenVK](https://github.com/KhronosGroup/MoltenVK/blob/main/LICENSE); текст ліцензії лежить і в самому архіві |
| **Steam** (Windows-клієнт) | `cdn.akamai.steamstatic.com/client/installer/SteamSetup.exe` (`cross steam install`) | пропрієтарна Valve; [Steam Subscriber Agreement](https://store.steampowered.com/subscriber_agreement/) | угоду користувач приймає сам під час входу в Steam |
| **Visual C++ 2015–2022 runtime** 14.36 | `download.visualstudio.microsoft.com/.../VC_redist.{x86,x64}.exe` (`cross deps install … vcrun2022`) | Microsoft Software License Terms | [Redistributing Visual C++ files](https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files) |
| Обкладинки ігор | `cdn.cloudflare.steamstatic.com/steam/apps/<appid>/…jpg` (`cross steam covers`) | належать правовласникам ігор | кешуються локально лише для показу користувачу |

Зауваги, знайдені під час перевірки:

- **В архівах Gcenx немає текстів ліцензій.** У встановленому `wine-devel-11.18` і в репаку DXVK не знайшлося жодного файлу `LICENSE`/`COPYING`; репозиторій збірок Gcenx не декларує ліцензії для власних скриптів. Обов'язки щодо LGPL (текст ліцензії, доступ до коду) лежать на тому, хто поширює бінарники, тобто на Gcenx. Поки cross-app лише завантажує їх з GitHub Releases Gcenx, а не кладе у свій DMG, нам достатньо вказати джерело. **Якщо Спринт 9 вирішить вбудувати двигун у `.app`/DMG, тоді треба покласти поряд COPYING.LIB, посилання на вихідний код Wine 11.18 і ліцензії вкладених бібліотек.**
- Запускаючи `wineboot`, cross-app вимикає завантаження Mono/Gecko (`WINEDLLOVERRIDES=mscoree,mshtml=`). Копії в архіві Gcenx лишаються на диску, але в пляшку автоматично не ставляться.

## 4. Що користувач надає сам

| Компонент | Як потрапляє на Mac | Ліцензія |
|---|---|---|
| **Apple Game Porting Toolkit / D3DMetal** | користувач сам завантажує `Game_Porting_Toolkit_*.dmg` з [developer.apple.com](https://developer.apple.com/games/) (потрібен Apple ID) і передає шлях у `cross engine import-gptk`; файли копіюються в його власну теку даних | ліцензія Apple, яку користувач приймає при завантаженні. Дозволу на редистрибуцію D3DMetal вона явно не дає (див. [обговорення на форумах Apple](https://developer.apple.com/forums/thread/841547)) |
| winetricks | лише якщо користувач сам його встановив (`brew install winetricks`) | LGPL-2.1 ([COPYING](https://github.com/Winetricks/winetricks/blob/master/COPYING)) |
| Ігри та програми | користувач встановлює сам | ліцензії їхніх власників |

## 5. Підтвердження: cross-app не поширює GPTK / D3DMetal

Перевірено 15.10.2026 на гілці Спринту 10:

1. У репозиторії немає жодного бінарного файлу: `git ls-files | xargs file` показує лише текстові файли.
2. У каталогах завантажень (`catalog.json`, `verbs.toml`, `steam-patches.toml`) немає адрес Apple і немає збірки Wine з GPTK від Gcenx. Її свідомо не додано, бо вона постачається разом із D3DMetal (рішення Спринту 2).
3. `crates/cross-engine/src/gptk.rs` лише монтує образ, який користувач вказав сам, тільки для читання, і копіює `lib/external/` у його теку даних (`~/Library/Application Support/cross-app/gptk`). Нічого не відправляється в мережу й не вкладається в дистрибутив.
4. **До кожного релізу** (пункт чекліста): перевірити, що в зібраному `.app`/DMG немає `D3DMetal*`, `libd3dshared*` і теки `lib/external`.

## 6. Steam: умови використання

cross-app не обходить і не змінює клієнт Steam. Він запускає офіційний Windows-клієнт під Wine з прапорцями запуску й налаштуваннями реєстру Wine (`steam-patches.toml`); паролів не бачить і не зберігає. Угода Steam забороняє «Cheats, mods, hacks, or any other unauthorized third-party software» в іграх (розділ 4.B Steam Subscriber Agreement). cross-app не змінює файлів ігор. Ігри з анти-чітом під Wine на macOS не підтримуються, і cross-app не намагається їх запускати в обхід захисту.
