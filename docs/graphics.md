# Графіка, продуктивність і ввід

Що вміє cross-app у Спринті 7, звідки взято кожен перемикач і що з цього перевірено на справжньому Wine. Скрізь, де написано «не перевірено», це означає саме це.

## Швидкий старт

```bash
cross bottle graphics games                          # показати план запуску пляшки
cross bottle graphics games --backend dxvk           # бекенд пляшки
cross bottle graphics games --appid 1086940 --backend d3dmetal --metal-hud on
cross bottle graphics games --sync msync --cpu-count 4
cross bottle graphics games --metal-hud-log on       # FPS у лог
cross bottle fps ~/Library/Application\ Support/cross-app/bottles/games/logs/steam.log
cross bottle preset games retina on
cross bottle preset games gamepad --hidraw off

cross engine import-gptk ~/Downloads/Game_Porting_Toolkit_4.0.dmg
cross engine d3dmetal status wine-devel-11.18
cross engine d3dmetal enable <engine>                # оверлей D3DMetal
```

`cross bottle graphics` без прапорців нічого не змінює і показує, з чим піде запуск: запитаний і фактичний бекенд, який `wine` стартує, змінні середовища, DLL overrides, що вміє двигун, а також попередження про все, що попросили, але не застосували.

Налаштування зберігаються в `bottle.toml`: `backend` і `[graphics]` для пляшки, `[apps.profile.graphics]` для гри. Значення гри перекриває значення пляшки, `inherit` повертає успадкування. Прапорці `cross run --metal-hud/--msync/...` діють поверх усього, лише на один запуск.

## Бекенди

| Бекенд | Що запускається | DLL overrides |
|---|---|---|
| `dxvk` | `wine` двигуна | `d3d10core`, `d3d11` = native (DLL з `cross deps install <пляшка> dxvk`) |
| `d3dmetal` з оверлеєм | `wine` з `engines/<id>/d3dmetal/` | `d3d10`, `d3d10core`, `d3d11`, `d3d12`, `dxgi` = builtin |
| `d3dmetal` без оверлею | `wine` двигуна | для гри з явним `d3dmetal`: `d3d10core`, `d3d11` = builtin (вбудований wined3d); для типового бекенду пляшки нічого не змінюється |

Типовий бекенд пляшки — `d3dmetal`. Пляшку, де встановлено DXVK, варто явно перемкнути: `cross bottle graphics <пляшка> --backend dxvk`. Інакше, щойно для двигуна з'явиться оверлей D3DMetal, ігри пляшки підуть через нього.

Steam: `cross steam launch` бере бекенд гри (або пляшки) і стартує `steam.exe` потрібним `wine`, тож гра, яку запускає Steam, завантажує ті самі DLL. Обмеження зі Спринту 5 лишається: якщо Steam уже запущений, гра отримує середовище й `wine` того Steam.

### Як підключається D3DMetal

Джерела: README Apple для GPTK-сумісних збірок Wine у переказі [frankea/Whisky#163](https://github.com/frankea/Whisky/issues/163) і код `GPTKImporter*.swift` з [frankea/Whisky](https://github.com/frankea/Whisky) (активний форк Whisky).

1. `lib/external/` з GPTK (`D3DMetal.framework`, `libd3dshared.dylib`) кладеться поруч із бібліотеками Wine: `<wine>/lib/external/`.
2. PE-форвардери Apple (`d3d10.dll`, `d3d11.dll`, `d3d12.dll`, `dxgi.dll` з `lib/wine/x86_64-windows/` тулкіта) заміняють builtin-DLL Wine.
3. `<wine>/lib/wine/x86_64-unix/{d3d10,d3d11,d3d12,dxgi}.so` стають символьними посиланнями на `../../external/libd3dshared.dylib`.

Замінити DLL у самому двигуні означало б забрати власний Direct3D Wine (і `dxgi`, без якого не працює наш DXVK) в усіх пляшок. Підмінити їх для окремого процесу не можна: Wine шукає DLL спершу у власному `lib/wine`, а `WINEDLLPATH` перевіряє лише після нього (`set_dll_path()` у `dlls/ntdll/unix/loader.c`, Wine 11.18). Builtin-DLL у `system32` Wine теж не завантажує. Тому `cross engine d3dmetal enable` будує **оверлей**: APFS-клон двигуна (`cp -cR`, займає майже нуль місця, ~1,5 с) у `engines/<id>/d3dmetal/`, куди розкладено файли Apple. Ігри на `d3dmetal` запускаються `wine` з оверлею, усі інші запускаються незмінним двигуном. `disable` видаляє оверлей. `cross engine remove` прибирає його разом із двигуном.

`cross engine import-gptk` тепер копіює і `lib/wine/x86_64-windows/*.dll` (раніше лише `lib/external`), перевіряє, що чотири форвардери позначені як Wine builtin (рядок `Wine builtin DLL` за зсувом 0x40, як у `builtin_signature` Wine), і створює unix-посилання. Імпорт, зроблений до Спринту 7, `d3dmetal status` показує як неповний, і його треба повторити. `nvapi64.dll` і `nvngx-on-metalfx.dll` копіюються, але в оверлей не кладуться: за даними frankea/Whisky, `nvapi64` валить CEF-хелпер Steam, а для `nvngx` перейменування потрібне з їхнього реверс-інжинірингу, не з документації.

**Головне обмеження.** D3DMetal працює лише у збірках Wine, похідних від CrossOver. Upstream Wine (наш каталог, Gcenx `wine-devel-11.18`) завантажує файли Apple, після чого кожен процес, що виконує код D3DMetal, падає. Причина в тому, що `libd3dshared.dylib` патчить таблицю unixcall під внутрішню структуру CrossOver (#163, дослідження з `nm` і робочою збіркою з вихідників CrossOver 25). Тому:

- у каталозі двигунів з'явилося поле `gptk_capable` (типово `false`; жоден двигун каталогу його не має);
- `cross engine d3dmetal enable` для такого двигуна відмовляє. З `--force` оверлей будується, але з попередженням у кожному плані запуску;
- справжній D3DMetal-двигун (Gcenx `game-porting-toolkit`, PhoenicisOrg/winecx, CrossOver, тобто Спринт 8) треба додати в каталог із `gptk_capable: true`.

**Перевірено** (15.10.2026, Wine 11.18, синтетичний GPTK, не код Apple): оверлей будується, `wine --version` з оверлею працює, `cmd` в оверлеї працює. З `WINEDEBUG=+loaddll` запуск на `d3dmetal` намагається завантажити `dxgi.dll` саме з оверлею (синтетичний файл, тому `Failed to load DLL`), а без оверлею вантажиться builtin `dxgi` Wine разом із `wined3d`. Те саме перевіряє тест `cargo test -p cross-deps --test real_graphics -- --ignored`. **Не перевірено:** справжній D3DMetal (GPTK на цій машині не імпортовано, потрібен Apple ID користувача), справжній GPTK-сумісний двигун, рендеринг будь-якої гри.

### DXVK: виправлення зі Спринту 7

Під час перевірки з'ясувалося, що `cross deps install <пляшка> dxvk` зі Спринту 4 фактично не вмикав DXVK. Компонент каталогу — «builtin»-репак Gcenx, його DLL мають маркер `Wine builtin DLL`. Такий файл у `system32` Wine 11.18 з override `native` не вантажить (`status=c0000135`), а з `native,builtin` бере власний `d3d11` (wined3d). Тепер крок `copy` verb `dxvk` має `native = true`: під час копіювання маркер замінюється стандартним початком DOS-заглушки. Результат побайтово (SHA-256) збігається з небілтін-архівом того ж релізу Gcenx (`dxvk-macOS-async-v1.10.3-20230507-repack.tar.gz`, перевірено для всіх чотирьох DLL). Після цього `d3d11.dll` вантажиться як `native`. Наявним пляшкам потрібно `cross deps install <пляшка> dxvk --force`. **Не перевірено:** що DXVK у парі з `dxgi` Wine реально малює гру. Gcenx пише, що прибрав `dxgi.dll` з репаку, бо на macOS він не потрібен, але сам рендеринг ми не бачили.

## Перемикачі

| Налаштування | Змінна | Джерело | Умова |
|---|---|---|---|
| `--metal-hud` | `MTL_HUD_ENABLED=1` | Apple, «Monitoring your Metal app's graphics performance» | — |
| `--metal-hud-log` | `MTL_HUD_ENABLED=1` + `MTL_HUD_LOG_ENABLED=1` | там само: «Enables logging of per-frame statistics» | — |
| `--dxvk-hud fps,...` | `DXVK_HUD` | README DXVK | лише бекенд `dxvk` |
| `--sync esync` | `WINEESYNC=1` | патчі esync (`README.esync` у Gcenx/game-porting-toolkit) | двигун читає `WINEESYNC` |
| `--sync msync` | `WINEMSYNC=1` і `WINEESYNC=1` | marzent/wine-msync; обидві змінні виставляє Whisky (`BottleSettings.swift`: «D3DM detects ESYNC…»), не перевірено | двигун читає `WINEMSYNC` |
| `--cpu-count N` | `WINENCPU=N` | `get_cpu_count_override()` у `dlls/ntdll/unix/system.c` Wine з GPTK Apple (Gcenx/game-porting-toolkit) | двигун читає `WINENCPU` |

**Що вміє двигун, визначається за його файлами** (`cross_engine::features`): у `ntdll.so` шукаються рядки `WINEESYNC`, `WINEMSYNC`, `WINENCPU`, у `winemac.so` рядок `RetinaMode`. Перемикач, який двигун не читає, не виставляється, а в плані з'являється попередження. На `wine-devel-11.18` з каталогу знайдено лише `RetinaMode`. **esync, msync і обмеження кількості CPU у цій збірці не існують**, тому прапорці `--esync`/`--msync` у `cross run` на ній нічого не роблять.

**Обмеження CPU на upstream Wine.** Перевірено у вихідниках Wine 11.18: змінної для кількості процесорів немає (зокрема, у `dlls/ntdll/unix/system.c` немає `WINE_CPU_TOPOLOGY`), `start /affinity` ігнорується (`programs/start/start.c`: «/affinity is ignored»), а прив'язку потоків wineserver робить лише через `sched_setaffinity`, якого на macOS немає. Отже, на upstream-двигунах цієї функції немає.

**MetalFX і Metal 4: не зроблено.** Apple ці перемикачі не документує: ні в документації Metal HUD, ні в публічному репозиторії `apple/game-porting-toolkit`. `D3DM_ENABLE_METALFX`, `D3DM_MTL4` і `D3DM_SUPPORT_DXR` відомі лише з реверс-інжинірингу frankea/Whisky. Їх можна задати вручну через `--env`, але як «прапорці» cross-app їх не пропонує.

### FPS для бенчмарків

З `--metal-hud-log on` Metal HUD раз на секунду пише рядок `metal-HUD: <кадр>,<пам'ять GPU>,<пам'ять процесу>,<інтервал>,<час GPU>,...` з парою значень на кожен кадр (формат від Apple). `cross bottle fps <файл>` рахує FPS як кількість пар у рядку, тож одиниці (Apple їх не вказує) не потрібні. Куди саме йдуть ці рядки, Apple не уточнює («to the console»). Якщо в stderr гри, вони потраплять у `logs/steam.log` або в термінал `cross run`. Якщо лише в unified log, їх можна витягти так: `log show --last 10m --predicate 'eventMessage CONTAINS "metal-HUD:"' > hud.log`. **Не перевірено на грі.** Для DXVK документованого способу писати FPS у файл немає.

## Retina, DPI, вікна

- `cross bottle preset <id> retina on|off` записує `HKCU\Software\Wine\Mac Driver\RetinaMode` = `y`/`n` (рядок; Wine перевіряє перший символ, `IS_OPTION_TRUE`). Перевірено у `dlls/winemac.drv/macdrv_main.c` Wine 11.18. Значення читається лише з ключа пляшки, не з `AppDefaults` гри («DPI and monitor sizes should be consistent»), і лише під час старту Mac-драйвера. Запис у реєстр перевірено наживо, візуальний ефект не перевірено.
- DPI: `cross bottle preset <id> dpi 192` (Спринт 3).
- **Безрамкове вікно: немає.** `setup_options()` Mac-драйвера такого параметра не має. Безрамковий режим вмикається в налаштуваннях самої гри. Віртуальний робочий стіл (`preset virtual-desktop`) лишається альтернативою.

## DXVA: відмовлено

Пресет DXVA, перенесений зі Спринту 3, не робиться. У Wine 11.18 `dxva2` не має декодера: усі методи `IDirectXVideoDecoderService` у `dlls/dxva2/main.c` повертають `E_NOTIMPL`, а параметра в реєстрі, який би щось вмикав, немає.

## Геймпади

Wine не використовує Game Controller framework від Apple. Драйвер `winebus` на macOS має два бекенди (Wine 11.18, `dlls/winebus.sys/`): SDL2 (`bus_sdl.c` через `dlopen` бібліотеки `libSDL2`, яку містить двигун Gcenx) та IOHID (`bus_iohid.c`, HID-пристрої з usage «joystick»/«gamepad»). `cross bottle preset <id> gamepad [--sdl on|off] [--hidraw on|off] [--map on|off]` пише DWORD-значення в `HKLM\System\CurrentControlSet\Services\WineBus`: `Enable SDL` (типово 1), `DisableHidraw` (типово 0), `Map Controllers` (типово 1, SDL-контролери як XInput-геймпади). Вони діють після перезапуску пляшки. Підказки SDL (`SDL_JOYSTICK_HIDAPI`, `SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS`, `SDL_GAMECONTROLLER_USE_BUTTON_LABELS` з `SDL_hints.h` SDL2) задаються через `--env`, але `winebus` живе в `winedevice.exe` і бачить середовище процесу, який запустив сесію Wine пляшки.

**Не перевірено:** жоден контролер (Xbox, DualSense, Switch Pro). Фізичних контролерів тут немає. Перевірено лише запис і читання значень у реєстрі справжнього префікса.

## Що потребує користувача

- Імпорт справжнього GPTK (`cross engine import-gptk`, потрібне завантаження з developer.apple.com з Apple ID) і GPTK-сумісний двигун, щоб побачити справжній D3DMetal.
- Тест трьох контролерів.
- Запуск ігор: FPS-лог, Retina, DXVK-рендеринг.
