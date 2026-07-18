//! Result types shared across commands, JSON serialization, and verdicts.

use crate::speed::SpeedResult;
use crate::ui;

pub fn print_speed(r: &SpeedResult) {
    ui::kv("sequential write", &format!("{:.1} MB/s", r.seq_write_mbps));
    ui::kv("sequential read", &format!("{:.1} MB/s", r.seq_read_mbps));
    ui::kv(
        "random 4K read",
        &format!("{:.0} IOPS ({:.1} MB/s)", r.rnd_read_iops, r.rnd_read_mbps),
    );
    ui::kv(
        "random 4K write",
        &format!("{:.0} IOPS ({:.1} MB/s)", r.rnd_write_iops, r.rnd_write_mbps),
    );
}
