cross-app @VERSION@ (Apple Silicon, macOS 14+)

Run Windows games and apps on Apple Silicon Macs through Wine.

Install the `cross` command
---------------------------
Open Terminal and run:

    /Volumes/"cross-app @VERSION@"/link-cli.sh --copy

This copies `cross` to ~/.local/bin (no administrator password needed) and
tells you how to add that folder to your PATH if it is not there yet.
For /usr/local/bin instead (needs an administrator password):

    /Volumes/"cross-app @VERSION@"/link-cli.sh --copy --system

Then check the Mac:

    cross doctor

Uninstall
---------
    ~/.local/bin/cross --version    # confirm what is installed
    "/Volumes/cross-app @VERSION@/link-cli.sh" --uninstall

Bottles, engines and settings live in ~/Library/Application Support/cross-app/
and are not removed by uninstalling the command.

Wine is LGPL-licensed and downloaded separately on first use; Apple's Game
Porting Toolkit (D3DMetal) is never included and must be imported by you.
