//! Result types shared across commands, JSON serialization, and verdicts.

use crate::speed::SpeedResult;
use crate::ui;
use crate::verify::VerifyOutcome;

/// Decimal units (GB = 10^9), matching how drive capacity is marketed.
pub fn human_bytes(b: u64) -> String {
    let b = b as f64;
    if b >= 1e12 {
        format!("{:.2} TB", b / 1e12)
    } else if b >= 1e9 {
        format!("{:.2} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else {
        format!("{:.0} KB", b / 1e3)
    }
}

pub fn print_verify(o: &VerifyOutcome, used_before: u64, total: u64) {
    ui::kv(
        "space tested",
        &format!("{} in {} file(s)", human_bytes(o.bytes_spanned), o.files),
    );
    if let (Some(w), Some(r)) = (o.write_mbps, o.read_mbps) {
        ui::kv("write speed", &format!("{w:.1} MB/s"));
        ui::kv("read speed", &format!("{r:.1} MB/s"));
    }
    println!();
    if o.is_ok() {
        ui::ok("no corruption found — capacity claim holds for the tested span");
        if o.mode == "quick" {
            println!("  (quick probe: run with --full for a conclusive, every-byte check)");
        }
    } else {
        ui::error(&format!(
            "{} corrupt byte(s) — this drive is lying about its capacity",
            o.corrupt_bytes
        ));
        if let Some(off) = o.first_error_offset {
            ui::kv(
                "first error at",
                &format!("{} into the tested span", human_bytes(off)),
            );
            ui::kv(
                "estimated real capacity",
                &format!(
                    "~{} (claimed: {})",
                    human_bytes(used_before + off),
                    human_bytes(total)
                ),
            );
        }
    }
}

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
