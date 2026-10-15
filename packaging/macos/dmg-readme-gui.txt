cross-app @VERSION@ (Apple Silicon, macOS 14+)

Run Windows games and apps on Apple Silicon Macs through Wine.

Install
-------
Drag @APP_NAME@ onto the Applications folder in this window, then open it.

Optional: the `cross` command line tool
---------------------------------------
The app contains the `cross` CLI. After copying the app to /Applications, run
in Terminal:

    "/Volumes/cross-app @VERSION@/link-cli.sh"

This links ~/.local/bin/cross to the CLI inside the app (no administrator
password; the link keeps working when the app updates itself). Add
--system to link into /usr/local/bin instead (needs an administrator password).

Bottles, engines and settings live in ~/Library/Application Support/cross-app/.

Wine is LGPL-licensed and downloaded separately on first use; Apple's Game
Porting Toolkit (D3DMetal) is never included and must be imported by you.
