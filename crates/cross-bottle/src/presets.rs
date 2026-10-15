//! Registry presets: DPI scaling, Windows version and virtual desktop.
//!
//! A DXVA preset is not offered: upstream Wine 11.18's `dxva2` implements no
//! decoder at all (every `IDirectXVideoDecoderService` method in
//! `dlls/dxva2/main.c` returns `E_NOTIMPL`) and reads no registry switch, so
//! there is nothing to turn on. See `docs/graphics.md`.

use cross_core::model::WindowsVersion;

use crate::registry::RegType;
use crate::runtime::Runtime;
use crate::{Error, Result};

const WINE_KEY: &str = r"HKCU\Software\Wine";
const EXPLORER_KEY: &str = r"HKCU\Software\Wine\Explorer";
const DESKTOPS_KEY: &str = r"HKCU\Software\Wine\Explorer\Desktops";
const DESKTOP_KEY: &str = r"HKCU\Control Panel\Desktop";
/// Where winecfg stores the screen DPI.
const FONTS_KEY: &str = r"HKLM\System\CurrentControlSet\Hardware Profiles\Current\Software\Fonts";
/// Name of the (only) virtual desktop we manage.
const DESKTOP_NAME: &str = "Default";

pub const MIN_DPI: u32 = 96;
pub const MAX_DPI: u32 = 480;

impl Runtime {
    pub fn set_windows_version(&self, version: WindowsVersion) -> Result<()> {
        self.reg_set(WINE_KEY, "Version", RegType::Sz, version.wine_name())
    }

    /// Set the screen DPI (96 = 100 %, 144 = 150 %, 192 = 200 %).
    pub fn set_dpi(&self, dpi: u32) -> Result<()> {
        if !(MIN_DPI..=MAX_DPI).contains(&dpi) {
            return Err(Error::InvalidInput(format!(
                "DPI must be between {MIN_DPI} and {MAX_DPI}, got {dpi}"
            )));
        }
        let value = dpi.to_string();
        self.reg_set(FONTS_KEY, "LogPixels", RegType::Dword, &value)?;
        self.reg_set(DESKTOP_KEY, "LogPixels", RegType::Dword, &value)
    }

    /// Run every program in one fixed-size window, e.g. `1920x1080`.
    pub fn enable_virtual_desktop(&self, width: u32, height: u32) -> Result<()> {
        if !(320..=16384).contains(&width) || !(200..=16384).contains(&height) {
            return Err(Error::InvalidInput(format!(
                "virtual desktop size {width}x{height} is out of range"
            )));
        }
        self.reg_set(EXPLORER_KEY, "Desktop", RegType::Sz, DESKTOP_NAME)?;
        self.reg_set(
            DESKTOPS_KEY,
            DESKTOP_NAME,
            RegType::Sz,
            &format!("{width}x{height}"),
        )
    }

    pub fn disable_virtual_desktop(&self) -> Result<()> {
        self.reg_delete(EXPLORER_KEY, "Desktop")
    }
}

/// Parse `1920x1080` (also `1920X1080`).
pub fn parse_resolution(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.split_once(['x', 'X'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use cross_core::paths::AppDirs;

    use super::*;
    use crate::runtime::test_support::fake_wine;

    fn runtime() -> (
        tempfile::TempDir,
        crate::runtime::test_support::FakeWine,
        Runtime,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_wine(tmp.path());
        let rt = Runtime::new(&AppDirs::with_root(tmp.path()), "demo", &fake.wine);
        (tmp, fake, rt)
    }

    #[test]
    fn parses_resolutions() {
        assert_eq!(parse_resolution("1920x1080"), Some((1920, 1080)));
        assert_eq!(parse_resolution("800X600"), Some((800, 600)));
        assert_eq!(parse_resolution("1920"), None);
        assert_eq!(parse_resolution("axb"), None);
    }

    #[test]
    fn windows_version_is_written_to_the_registry() {
        let (_tmp, _fake, rt) = runtime();
        rt.set_windows_version(WindowsVersion::Win7).unwrap();
        let v = rt.reg_get(WINE_KEY, "Version").unwrap().unwrap();
        assert_eq!(v.data, "win7");
    }

    #[test]
    fn dpi_is_validated_and_stored_in_both_places() {
        let (_tmp, _fake, rt) = runtime();
        assert!(rt.set_dpi(50).is_err());
        assert!(rt.set_dpi(9999).is_err());
        rt.set_dpi(144).unwrap();
        assert_eq!(
            rt.reg_get(FONTS_KEY, "LogPixels").unwrap().unwrap().data,
            "144"
        );
        assert_eq!(
            rt.reg_get(DESKTOP_KEY, "LogPixels").unwrap().unwrap().data,
            "144"
        );
    }

    #[test]
    fn virtual_desktop_on_and_off() {
        let (_tmp, _fake, rt) = runtime();
        assert!(rt.enable_virtual_desktop(10, 10).is_err());
        rt.enable_virtual_desktop(1920, 1080).unwrap();
        assert_eq!(
            rt.reg_get(EXPLORER_KEY, "Desktop").unwrap().unwrap().data,
            "Default"
        );
        assert_eq!(
            rt.reg_get(DESKTOPS_KEY, "Default").unwrap().unwrap().data,
            "1920x1080"
        );
        rt.disable_virtual_desktop().unwrap();
        assert_eq!(rt.reg_get(EXPLORER_KEY, "Desktop").unwrap(), None);
    }
}
