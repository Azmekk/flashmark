//! Capacity verification: quick marker probe and full H2testw-style fill.
//!
//! Fake flash drives report a large capacity but silently wrap or discard
//! writes beyond the real storage. Both modes write deterministic pattern data
//! (regenerable from the byte offset alone) into numbered files on the
//! filesystem, then read it back:
//!
//! - **full**: writes every free byte, then verifies every byte. Slow but
//!   conclusive; the first corrupt offset estimates the real capacity.
//! - **quick**: preallocates the same span of files without writing their
//!   bodies, then writes and verifies 4K markers at the start, middle, and end
//!   of each file. Address-wrapping fakes corrupt earlier markers when later
//!   ones land on the same physical storage. Minutes instead of hours, but a
//!   drive with a large cache can in principle fool it — `--full` is the
//!   authoritative answer.

use crate::pattern;
use crate::speed::{AlignedBuf, open_unbuffered};
use crate::{cleanup, ui};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::time::Instant;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::FSCTL_SET_SPARSE;

const CHUNK: u64 = 1 << 30; // 1 GiB per file, safely under the FAT32 4 GiB cap
const BLOCK: usize = 1 << 20;
const MARKER: usize = 4096;
/// Free space to leave untouched so the filesystem keeps breathing room.
const MARGIN: u64 = 64 << 20;

#[derive(Serialize)]
pub struct VerifyOutcome {
    pub mode: &'static str,
    pub bytes_spanned: u64,
    pub files: usize,
    pub write_mbps: Option<f64>,
    pub read_mbps: Option<f64>,
    pub corrupt_bytes: u64,
    /// Offset (within the tested stream) of the first corrupt byte, if any.
    pub first_error_offset: Option<u64>,
}

impl VerifyOutcome {
    pub fn is_ok(&self) -> bool {
        self.corrupt_bytes == 0
    }
}

fn file_name(index: usize) -> String {
    format!("flashmark_{index:04}.fmk")
}

/// Split `total` bytes into per-file sizes of at most CHUNK, 1 MiB granular.
fn plan_files(total: u64) -> Vec<u64> {
    let mut sizes = Vec::new();
    let mut remaining = total - (total % BLOCK as u64);
    while remaining >= CHUNK {
        sizes.push(CHUNK);
        remaining -= CHUNK;
    }
    if remaining >= (16 << 20) {
        sizes.push(remaining);
    }
    sizes
}

fn budget(free: u64, limit_gb: Option<u64>) -> io::Result<u64> {
    let usable = free.saturating_sub(MARGIN);
    let capped = match limit_gb {
        Some(gb) => usable.min(gb << 30),
        None => usable,
    };
    if capped < (16 << 20) {
        return Err(io::Error::other(
            "less than 16 MiB of testable space — nothing to verify",
        ));
    }
    Ok(capped)
}

fn bytes_bar(prefix: &'static str, total: u64) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template(
            "  {prefix:16} [{bar:32.cyan/238}] {decimal_bytes:>9}/{decimal_total_bytes:9} {decimal_bytes_per_sec:>12} eta {eta}",
        )
        .expect("template")
        .progress_chars("━╸─"),
    );
    bar.set_prefix(prefix);
    bar
}

/// Count differing bytes in `got` vs the pattern at `offset`; returns
/// (corrupt bytes, first corrupt index).
fn diff_block(got: &[u8], offset: u64) -> (u64, Option<usize>) {
    if pattern::first_mismatch(got, offset).is_none() {
        return (0, None);
    }
    let mut expected = vec![0u8; got.len()];
    pattern::fill(&mut expected, offset);
    let mut corrupt = 0u64;
    let mut first = None;
    for (i, (a, b)) in got.iter().zip(expected.iter()).enumerate() {
        if a != b {
            corrupt += 1;
            if first.is_none() {
                first = Some(i);
            }
        }
    }
    (corrupt, first)
}

/// Write pattern data across `total` bytes of free space, then read it all
/// back and verify.
pub fn full(root: &Path, free: u64, limit_gb: Option<u64>, keep: bool) -> io::Result<VerifyOutcome> {
    let total: u64 = budget(free, limit_gb)?;
    let sizes = plan_files(total);
    let spanned: u64 = sizes.iter().sum();
    let paths: Vec<PathBuf> = (0..sizes.len()).map(|i| root.join(file_name(i))).collect();
    for p in &paths {
        cleanup::register(p.clone());
    }

    let result = (|| {
        let mut buf = AlignedBuf::new(BLOCK);

        let bar = bytes_bar("writing", spanned);
        let start = Instant::now();
        let mut done = 0u64;
        for (i, (path, &size)) in paths.iter().zip(&sizes).enumerate() {
            let mut file = open_unbuffered(path, true)?;
            let base = i as u64 * CHUNK;
            let mut written = 0u64;
            while written < size {
                pattern::fill(buf.as_mut_slice(), base + written);
                file.write_all(buf.as_slice())?;
                written += BLOCK as u64;
                done += BLOCK as u64;
                bar.set_position(done);
            }
        }
        let write_secs = start.elapsed().as_secs_f64();
        bar.finish_and_clear();

        let bar = bytes_bar("verifying", spanned);
        let start = Instant::now();
        let mut done = 0u64;
        let mut corrupt_bytes = 0u64;
        let mut first_error: Option<u64> = None;
        for (i, (path, &size)) in paths.iter().zip(&sizes).enumerate() {
            let mut file = open_unbuffered(path, false)?;
            let base = i as u64 * CHUNK;
            let mut read = 0u64;
            while read < size {
                file.read_exact(buf.as_mut_slice())?;
                let (bad, first) = diff_block(buf.as_slice(), base + read);
                corrupt_bytes += bad;
                if first_error.is_none() {
                    first_error = first.map(|f| base + read + f as u64);
                }
                read += BLOCK as u64;
                done += BLOCK as u64;
                bar.set_position(done);
            }
        }
        let read_secs = start.elapsed().as_secs_f64();
        bar.finish_and_clear();

        Ok(VerifyOutcome {
            mode: "full",
            bytes_spanned: spanned,
            files: sizes.len(),
            write_mbps: Some(spanned as f64 / write_secs / 1e6),
            read_mbps: Some(spanned as f64 / read_secs / 1e6),
            corrupt_bytes,
            first_error_offset: first_error,
        })
    })();

    if !keep || result.is_err() {
        remove_files(&paths);
    } else {
        for p in &paths {
            cleanup::unregister(p);
        }
    }
    result
}

/// Marker offsets within a file of `size` bytes: start, middle, end.
fn marker_offsets(size: u64) -> Vec<u64> {
    let mut offs = vec![0u64];
    let mid = (size / 2) & !(MARKER as u64 - 1);
    if mid > 0 && mid < size - MARKER as u64 {
        offs.push(mid);
    }
    offs.push(size - MARKER as u64);
    offs
}

/// Ask NTFS to treat the file as sparse so marker writes at high offsets do
/// not trigger zero-filling up to the valid-data length. Harmless no-op on
/// filesystems without sparse support (FAT32/exFAT).
fn try_set_sparse(file: &File) {
    let handle = HANDLE(file.as_raw_handle());
    let mut returned = 0u32;
    unsafe {
        let _ = DeviceIoControl(
            handle,
            FSCTL_SET_SPARSE,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        );
    }
}

/// Preallocate files across free space and verify spaced 4K markers.
pub fn quick(
    root: &Path,
    free: u64,
    limit_gb: Option<u64>,
    keep: bool,
) -> io::Result<VerifyOutcome> {
    let total: u64 = budget(free, limit_gb)?;
    let sizes = plan_files(total);
    let spanned: u64 = sizes.iter().sum();
    let paths: Vec<PathBuf> = (0..sizes.len()).map(|i| root.join(file_name(i))).collect();
    for p in &paths {
        cleanup::register(p.clone());
    }

    let result = (|| {
        let mut buf = AlignedBuf::new(MARKER);

        let bar = ProgressBar::new(sizes.len() as u64 * 2);
        bar.set_style(
            ProgressStyle::with_template("  {prefix:16} [{bar:32.cyan/238}] {pos}/{len} files")
                .expect("template")
                .progress_chars("━╸─"),
        );
        bar.set_prefix("writing markers");

        for (i, (path, &size)) in paths.iter().zip(&sizes).enumerate() {
            let mut file = open_unbuffered(path, true)?;
            try_set_sparse(&file);
            file.set_len(size)?;
            let base = i as u64 * CHUNK;
            for off in marker_offsets(size) {
                pattern::fill(buf.as_mut_slice(), base + off);
                file.seek(SeekFrom::Start(off))?;
                file.write_all(buf.as_slice())?;
            }
            bar.inc(1);
        }

        bar.set_prefix("verifying markers");
        let mut corrupt_bytes = 0u64;
        let mut first_error: Option<u64> = None;
        for (i, (path, &size)) in paths.iter().zip(&sizes).enumerate() {
            let mut file = open_unbuffered(path, false)?;
            let base = i as u64 * CHUNK;
            for off in marker_offsets(size) {
                file.seek(SeekFrom::Start(off))?;
                file.read_exact(buf.as_mut_slice())?;
                let (bad, first) = diff_block(buf.as_slice(), base + off);
                corrupt_bytes += bad;
                if first_error.is_none() {
                    first_error = first.map(|f| base + off + f as u64);
                }
            }
            bar.inc(1);
        }
        bar.finish_and_clear();

        Ok(VerifyOutcome {
            mode: "quick",
            bytes_spanned: spanned,
            files: sizes.len(),
            write_mbps: None,
            read_mbps: None,
            corrupt_bytes,
            first_error_offset: first_error,
        })
    })();

    if !keep || result.is_err() {
        remove_files(&paths);
    } else {
        for p in &paths {
            cleanup::unregister(p);
        }
    }
    result
}

fn remove_files(paths: &[PathBuf]) {
    for p in paths {
        let _ = std::fs::remove_file(p);
        cleanup::unregister(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_block_counts_and_locates_corruption() {
        let mut buf = vec![0u8; 4096];
        pattern::fill(&mut buf, 1 << 30);
        assert_eq!(diff_block(&buf, 1 << 30), (0, None));
        buf[100] ^= 0xAA;
        buf[200] ^= 0xAA;
        let (bad, first) = diff_block(&buf, 1 << 30);
        assert_eq!(bad, 2);
        assert_eq!(first, Some(100));
    }

    #[test]
    fn plan_files_respects_chunk_and_granularity() {
        assert_eq!(plan_files(CHUNK * 2 + (32 << 20)), vec![CHUNK, CHUNK, 32 << 20]);
        // Sub-16MiB remainder is dropped rather than creating a tiny file.
        assert_eq!(plan_files(CHUNK + (8 << 20)), vec![CHUNK]);
    }
}

/// Delete leftover flashmark files in `root`. Returns how many were removed.
pub fn clean(root: &Path) -> io::Result<usize> {
    let mut removed = 0;
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let is_ours = name == "flashmark.tmp"
            || (name.starts_with("flashmark_") && name.ends_with(".fmk"));
        if is_ours {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(e) => ui::warn(&format!("could not remove {name}: {e}")),
            }
        }
    }
    Ok(removed)
}
