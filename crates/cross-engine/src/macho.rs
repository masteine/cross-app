//! Reading the CPU architectures of a Mach-O executable from its header.
//!
//! This is what tells an engine's translation apart in practice: an x86_64-only
//! `wine` needs Rosetta 2, an arm64 one does not. Only the header is read; the
//! formats are those of `<mach-o/loader.h>` and `<mach-o/fat.h>`.

use std::fs::File;
use std::io::Read;
use std::path::Path;

const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_CIGAM_64: u32 = 0xcffa_edfe;
const FAT_MAGIC: u32 = 0xcafe_babe;
const FAT_MAGIC_64: u32 = 0xcafe_babf;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
/// A fat header names at most a handful of slices; refuse absurd counts.
const MAX_FAT_ARCHS: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    /// arm64 and arm64e.
    Arm64,
    Other(u32),
}

impl Arch {
    fn from_cputype(cputype: u32) -> Self {
        match cputype {
            CPU_TYPE_X86_64 => Arch::X86_64,
            CPU_TYPE_ARM64 => Arch::Arm64,
            other => Arch::Other(other),
        }
    }

    pub fn name(self) -> String {
        match self {
            Arch::X86_64 => "x86_64".to_owned(),
            Arch::Arm64 => "arm64".to_owned(),
            Arch::Other(t) => format!("cputype {t:#x}"),
        }
    }
}

/// Architectures of a Mach-O file, or `None` when it is not a 64-bit Mach-O
/// (a shell script wrapper, for example). I/O errors are reported as `None`
/// too: callers use this as a hint, never as the only check.
pub fn architectures(path: &Path) -> Option<Vec<Arch>> {
    let mut header = [0u8; 8 + MAX_FAT_ARCHS as usize * 32];
    let mut file = File::open(path).ok()?;
    let mut len = 0;
    while len < header.len() {
        match file.read(&mut header[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(_) => return None,
        }
    }
    parse(&header[..len])
}

fn parse(bytes: &[u8]) -> Option<Vec<Arch>> {
    let be = |at: usize| -> Option<u32> {
        Some(u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    let le = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
    };
    match be(0)? {
        // Thin files are little-endian on both Apple architectures.
        MH_CIGAM_64 => Some(vec![Arch::from_cputype(le(4)?)]),
        MH_MAGIC_64 => Some(vec![Arch::from_cputype(be(4)?)]),
        magic @ (FAT_MAGIC | FAT_MAGIC_64) => {
            let count = be(4)?;
            if count == 0 || count > MAX_FAT_ARCHS {
                return None;
            }
            let entry = if magic == FAT_MAGIC { 20 } else { 32 };
            (0..count as usize)
                .map(|i| be(8 + i * entry).map(Arch::from_cputype))
                .collect()
        }
        _ => None,
    }
}

/// `true` when the file is a Mach-O that cannot run natively on Apple Silicon
/// (it has an x86_64 slice and no arm64 slice), i.e. it needs Rosetta 2.
pub fn needs_rosetta(path: &Path) -> bool {
    architectures(path)
        .is_some_and(|archs| archs.contains(&Arch::X86_64) && !archs.contains(&Arch::Arm64))
}

#[cfg(test)]
pub(crate) mod test_support {
    /// A minimal thin 64-bit Mach-O header for `cputype` (enough for [`super::architectures`]).
    pub fn thin_header(cputype: u32) -> Vec<u8> {
        let mut bytes = super::MH_MAGIC_64.to_le_bytes().to_vec();
        bytes.extend_from_slice(&cputype.to_le_bytes());
        bytes.extend_from_slice(&[0; 24]);
        bytes
    }

    pub const X86_64: u32 = super::CPU_TYPE_X86_64;
    pub const ARM64: u32 = super::CPU_TYPE_ARM64;
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    fn fat(entries: &[u32], magic: u32) -> Vec<u8> {
        let entry = if magic == FAT_MAGIC { 20 } else { 32 };
        let mut bytes = magic.to_be_bytes().to_vec();
        bytes.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for cputype in entries {
            let mut e = vec![0u8; entry];
            e[..4].copy_from_slice(&cputype.to_be_bytes());
            bytes.extend_from_slice(&e);
        }
        bytes
    }

    #[test]
    fn thin_and_fat_headers() {
        assert_eq!(parse(&thin_header(X86_64)), Some(vec![Arch::X86_64]));
        assert_eq!(parse(&thin_header(ARM64)), Some(vec![Arch::Arm64]));
        assert_eq!(
            parse(&fat(&[X86_64, ARM64], FAT_MAGIC)),
            Some(vec![Arch::X86_64, Arch::Arm64])
        );
        assert_eq!(
            parse(&fat(&[ARM64, 7], FAT_MAGIC_64)),
            Some(vec![Arch::Arm64, Arch::Other(7)])
        );
    }

    #[test]
    fn non_mach_o_is_none() {
        assert_eq!(parse(b"#!/bin/sh\necho hi\n"), None);
        assert_eq!(parse(b""), None);
        assert_eq!(parse(&fat(&[], FAT_MAGIC)), None);
        // Truncated fat header.
        assert_eq!(parse(&fat(&[X86_64, ARM64], FAT_MAGIC)[..20]), None);
    }

    #[test]
    fn rosetta_need_follows_the_slices() {
        let tmp = tempfile::tempdir().unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = tmp.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        assert!(needs_rosetta(&write("intel", &thin_header(X86_64))));
        assert!(!needs_rosetta(&write("arm", &thin_header(ARM64))));
        assert!(!needs_rosetta(&write(
            "universal",
            &fat(&[X86_64, ARM64], FAT_MAGIC)
        )));
        assert!(!needs_rosetta(&write("script", b"#!/bin/sh\n")));
        assert!(!needs_rosetta(&tmp.path().join("missing")));
    }

    #[test]
    fn system_binaries_have_an_arm64_slice() {
        // Real file from this Mac: everything in /usr/bin runs natively on Apple Silicon.
        let archs = architectures(Path::new("/usr/bin/true")).expect("Mach-O");
        assert!(archs.contains(&Arch::Arm64), "{archs:?}");
    }
}
