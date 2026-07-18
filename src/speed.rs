//! Sequential and random 4K benchmarks using unbuffered, write-through I/O.
//!
//! Every handle is opened with FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH
//! so measurements hit the device instead of the Windows cache. That imposes
//! the classic constraints: sector-aligned buffers, transfer sizes, and file
//! offsets. `AlignedBuf` provides the buffers; block sizes are 1 MiB
//! (sequential) and 4 KiB (random), both safely aligned.

use crate::{cleanup, pattern};
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::alloc::{Layout, alloc, dealloc};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH};

pub const SECTOR_ALIGN: usize = 4096;
const SEQ_BLOCK: usize = 1 << 20;
const RND_BLOCK: usize = 4096;

pub struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

impl AlignedBuf {
    pub fn new(size: usize) -> Self {
        let layout = Layout::from_size_align(size, SECTOR_ALIGN).expect("invalid layout");
        let ptr = unsafe { alloc(layout) };
        assert!(!ptr.is_null(), "aligned allocation failed");
        AlignedBuf { ptr, layout }
    }

    pub fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.layout.size()) }
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.layout.size()) }
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, self.layout) }
    }
}

pub fn open_unbuffered(path: &Path, write: bool) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.read(true);
    if write {
        opts.write(true).create(true);
    }
    opts.custom_flags(FILE_FLAG_NO_BUFFERING.0 | FILE_FLAG_WRITE_THROUGH.0);
    opts.open(path)
}

#[derive(Serialize, Clone, Copy)]
pub struct SpeedResult {
    pub file_size_bytes: u64,
    pub seq_write_mbps: f64,
    pub seq_read_mbps: f64,
    pub rnd_read_iops: f64,
    pub rnd_read_mbps: f64,
    pub rnd_write_iops: f64,
    pub rnd_write_mbps: f64,
}

/// MB/s uses decimal megabytes (10^6), matching how drives are marketed and
/// how CrystalDiskMark reports.
fn mbps(bytes: u64, secs: f64) -> f64 {
    bytes as f64 / secs / 1e6
}

fn seq_bar(name: &'static str, total: u64) -> ProgressBar {
    let bar = ProgressBar::new(total);
    bar.set_style(
        ProgressStyle::with_template(
            "  {prefix:16} [{bar:32.cyan/238}] {decimal_bytes:>9} {decimal_bytes_per_sec:>12}",
        )
        .expect("template")
        .progress_chars("━╸─"),
    );
    bar.set_prefix(name);
    bar
}

fn rnd_bar(name: &'static str, total_ms: u64) -> ProgressBar {
    let bar = ProgressBar::new(total_ms);
    bar.set_style(
        ProgressStyle::with_template("  {prefix:16} [{bar:32.magenta/238}] {msg:>22}")
            .expect("template")
            .progress_chars("━╸─"),
    );
    bar.set_prefix(name);
    bar
}

fn seq_write(path: &Path, file_size: u64) -> io::Result<f64> {
    let mut buf = AlignedBuf::new(SEQ_BLOCK);
    let mut file = open_unbuffered(path, true)?;
    let bar = seq_bar("sequential write", file_size);
    let start = Instant::now();
    let mut written = 0u64;
    while written < file_size {
        pattern::fill(buf.as_mut_slice(), written);
        file.write_all(buf.as_slice())?;
        written += SEQ_BLOCK as u64;
        bar.set_position(written);
    }
    let secs = start.elapsed().as_secs_f64();
    bar.finish_and_clear();
    Ok(mbps(written, secs))
}

fn seq_read(path: &Path, file_size: u64) -> io::Result<f64> {
    let mut buf = AlignedBuf::new(SEQ_BLOCK);
    let mut file = open_unbuffered(path, false)?;
    let bar = seq_bar("sequential read", file_size);
    let start = Instant::now();
    let mut read = 0u64;
    while read < file_size {
        file.read_exact(buf.as_mut_slice())?;
        read += SEQ_BLOCK as u64;
        bar.set_position(read);
    }
    let secs = start.elapsed().as_secs_f64();
    bar.finish_and_clear();
    Ok(mbps(read, secs))
}

fn rnd_phase(
    name: &'static str,
    path: &Path,
    file_size: u64,
    duration: Duration,
    write: bool,
) -> io::Result<(f64, f64)> {
    let mut buf = AlignedBuf::new(RND_BLOCK);
    if write {
        pattern::fill(buf.as_mut_slice(), 0);
    }
    let mut file = open_unbuffered(path, write)?;
    let mut rng = pattern::Rng::from_time();
    let blocks = file_size / RND_BLOCK as u64;
    let bar = rnd_bar(name, duration.as_millis() as u64);
    let start = Instant::now();
    let mut ops = 0u64;
    loop {
        let elapsed = start.elapsed();
        if elapsed >= duration {
            break;
        }
        let offset = (rng.next() % blocks) * RND_BLOCK as u64;
        file.seek(SeekFrom::Start(offset))?;
        if write {
            file.write_all(buf.as_slice())?;
        } else {
            file.read_exact(buf.as_mut_slice())?;
        }
        ops += 1;
        if ops % 64 == 0 {
            let secs = elapsed.as_secs_f64();
            bar.set_position(elapsed.as_millis() as u64);
            bar.set_message(format!("{:.0} IOPS", ops as f64 / secs));
        }
    }
    let secs = start.elapsed().as_secs_f64();
    bar.finish_and_clear();
    let iops = ops as f64 / secs;
    Ok((iops, mbps(ops * RND_BLOCK as u64, secs)))
}

/// Run the full benchmark using a temporary file inside `dir`.
/// `file_size` must be a multiple of 1 MiB.
pub fn run(dir: &Path, file_size: u64, rnd_duration: Duration) -> io::Result<SpeedResult> {
    assert!(file_size > 0 && file_size % SEQ_BLOCK as u64 == 0);
    let path = dir.join("flashmark.tmp");
    cleanup::register(path.clone());

    let result = (|| {
        let seq_write_mbps = seq_write(&path, file_size)?;
        let seq_read_mbps = seq_read(&path, file_size)?;
        let (rnd_read_iops, rnd_read_mbps) =
            rnd_phase("random 4K read", &path, file_size, rnd_duration, false)?;
        let (rnd_write_iops, rnd_write_mbps) =
            rnd_phase("random 4K write", &path, file_size, rnd_duration, true)?;
        Ok(SpeedResult {
            file_size_bytes: file_size,
            seq_write_mbps,
            seq_read_mbps,
            rnd_read_iops,
            rnd_read_mbps,
            rnd_write_iops,
            rnd_write_mbps,
        })
    })();

    let _ = std::fs::remove_file(&path);
    cleanup::unregister(&path);
    result
}
