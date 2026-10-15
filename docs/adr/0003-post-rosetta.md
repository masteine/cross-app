# ADR 0003 — Стратегія після Rosetta

- Статус: прийнято (15.10.2026, Спринт 8)
- Контекст: `SPRINT_PLAN.md`, розділ 0 (ризик 1), Спринт 8; ADR 0001
- Машина, на якій усе перевірялось: Apple M1 Pro, macOS 26.5.2 (25F84), Rosetta встановлена, Xcode CLT з SDK 26.5 (clang 21, ld-1267)

## Коротко

1. **Основний шлях без Rosetta на 2027 рік — CrossOver, який користувач встановлює сам** (зовнішній двигун, `translation = external`). CodeWeavers уже має ARM64-збірку з власним FEX, а ми лише керуємо нею: знаходимо застосунок, реєструємо його як двигун, імпортуємо пляшки, запускаємо програми через його `wine`.
2. **`Engine.translation` тепер визначає поведінку.** Новий двигун додається лише записом у `catalog.json`: `rosetta` і `native` завантажуються, `external` шукається в системі. Правила зібрано в одному місці: `cross-engine/src/policy.rs`.
3. **Власний Wine ARM64EC + FEX зараз не збираємо.** Upstream Wine ще не підтримує ARM64 macOS, FEX офіційно працює лише на Linux, а потрібні можливості ядра відкриваються тільки з обмеженим entitlement від Apple. До питання повертаємося, коли зміниться хоча б одна з цих умов (див. «Коли переглянути»).
4. **На «legacy games»-підмножину Rosetta не розраховуємо.** Apple не пояснює, як macOS 28 розпізнає гру, а перевірити це на цій машині неможливо.
5. **Справжнього запуску без Rosetta на цій машині не було.** Він тут неможливий без дій користувача. Прототип — це код шляхів `native` і `external`, перевірений на підставних бінарниках і бандлі. Що потрібно для справжнього запуску, описано в розділі «Що потрібно від користувача».

## Факти з джерелами

Позначки: **[перв.]** — первинне джерело (Apple, CodeWeavers, WineHQ, репозиторій проєкту); **[локально]** — перевірено на цій машині; **[вторин.]** — сторонній опис. Там, де джерела немає, це сказано прямо.

### Rosetta у macOS 27 і 28

- **[перв.]** Apple Developer News, 1.09.2026 ([developer.apple.com/news/?id=w5ngl9k2](https://developer.apple.com/news/?id=w5ngl9k2)): macOS 27 — останній реліз із Rosetta для застосунків. Підтримку Rosetta для старих непідтримуваних ігор, що спираються на Intel-фреймворки, збережуть. Про те, що саме вважається грою, там нічого немає.
- **[перв.]** Apple Support 102527, опубліковано 21.09.2026 ([support.apple.com/en-us/102527](https://support.apple.com/en-us/102527)): починаючи з macOS 28 Rosetta буде доступна лише для «певних старших непідтримуваних ігор». Критеріїв відбору немає.
- **[перв.]** Release notes macOS 26.4 (169228455, [посилання](https://developer.apple.com/documentation/macos-release-notes/macos-26_4-release-notes)): підтримка лишиться для старих ігор і для Intel-бінарників у Linux VM.
- **[перв.]** Release notes macOS 27 ([посилання](https://developer.apple.com/documentation/macos-release-notes/macos-27-release-notes); текст отримано через JSON-API документації Apple, бо сторінка рендериться скриптом):
  - 166398727: у бета-версіях є `sudo game-test-tool enable` для режиму legacy-ігор. Після його ввімкнення Rosetta вимикається, «не-ігрові» процеси можуть падати, а поза бета-релізами інструмент недоступний.
  - 176042635: усе Intel-ПЗ, крім legacy-ігор, несумісне з macOS 28.0.
  - 177192993 (виправлено): ігри не мають з'являтися у списку несумісних застосунків, а запис «Steam Game» можна ігнорувати.
- **[локально]** На macOS 26.5.2 `game-test-tool` немає (`which game-test-tool` нічого не знаходить).
- **Чи «ловить» legacy-режим Wine-процеси?** Первинного джерела немає ні в один бік. Apple не описує, як процес класифікується як гра. Wine — це загальний завантажувач (`wineloader`), а не Intel-гра з Intel-фреймворками, тож потрапляння під критерій сумнівне, але це лише **висновок, а не факт**. Перевірити можна тільки на бета-версії macOS з `game-test-tool` (див. крок В нижче). Опосередкована ознака: CodeWeavers прямо планує, що ARM64-збірка CrossOver працюватиме «без Rosetta 2», а не через цю підмножину (джерело нижче).

### CrossOver і CodeWeavers

- **[перв.]** CodeWeavers, «What's in and what's out for CrossOver 27», 11.06.2026 ([посилання](https://www.codeweavers.com/blog/mjohnson/2026/6/11/whats-in-and-whats-out-for-crossover-27)):
  - CrossOver Mac 27 працюватиме лише на macOS Sonoma+ і лише на Apple Silicon;
  - 32-бітні пляшки більше не запускатимуться;
  - зараз CrossOver потребує Rosetta 2, а ARM64-збірка працюватиме без неї.
- **[перв.]** CodeWeavers, «CrossOver Preview: The right to bear ARM64 on Mac», 31.07.2026 ([посилання](https://www.codeweavers.com/blog/mjohnson/2026/7/31/crossover-preview-the-right-to-bear-arm64-on-mac)). Сайт codeweavers.com повертає нашим інструментам 403, тому текст узято зі знімка Wayback Machine від 27.08.2026. Зміст:
  - у липні 2026 команда зробила власну версію FEX, сумісну з macOS, і випустила перші Mac ARM64-збірки;
  - збірки «universal»: ARM64-частина потребує **macOS 26.5+**, на старіших системах працюють Intel Wine і Rosetta;
  - у збірках є ARM64 DXMT;
  - D3DMetal поки немає (DX12 «скоро»);
  - багато лаунчерів ігор не працюють;
  - старі пляшки на ARM64 не конвертуються, потрібні нові;
  - CodeWeavers очікує розв'язати це до CrossOver 27 на початку 2027 року;
  - збірки призначені лише для тестування.
- **Вихідних кодів ARM64-прев'ю і «власного FEX» CodeWeavers я не знайшов.** Твердження «коди вийдуть разом з CrossOver 27» трапляється лише у сторонньому GitHub-issue, тому на нього не спираюся.
- **[перв.] Документація CodeWeavers про CLI і пляшки:**
  - [CrossOver User Guide (23.5.0), «Command Line»](https://www.codeweavers.com/support/docs/crossover-mac/index): `…/CrossOver.app/Contents/SharedSupport/CrossOver/bin/wine --bottle <назва> --cx-app <exe>`; пляшки лежать у `~/Library/Application Support/CrossOver/Bottles`;
  - [Advanced CrossOver Mac Configuration](https://support.codeweavers.com/advanced-crossover-mac-configuration): хуки працюють «у контексті Wine» з `CX_ROOT`, `CX_BOTTLE`, `WINEPREFIX` і тоді не потребують `--bottle`; уподобання `BottleDir`; шаблони `win10_64` тощо;
  - [Change Where CrossOver Stores Bottles](https://support.codeweavers.com/en_US/common-actions/change-the-bottle-directory-in-crossover-mac): `defaults write com.codeweavers.CrossOver BottleDir`, `CX_BOTTLE_PATH` з кількома теками через `:`.
- **[локально] На машині знайшовся CrossOver.** У постановці задачі сказано, що він не встановлений, але це так лише для `/Applications`. Насправді є **trial-версія CrossOver 26.2.0.39821 у `~/Applications/CrossOver.app`** і пляшка `Steam` (CrossOver 26.2). Я перевіряв лише читанням, нічого не змінював:
  - `Contents/SharedSupport/CrossOver/bin` — це симлінк на `CrossOver-Hosted Application`; у ньому є `wine` (Perl-скрипт), `wineloader` і `wineserver`; обидва Mach-O **лише x86_64**, тож CrossOver 26.2 працює через Rosetta;
  - код `bin/wine` і `lib/perl/CXBottle.pm`: пляшка береться з `--bottle`, далі з `$CX_BOTTLE`, далі `default`. Шукається вона як `<тека>/<назва>/system.reg` у кожному елементі `CX_BOTTLE_PATH`, після чого `WINEPREFIX` перевизначається на її реальний шлях. Абсолютний шлях у `--bottle` теж приймається;
  - `bin/wine --version` відпрацював (Product Version 26.2.0.39821);
  - `cxbottle.conf` пляшки має секцію `[Bottle]` з рядками `"Template" = "win10_64"`, `"Version" = "26.2.0.39821"`, `"WineArch" = "win64"`; `system.reg` починається з `#arch=win64`;
  - в уподобаннях CrossOver є `NSWindow Frame DemoNag`, отже це незареєстрований trial, який показує вікно-нагадування.

### Wine, FEX, Box64, Hangover

- **[перв.]** Brendan Shanks (CodeWeavers), wine-devel, 7.08.2026, «Wine on ARM64 macOS» ([тред](https://list.winehq.org/hyperkitty/list/wine-devel@list.winehq.org/thread/CKG5CEN2BE5VRXZ7O7NX4YUSBH3247WH/)):
  - у macOS уже є все потрібне для нативного ARM64 Wine, MR-и в upstream він надішле згодом;
  - x18: режим «custom x18 ABI» (macOS 26.5, `<os/arch/arm64.h>`);
  - 4K-сторінки: `posix_spawnattr_set_4k_page_size_np()`;
  - PAGEZERO ≥ 4 ГБ заважає `USER_SHARED_DATA` на 0x7ffe0000; з Xcode 26.4 його обходить прапорець лінкера `-x86_64_layout_emulation`;
  - TSO вмикається на потік через `thread_set_x86_64_compat()` і не успадковується новими потоками;
  - усе це потребує обмежених entitlements `com.apple.developer.cross-architecture-support` (платний акаунт) або `…-unmanaged` (безкоштовний) з provisioning profile, прив'язаним до UDID Mac. Альтернатива — вимкнути SIP, і її він не радить.
- **[перв.]** GitLab WineHQ (API, 15.10.2026):
  - останній тег — `wine-11.19` (2.10.2026);
  - злиті лише підготовчі MR-и Shanks, наприклад !11402 «Hard code the host address space limit on macOS» і !11668 «Populate the CPU name and vendor on ARM64 macOS»;
  - MR, що вмикає ARM64 macOS, серед злитих **не знайшов**. Сторонній !11638 «Support Apple silicon mac…» відкритий з 11.08.2026;
  - висновок: **upstream Wine 11.19 не збирається й не працює як ARM64 macOS-хост** (наскільки видно з MR-ів; самі гілки я не збирав).
- **[перв.]** ARM64EC в upstream Wine: повна підтримка емульованого x86-64 через ARM64EC з'явилася у Wine 10.0, січень 2025 (за хронологією CodeWeavers з блогу від 31.07.2026).
- **[перв.]** FEX ([github.com/FEX-Emu/FEX](https://github.com/FEX-Emu/FEX)): опис «A fast usermode x86 and x86-64 emulator for Arm64 Linux»; останній реліз FEX-2610 (7.10.2026); підтримки Darwin в описі немає. Issue [#5222](https://github.com/FEX-Emu/FEX/issues/5222) пояснює, що інтеграція з Wine йде через ARM64EC/WoW64-модулі, а не через звичайну arm64-DLL.
- **[вторин.]** [dbc-hbin/FEX-darwin](https://github.com/dbc-hbin/FEX-darwin), сторонній форк: 1 зірка, реліз v0.1.0 від 7.10.2026, основа FEX-2609.1.
  - Збирає `xtajit64.dll` (ARM64EC, x64) і `xtajit.dll` (WoW64, x86), потрібен llvm-mingw 22.1.8.
  - За README, це «не самостійний завантажувач»: Wine мусить надати хуки (shared data, JIT write-protection, JIT-алокація, WoW64 guest window).
  - Зв'язку з CodeWeavers не видно.
- **[перв.]** Hangover ([AndreRH/hangover](https://github.com/AndreRH/hangover)): «runs Win64 and Win32 applications on arm64 Linux»; x86_64 емулюється через ARM64EC і FEX (`libarm64ecfex.dll`), i386 — через `wowbox64.dll` (Box64) або `libwow64fex.dll`. Останній реліз hangover-11.16 (30.08.2026). Підтримки macOS немає.
- **[перв.]** Box64 ([ptitSeb/box64](https://github.com/ptitSeb/box64)): «Linux Userspace x86_64 Emulator … ARM64, RV64 and LoongArch Linux devices», v0.4.4. Сам по собі це емулятор Linux-процесів; для Wine-WoW64 є PE-модуль `wowbox64.dll` (лише i386, через Hangover). Для macOS не придатний.
- **[перв.]** DXMT ([3Shain/dxmt](https://github.com/3Shain/dxmt)): D3D10/11 поверх Metal для macOS/Wine, v0.80. Його ARM64-версія постачається в прев'ю CrossOver.

## Чи реально зібрати Wine ARM64EC + x86-емулятор на macOS: блокери

Перевірено маленькою C-програмою (clang `-arch arm64 -mmacosx-version-min=26.5`) на macOS 26.5.2 **[локально]**. Програма викликає ті самі примітиви, про які пише Shanks:

| Примітив | Без entitlement (ad-hoc підпис) | Що це означає |
|---|---|---|
| `thread_set_x86_64_compat(1)` (TSO на потік) | `KERN_FAILURE (5)` | без обмеженого entitlement x86-код з багатьма потоками не матиме x86-моделі пам'яті: гонки та падіння. FEX без апаратного TSO мусить емулювати бар'єри (повільно) |
| `os_set_custom_x18_abi_enabled(true)` | працює | x18 для TEB розв'язаний; у SDK позначено `API_AVAILABLE(macos(26.4))`, а Shanks пише «26.5» |
| `posix_spawnattr_set_4k_page_size_np` + `posix_spawn` | атрибут приймається, але запуск дає `EBADMACH (88) "Malformed Mach-o file"`, навіть якщо дочірній бінарник злінковано з `-x86_64_layout_emulation` | 4K-сторінки для Windows-семантики так просто не вмикаються. Що саме потрібно (entitlement, особливий Mach-O) — **не з'ясовано** |
| `mmap(MAP_JIT, RWX)` | працює без hardened runtime; з hardened runtime дає `EINVAL`, з `com.apple.security.cs.allow-jit` знову працює | JIT/W^X розв'язується звичайним entitlement (Спринт 9 його вже планує). `pthread_jit_write_protect_supported_np() = 1` |
| `mmap(RWX)` без `MAP_JIT` | `EACCES` | емулятор мусить писати код лише через `MAP_JIT` і перемикати W^X на потік |
| ad-hoc підпис з `com.apple.developer.cross-architecture-support-unmanaged` | процес убитий при запуску (код 137, SIGKILL) | обмежений entitlement без provisioning profile не працює. Потрібен Apple-акаунт, App ID і профіль, прив'язаний до UDID |
| `ld -x86_64_layout_emulation` | ld-1267 приймає прапорець з `-arch arm64` | PAGEZERO розв'язується лінкером |

Решта блокерів (з джерел вище):

- **Код Wine.** В upstream немає ARM64-macOS-хоста. Потрібна гілка CodeWeavers, але її кодів для прев'ю я не знайшов, або чужі незлиті патчі (!11638). Збирання Wine разом з тулчейном llvm-mingw займає години, а результат без хуків для FEX не запуститься.
- **Емулятор.** Офіційний FEX працює лише на Linux. Darwin-порт CodeWeavers не опублікований, а FEX-darwin — свіжий форк з одним автором. Box64 і Hangover для macOS не підходять.
- **Підпис і дистрибуція.** За Shanks, entitlement видається через App ID і provisioning profile з UDID конкретного Mac, тобто як для розробки. Чи можна поширювати такий завантажувач з Developer ID для довільних Mac, **первинного джерела не знайшов**. Як це робить CodeWeavers, не відомо. Це прямо зачіпає Спринт 9: hardened runtime, нотаризацію.
- **32 біти.** WoW64 через `xtajit.dll` теоретично можливий, але CodeWeavers відмовляється від 32-бітних пляшок навіть у CrossOver 27.
- **Графіка.** На ARM64 у прев'ю є DXMT, D3DMetal ще немає. Стан DXVK/MoltenVK під ARM64EC не перевіряв.

Висновок: **власна збірка у строки спринту нереальна.** Причина не в обсязі роботи: бракує частин, які від нас не залежать (upstream Wine, Darwin-FEX, entitlement від Apple).

## Рішення

### 1. Абстракція `translation` (зроблено)

| | `rosetta` | `native` | `external` |
|---|---|---|---|
| Звідки | `archive` (URL + SHA-256) | `archive` | `external: "crossover"`, пошук у системі |
| Install | завантажити; якщо Rosetta немає або macOS ≥ 28, попередити | перевірити `min_macos` і Apple Silicon **до** завантаження; після розпакування `wine` мусить мати arm64-зріз, інакше відмова `NotNative` | знайти `CrossOver.app`, записати `engines/<id>/engine.toml` з абсолютним `wine_bin`; нічого не копіюється |
| Запуск (`Runtime::open`, `BottleManager::create`) | якщо `wine` — x86_64-only Mach-O, а Rosetta немає, то `RosettaRequired`; на macOS ≥ 28 попередження в лог | `wine` не має бути x86_64-only | застосунок має існувати; якщо `bin/wineloader` x86_64-only, а Rosetta немає, то `RosettaRequired` (так поводиться CrossOver 26.2) |
| `cross doctor` | Rosetta не потрібна, якщо таких двигунів немає; на macOS ≥ 28 попередження, якщо лишились тільки вони | працює на arm64 | вважається придатним |

Архітектуру бінарника визначаємо за Mach-O-заголовком (`cross-engine/src/macho.rs`), а не лише за маніфестом. Підставні shell-скрипти в тестах трактуються як «нативні», тому тести інших крейтів не залежать від наявності Rosetta.

### 2. CrossOver як зовнішній двигун (зроблено, частково перевірено)

- Пошук: `$CROSS_CROSSOVER_APP`, далі `/Applications/CrossOver.app`, далі `~/Applications/CrossOver.app`. Версія береться з `CFBundleShortVersionString` через `plutil`.
- Пляшки: `CX_BOTTLE_PATH`, далі `defaults read com.codeweavers.CrossOver BottleDir`, далі `~/Library/Application Support/CrossOver/Bottles`. Пляшкою вважається тека з `system.reg`, як у самому CrossOver. Додатково читаються `Template`, `Version` і `WineArch` з `cxbottle.conf`, а `#arch=` з `system.reg` слугує запасним варіантом.
- Імпорт (`cross crossover import <назва>`): `bottles/<id>/prefix` стає **симлінком** на пляшку CrossOver. Нічого не копіюється й не змінюється. `bottle delete` видаляє лише посилання, `bottle clone` для таких пляшок заборонено, а `bottle create --engine crossover` відсилає до імпорту.
- Запуск: `Runtime` для `external` встановлює `CX_ROOT`, `CX_BOTTLE`, `CX_BOTTLE_PATH` і `WINEPREFIX` на реальний шлях пляшки, а далі звичайний `cross run` викликає `…/bin/wine <програма>`. Саме так `bin/wine` у CrossOver 26.2 знаходить пляшку, якщо `--bottle` не задано.
- 32-бітна пляшка імпортується з попередженням, бо CrossOver 27 таких не запускає.

### 3. Власний `native`-двигун: відкладено

Каталог готовий: досить додати запис `translation: "native"`, `min_macos: "26.5"` з реальним `archive`. **Такого артефакту зараз немає**, а URL і хеші вигадувати не можна. Тому в `catalog.json` native-двигуна немає, а шлях перевірено на синтетичному архіві з arm64-заголовком.

### Коли переглянути

- MR-и Shanks злито в upstream Wine; перевірити `git log --grep` у wine/wine за ключовими словами `x18`, `4k`, `PAGEZERO`.
- FEX отримав офіційну підтримку Darwin, або CodeWeavers опублікував коди CrossOver 27 разом з FEX.
- Apple задокументувала, як отримати `cross-architecture-support` для Developer ID-дистрибуції.
- Вийшла бета macOS 28 (за досвідом попередніх років — червень 2027; дати від Apple немає): перевірити `game-test-tool` з Wine.

## Прототип запуску без Rosetta: що є і чого немає

- **Є (код + тести на підставних даних):**
  - `native`: установка зі справжнього HTTP-архіву з arm64-Mach-O-заголовком і запуск без Rosetta. Відмова для x86_64-«native» і для `min_macos` вище за систему.
  - `external`: реєстрація підставного `CrossOver.app` (бінарний `Info.plist`), імпорт двох пляшок, `cross run <пляшка> notepad`. Підставний `wine` отримав правильні `CX_ROOT`, `CX_BOTTLE`, `CX_BOTTLE_PATH` і `WINEPREFIX`. Перевірено також відмову clone/create і те, що `delete` не чіпає пляшку CrossOver.
- **Є (на справжніх даних, лише читання):**
  - справжній CrossOver 26.2 знайдено, версію прочитано, `wineloader` розпізнано як x86_64;
  - справжню пляшку `Steam` розпізнано (`win64`, `win10_64`, 26.2.0.39821);
  - справжній Wine 11.18 від Gcenx розпізнано як x86_64; без Rosetta він дає `RosettaRequired`;
  - пробні виклики примітивів macOS (таблиця вище).
- **Немає:**
  - **справжнього запуску Windows-програми без Rosetta.** На цій машині немає ARM64 Wine. Встановлений CrossOver 26.2 сам працює через Rosetta, а запуск програми в ньому, найімовірніше, показав би вікно trial-нагадування (`DemoNag`). Тому програм через нього не запускав зовсім, навіть через Rosetta;
  - імпорт справжньої пляшки в cross-app з подальшим запуском не робився: незрозуміло, чи не змінить CrossOver пляшку користувача під час оновлення.

## Що потрібно від користувача (кроки на погодження)

**А. Справжній запуск без Rosetta через CrossOver ARM64 Preview** (найкоротший шлях):

1. Завантажити ARM64-збірку з [Preview Center](https://www.codeweavers.com/preview) (у назві є «arm64»; розмір CodeWeavers не вказує; можливо, потрібен вхід в акаунт). Встановити окремо від 26.2, наприклад як `~/Applications/CrossOver Preview.app`.
2. У CrossOver створити **нову** 64-бітну пляшку, наприклад `arm-test` (старі пляшки на ARM64 не конвертуються).
3. Виконати:

   ```bash
   export CROSS_CROSSOVER_APP="$HOME/Applications/CrossOver Preview.app"
   cross crossover status        # має показати «Wine loader: arm64… (runs without Rosetta 2)»
   cross engine install crossover
   cross crossover import arm-test
   cross run arm-test notepad
   ```

4. Для доказу «без Rosetta» на цьому ж Mac: поки програма працює, перевірити `file` бінарника, з якого запущено `wineserver` (`ps -o command= -p $(pgrep -f wineserver)`), і колонку «Kind» в Activity Monitor: має бути «Apple», а не «Intel». За CodeWeavers, на macOS 26.5+ universal-збірка сама бере ARM64-частину, тож Rosetta можна не видаляти.

**Б. Експеримент з власною збіркою** (довго, без гарантії результату):

1. Apple Developer-акаунт: створити App ID із cross-architecture entitlement, зареєструвати UDID цього Mac, завантажити provisioning profile (процедура з треду Shanks).
2. Встановити тулчейн llvm-mingw 22.1.8 (розмір і URL уточнити на сторінці релізів проєкту llvm-mingw; я його не завантажував), CMake і Python 3.
3. Зібрати DLL-и FEX:

   ```bash
   git clone https://github.com/dbc-hbin/FEX-darwin && cd FEX-darwin
   git submodule update --init --depth 1   # підмодулі зі списку в README
   python3 Darwin/build.py --output build/local/darwin01 --jobs 8
   ```

4. Потрібен ще Wine для ARM64 macOS з хуками для FEX. В upstream його немає, тож лишаються патчі Shanks, коли їх опублікують. Без цього кроки 1–3 нічого не запускають.

**В. Перевірка legacy-games-режиму:** потрібна бета macOS з `game-test-tool` (за release notes — лише в бета-релізах): `sudo game-test-tool enable`, перезавантаження, `cross run <пляшка> notepad` з Rosetta-двигуном. Встановлювати бету на робочий Mac — рішення користувача.

## Наслідки

- Для `cross-cli/src/main.rs` (його змінює лід) потрібна реєстрація команди `cross crossover` і перехід `doctor` на `evaluate_with_engines`. Деталі — у звіті спринту.
- Спринт 9: до entitlements двигуна (`allow-jit` тощо) додається питання `cross-architecture-support`, якщо колись постачатимемо власний ARM64 Wine. Для CrossOver воно нас не стосується: підписує CodeWeavers.
- Спринт 6 (GUI): майстер першого запуску має пропонувати «Використати CrossOver», якщо `cross crossover status` його знаходить, а на macOS ≥ 28 без native/external-двигуна показувати попередження з `doctor`.
- CrossOver — платний продукт третьої сторони. Без нього в користувача на macOS 28 може не лишитися робочого двигуна, доки не з'явиться відкритий ARM64 Wine. Цей ризик лишається відкритим.
