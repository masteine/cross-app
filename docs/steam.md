# Steam у cross-app

Windows-клієнт Steam ставиться в пляшку і працює під Wine. Інтеграції зі Steam для macOS немає.

## Швидкий старт

```bash
cross bottle create steam
cross steam install steam      # закріплений SteamSetup.exe, тиха установка, патчі
cross steam run steam          # вікно Steam: увійти, встановити гру
cross steam games steam        # список встановлених ігор
cross steam launch steam 220   # запуск через Steam (-applaunch)
```

Налаштування окремої гри:

```bash
cross deps install steam dxvk
cross steam profile steam 220 --backend dxvk --env DXVK_HUD=fps --arg -novid
cross steam profile steam 220            # показати
cross steam profile steam 220 --clear    # прибрати
```

## Як це влаштовано

- **Інсталятор.** `SteamSetup.exe` завантажується з CDN Valve і перевіряється за SHA-256 із `steam-patches.toml`, після чого запускається з `/S`. Якщо Valve випустить новий інсталятор, перевірка навмисно впаде. Лікування: нова ревізія патчів із новим хешем, а не вимкнення перевірки.
- **Патчі (`steam-patches.toml`).** Файл містить прапорці запуску `steam.exe` і виправлення реєстру. Вбудована копія йде з бінарником. Новішу можна взяти командою `cross steam patches --update` (URL у `steam_patches_url` у `config.toml` або через `--url`). Приймається тільки файл з більшою `revision`. Оновлені registry-патчі застосовуються до пляшки при наступному `cross steam run/launch`.
- **Межі для оновлень із мережі.** Реєстр можна змінювати лише в `HKCU\Software\Wine` і `HKCU\Software\Valve`. Аргументи мають бути простими `-прапорцями`. Змінні `WINEPREFIX`, `WINEARCH`, `WINE`, `WINESERVER`, `PATH` і `DYLD_*` заборонені.
- **Бібліотека.** Шляхи бібліотек беруться з `steamapps/libraryfolders.vdf` (нового й старого формату) і перетворюються на шляхи macOS через `dosdevices` префікса. Ігри читаються з `appmanifest_*.acf`. Бібліотеки на інших дисках (`D:\SteamLibrary`) показуються за реальним шляхом.
- **Обкладинки.** `cross steam covers` завантажує `library_600x900.jpg`, а якщо її немає, то `header.jpg` з CDN Steam за appid. Файли кешуються в `cache/covers/<appid>.jpg`, у кеш потрапляють лише JPEG.

## Поточні патчі та їхнє походження

Усі патчі взято з verb `steam` у [winetricks](https://github.com/Winetricks/winetricks). **На нашій тестовій матриці жоден ще не перевірявся:** рецепт зі Спринту 0 не зроблено.

| Патч | Причина |
|---|---|
| `-allosarches -cef-force-32bit -cef-in-process-gpu -cef-disable-sandbox` | Wine bug 49839: `steamwebhelper.exe` падає на macOS |
| `gameoverlayrenderer` / `gameoverlayrenderer64` вимкнено | Wine bug 22053: оверлей валить ігри (64-бітну DLL ми вимикаємо самі, winetricks вимикає лише 32-бітну) |
| `libglesv2` вимкнено для `steamwebhelper.exe` | Wine bug 44985: порожні Store і Library |
| `ShowCrashDialog = 0` | на macOS падають `vulkandriverquery*`, і діалоги аварій заважають |

`-cef-force-32bit` може бути застарілим, якщо клієнт Steam уже лише 64-бітний. Це треба перевірити й прибрати новою ревізією.

## Відомі обмеження

| Тема | Статус |
|---|---|
| Вхід у Steam (логін, 2FA) | **не перевірено**. Вхід виконує користувач сам у вікні Steam, cross-app паролів не торкається. |
| Store / Library (CEF) | **не перевірено**. winetricks для цього додатково ставить `corefonts`, а в нашому каталозі verbs його ще немає. Без нього текст може не показуватись. |
| Оверлей Steam | вимкнено патчем (bug 22053). Shift+Tab-оверлей не працюватиме. |
| Steam Input / геймпади | **не перевірено**. Бекенди контролерів Wine (SDL/IOHID) налаштовуються командою `cross bottle preset <пляшка> gamepad`, див. [graphics.md](graphics.md#геймпади) |
| Steam Cloud | **не перевірено**. Синхронізація йде всередині клієнта і має працювати як звичайна мережева функція, але підтверджень немає. |
| Налаштування гри при вже запущеному Steam | не застосовуються: працюючий клієнт запускає гру зі своїм оточенням. `cross steam launch` попереджає про це. |
| `--backend d3dmetal` | D3DMetal через оверлей двигуна (`cross engine d3dmetal enable`), тоді `steam.exe` стартує з `wine` оверлею. Без оверлею використовується вбудований Direct3D Wine. На upstream-двигунах каталогу D3DMetal не працює, див. [graphics.md](graphics.md#як-підключається-d3dmetal). Інші налаштування гри: `cross bottle graphics <пляшка> --appid <appid>`. |
| Ігри з анти-чітом (EAC, BattlEye, Vanguard) | не підтримуються (not-a-goal) |
