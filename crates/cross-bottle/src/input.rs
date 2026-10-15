//! Game controllers.
//!
//! Wine does not use Apple's Game Controller framework. On macOS its `winebus`
//! driver finds controllers through two back ends (Wine 11.18,
//! `dlls/winebus.sys/`): SDL2 (`bus_sdl.c`, which `dlopen`s the engine's
//! bundled `libSDL2`) and IOHID (`bus_iohid.c`, raw HID devices that announce
//! themselves as joystick or gamepad). Both are switched by DWORD values under
//! `HKLM\System\CurrentControlSet\Services\WineBus`, read once when the driver
//! starts (`bus_options_init()` in `main.c`):
//!
//! | value | default | meaning |
//! |---|---|---|
//! | `Enable SDL` | 1 | use SDL for controllers |
//! | `DisableHidraw` | 0 | 1 = ignore IOHID devices |
//! | `Map Controllers` | 1 | present SDL game controllers as XInput-style HID gamepads |
//!
//! SDL itself also reads hints from the environment (`SDL_hints.h` in SDL2):
//! `SDL_JOYSTICK_HIDAPI`, `SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS`,
//! `SDL_GAMECONTROLLER_USE_BUTTON_LABELS`. They can be set with `--env`, but
//! `winebus` runs inside the bottle's `winedevice.exe`, which inherits the
//! environment of whatever started the bottle's Wine session.

use crate::registry::RegType;
use crate::runtime::Runtime;
use crate::Result;

const WINEBUS_KEY: &str = r"HKLM\System\CurrentControlSet\Services\WineBus";

/// Changes to the controller back ends; `None` leaves a value alone.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GamepadOptions {
    pub sdl: Option<bool>,
    pub hidraw: Option<bool>,
    pub map_controllers: Option<bool>,
}

/// Effective controller settings of a bottle (Wine's defaults filled in).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct GamepadStatus {
    pub sdl: bool,
    pub hidraw: bool,
    pub map_controllers: bool,
}

impl Runtime {
    pub fn gamepad_status(&self) -> Result<GamepadStatus> {
        let dword = |name: &str, default: u64| -> Result<u64> {
            Ok(self
                .reg_get(WINEBUS_KEY, name)?
                .and_then(|v| v.as_u64())
                .unwrap_or(default))
        };
        Ok(GamepadStatus {
            sdl: dword("Enable SDL", 1)? != 0,
            hidraw: dword("DisableHidraw", 0)? == 0,
            map_controllers: dword("Map Controllers", 1)? != 0,
        })
    }

    /// Write the given options. They take effect the next time the bottle
    /// starts (stop it with `cross bottle kill`).
    pub fn set_gamepad_options(&self, options: &GamepadOptions) -> Result<()> {
        let flag = |on: bool| if on { "1" } else { "0" };
        if let Some(on) = options.sdl {
            self.reg_set(WINEBUS_KEY, "Enable SDL", RegType::Dword, flag(on))?;
        }
        if let Some(on) = options.hidraw {
            self.reg_set(WINEBUS_KEY, "DisableHidraw", RegType::Dword, flag(!on))?;
        }
        if let Some(on) = options.map_controllers {
            self.reg_set(WINEBUS_KEY, "Map Controllers", RegType::Dword, flag(on))?;
        }
        tracing::info!(bottle = self.bottle_id(), ?options, "gamepad options set");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cross_core::paths::AppDirs;

    use super::*;
    use crate::runtime::test_support::fake_wine;

    #[test]
    fn defaults_and_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_wine(tmp.path());
        let rt = Runtime::new(&AppDirs::with_root(tmp.path()), "demo", &fake.wine);
        assert_eq!(
            rt.gamepad_status().unwrap(),
            GamepadStatus {
                sdl: true,
                hidraw: true,
                map_controllers: true
            }
        );
        rt.set_gamepad_options(&GamepadOptions {
            hidraw: Some(false),
            map_controllers: Some(false),
            ..Default::default()
        })
        .unwrap();
        assert!(fake.calls().contains(
            r"ARGS:reg add HKLM\System\CurrentControlSet\Services\WineBus /v DisableHidraw /t REG_DWORD /d 1 /f"
        ));
        assert!(!fake
            .calls()
            .contains("reg add HKLM\\System\\CurrentControlSet\\Services\\WineBus /v Enable SDL"));

        // The fake reg.exe echoes data back verbatim; real reg.exe prints 0x1.
        let db = fake.wine.with_file_name("reg.db");
        let text = fs::read_to_string(&db)
            .unwrap()
            .replace("|1\n", "|0x1\n")
            .replace("|0\n", "|0x0\n");
        fs::write(&db, text).unwrap();
        assert_eq!(
            rt.gamepad_status().unwrap(),
            GamepadStatus {
                sdl: true,
                hidraw: false,
                map_controllers: false
            }
        );
    }
}
