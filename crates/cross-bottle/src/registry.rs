//! Reading and writing the bottle's registry through Wine's `reg.exe`.

use std::time::Duration;

use crate::exec::{run_captured, run_checked};
use crate::runtime::Runtime;
use crate::{Error, Result};

const REG_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegType {
    Sz,
    ExpandSz,
    MultiSz,
    Dword,
    Qword,
    Binary,
}

impl RegType {
    pub fn as_str(self) -> &'static str {
        match self {
            RegType::Sz => "REG_SZ",
            RegType::ExpandSz => "REG_EXPAND_SZ",
            RegType::MultiSz => "REG_MULTI_SZ",
            RegType::Dword => "REG_DWORD",
            RegType::Qword => "REG_QWORD",
            RegType::Binary => "REG_BINARY",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text.to_ascii_uppercase().as_str() {
            "REG_SZ" | "SZ" | "STRING" => RegType::Sz,
            "REG_EXPAND_SZ" | "EXPAND_SZ" => RegType::ExpandSz,
            "REG_MULTI_SZ" | "MULTI_SZ" => RegType::MultiSz,
            "REG_DWORD" | "DWORD" => RegType::Dword,
            "REG_QWORD" | "QWORD" => RegType::Qword,
            "REG_BINARY" | "BINARY" => RegType::Binary,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegValue {
    pub name: String,
    /// Type exactly as printed by reg.exe, e.g. `REG_DWORD`.
    pub kind: String,
    /// Data as printed by reg.exe (DWORDs are hex, e.g. `0x78`).
    pub data: String,
}

impl RegValue {
    /// Numeric value of a DWORD/QWORD.
    pub fn as_u64(&self) -> Option<u64> {
        let hex = self.data.strip_prefix("0x")?;
        u64::from_str_radix(hex, 16).ok()
    }
}

/// Parse `reg query KEY /v NAME` output. Wine prints CRLF line endings and
/// value lines as `    NAME    TYPE    DATA`.
pub fn parse_query(output: &str, name: &str) -> Option<RegValue> {
    output.lines().find_map(|line| {
        let line = line.trim_end_matches('\r');
        if !line.starts_with(' ') {
            return None;
        }
        let mut parts = line.trim_start().splitn(3, "    ");
        let found = parts.next()?;
        let kind = parts.next()?;
        let data = parts.next().unwrap_or("");
        found.eq_ignore_ascii_case(name).then(|| RegValue {
            name: found.to_owned(),
            kind: kind.to_owned(),
            data: data.to_owned(),
        })
    })
}

impl Runtime {
    /// Read one registry value; `None` when the key or value does not exist.
    pub fn reg_get(&self, key: &str, name: &str) -> Result<Option<RegValue>> {
        let mut cmd = self.wine_command();
        cmd.args(["reg", "query", key, "/v", name]);
        let out = run_captured(cmd, "reg query", REG_TIMEOUT)?;
        if let Some(value) = parse_query(&out.stdout, name) {
            return Ok(Some(value));
        }
        if out.status.success() || out.reports_not_found() {
            return Ok(None);
        }
        Err(Error::Wine {
            what: "reg query".into(),
            message: out.message(),
        })
    }

    pub fn reg_set(&self, key: &str, name: &str, kind: RegType, data: &str) -> Result<()> {
        let mut cmd = self.wine_command();
        cmd.args([
            "reg",
            "add",
            key,
            "/v",
            name,
            "/t",
            kind.as_str(),
            "/d",
            data,
            "/f",
        ]);
        run_checked(cmd, "reg add", REG_TIMEOUT)?;
        Ok(())
    }

    /// Delete one value. Deleting something that is not there is not an error.
    pub fn reg_delete(&self, key: &str, name: &str) -> Result<()> {
        let mut cmd = self.wine_command();
        cmd.args(["reg", "delete", key, "/v", name, "/f"]);
        let out = run_captured(cmd, "reg delete", REG_TIMEOUT)?;
        if out.status.success() || out.reports_not_found() {
            Ok(())
        } else {
            Err(Error::Wine {
                what: "reg delete".into(),
                message: out.message(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use cross_core::paths::AppDirs;

    use super::*;
    use crate::runtime::test_support::fake_wine;

    #[test]
    fn parses_wine_query_output() {
        let out = "\r\nHKEY_CURRENT_USER\\Software\\Wine\r\n    Version    REG_SZ    win10\r\n\r\n";
        let v = parse_query(out, "Version").unwrap();
        assert_eq!((v.kind.as_str(), v.data.as_str()), ("REG_SZ", "win10"));

        let dword = "\r\nHKEY_CURRENT_USER\\Software\\X\r\n    D    REG_DWORD    0x78\r\n\r\n";
        assert_eq!(parse_query(dword, "d").unwrap().as_u64(), Some(120));

        let spaced = "K\r\n    S    REG_SZ    hello  world\r\n";
        assert_eq!(parse_query(spaced, "S").unwrap().data, "hello  world");

        assert_eq!(
            parse_query(
                "\r\nreg: Unable to find the specified registry value\r\n",
                "x"
            ),
            None
        );
        assert_eq!(parse_query(out, "Other"), None);
    }

    #[test]
    fn reg_type_round_trips() {
        for t in [
            RegType::Sz,
            RegType::Dword,
            RegType::Binary,
            RegType::MultiSz,
        ] {
            assert_eq!(RegType::parse(t.as_str()), Some(t));
        }
        assert_eq!(RegType::parse("dword"), Some(RegType::Dword));
        assert_eq!(RegType::parse("nope"), None);
    }

    #[test]
    fn set_get_delete_via_wine() {
        let tmp = tempfile::tempdir().unwrap();
        let fake = fake_wine(tmp.path());
        let rt = Runtime::new(&AppDirs::with_root(tmp.path()), "demo", &fake.wine);

        assert_eq!(rt.reg_get("HKCU\\Software\\Wine", "Version").unwrap(), None);
        rt.reg_set("HKCU\\Software\\Wine", "Version", RegType::Sz, "win10")
            .unwrap();
        let v = rt
            .reg_get("HKCU\\Software\\Wine", "Version")
            .unwrap()
            .unwrap();
        assert_eq!((v.kind.as_str(), v.data.as_str()), ("REG_SZ", "win10"));
        rt.reg_delete("HKCU\\Software\\Wine", "Version").unwrap();
        assert_eq!(rt.reg_get("HKCU\\Software\\Wine", "Version").unwrap(), None);
        rt.reg_delete("HKCU\\Software\\Wine", "Version").unwrap();
        assert!(fake
            .calls()
            .contains("ARGS:reg add HKCU\\Software\\Wine /v Version /t REG_SZ /d win10 /f"));
    }
}
