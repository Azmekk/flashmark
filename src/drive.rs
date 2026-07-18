//! Drive-letter validation, capacity queries, and safety guards.

use std::io;
use std::path::PathBuf;
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW,
};
use windows::core::PCWSTR;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DriveKind {
    Unknown,
    NoRootDir,
    Removable,
    Fixed,
    Remote,
    CdRom,
    RamDisk,
}

impl DriveKind {
    pub fn label(self) -> &'static str {
        match self {
            DriveKind::Unknown => "unknown",
            DriveKind::NoRootDir => "no such drive",
            DriveKind::Removable => "removable",
            DriveKind::Fixed => "fixed",
            DriveKind::Remote => "network",
            DriveKind::CdRom => "CD-ROM",
            DriveKind::RamDisk => "RAM disk",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Drive {
    pub letter: char,
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

impl Drive {
    pub fn parse(s: &str) -> Result<Self, String> {
        let trimmed = s.trim().trim_end_matches(['\\', '/']).trim_end_matches(':');
        let mut chars = trimmed.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => Ok(Drive {
                letter: c.to_ascii_uppercase(),
            }),
            _ => Err(format!(
                "'{s}' is not a drive letter (expected something like \"E\" or \"E:\")"
            )),
        }
    }

    pub fn root(&self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.letter))
    }

    pub fn display(&self) -> String {
        format!("{}:", self.letter)
    }

    pub fn kind(&self) -> DriveKind {
        let root = wide(&format!("{}:\\", self.letter));
        match unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) } {
            1 => DriveKind::NoRootDir,
            2 => DriveKind::Removable,
            3 => DriveKind::Fixed,
            4 => DriveKind::Remote,
            5 => DriveKind::CdRom,
            6 => DriveKind::RamDisk,
            _ => DriveKind::Unknown,
        }
    }

    /// (free bytes available, total bytes) for the volume.
    pub fn space(&self) -> io::Result<(u64, u64)> {
        let root = wide(&format!("{}:\\", self.letter));
        let mut free = 0u64;
        let mut total = 0u64;
        unsafe {
            GetDiskFreeSpaceExW(
                PCWSTR(root.as_ptr()),
                Some(&mut free),
                Some(&mut total),
                None,
            )
        }
        .map_err(|e| io::Error::other(format!("GetDiskFreeSpaceExW failed: {e}")))?;
        Ok((free, total))
    }

    /// Filesystem name ("NTFS", "exFAT", "FAT32", ...), if queryable.
    pub fn fs_name(&self) -> Option<String> {
        let root = wide(&format!("{}:\\", self.letter));
        let mut fs = [0u16; 64];
        unsafe {
            GetVolumeInformationW(
                PCWSTR(root.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut fs),
            )
        }
        .ok()?;
        let s = String::from_utf16_lossy(&fs)
            .trim_end_matches('\0')
            .to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn is_system(&self) -> bool {
        std::env::var("SystemDrive")
            .ok()
            .and_then(|d| d.chars().next())
            .map(|c| c.to_ascii_uppercase() == self.letter)
            .unwrap_or(false)
    }

    /// Gate for anything that writes to the drive.
    pub fn guard_writes(&self, allow_fixed: bool) -> Result<(), String> {
        let kind = self.kind();
        if kind == DriveKind::NoRootDir {
            return Err(format!("drive {} does not exist", self.display()));
        }
        if self.is_system() {
            return Err(format!(
                "refusing to run write tests on the system drive ({})",
                self.display()
            ));
        }
        match kind {
            DriveKind::Removable => Ok(()),
            DriveKind::Fixed | DriveKind::RamDisk if allow_fixed => Ok(()),
            DriveKind::Fixed => Err(format!(
                "{} is a fixed drive, not a removable flash drive — pass --allow-fixed to test it anyway",
                self.display()
            )),
            other => Err(format!(
                "{} is a {} drive and cannot be write-tested",
                self.display(),
                other.label()
            )),
        }
    }
}
