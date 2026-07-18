//! Result types shared across commands, JSON serialization, and verdicts.

use crate::speed::SpeedResult;
use crate::ui;
use crate::usb::{LinkSpeed, UsbInfo};
use crate::verify::VerifyOutcome;

pub struct CardInput<'a> {
    pub drive_display: String,
    pub total: u64,
    pub used_before: u64,
    pub usb: Option<&'a UsbInfo>,
    pub speed: &'a SpeedResult,
    pub verify: Option<&'a VerifyOutcome>,
}

/// Cross-check measured throughput against the negotiated link.
pub fn speed_link_note(u: &UsbInfo, r: &SpeedResult) -> Option<String> {
    let best = r.seq_read_mbps.max(r.seq_write_mbps);
    let usb2_ceiling = LinkSpeed::High.effective_ceiling_mbps();
    if u.negotiated >= LinkSpeed::Super && best <= usb2_ceiling * 1.1 {
        return Some(
            "throughput is USB 2-class despite a SuperSpeed link — the flash itself is the bottleneck".into(),
        );
    }
    if u.negotiated <= LinkSpeed::High && best >= u.negotiated.effective_ceiling_mbps() * 0.8 {
        return Some(format!(
            "sequential speed is capped by the {} link — the flash may be capable of more",
            u.negotiated.label()
        ));
    }
    None
}

/// (tier, summary) where tier is PASS/WARN/FAIL.
pub fn verdict(c: &CardInput) -> (&'static str, String) {
    if let Some(v) = c.verify {
        if !v.is_ok() {
            let est = v
                .first_error_offset
                .map(|off| format!(" — estimated real capacity ~{}", human_bytes(c.used_before + off)))
                .unwrap_or_default();
            return ("FAIL", format!("drive is misreporting its capacity{est}"));
        }
    }
    if let Some(u) = c.usb {
        if u.link_downgraded() {
            return (
                "WARN",
                "capacity checks passed, but the USB 3 claim is not met by the negotiated link".into(),
            );
        }
    }
    let cap = match c.verify {
        Some(v) if v.mode == "full" => "capacity verified",
        Some(_) => "capacity spot-check passed",
        None => "capacity not tested",
    };
    let link = match c.usb {
        Some(_) => "link as advertised",
        None => "USB link not identified",
    };
    ("PASS", format!("{cap} · {link} · speeds recorded"))
}

pub fn print_card(c: &CardInput) {
    ui::header(&format!("Report card — {}", c.drive_display));
    if let Some(u) = c.usb {
        let name = u.product.clone().unwrap_or_else(|| "(unknown device)".into());
        ui::kv("device", &format!("{name} ({:04X}:{:04X})", u.vid, u.pid));
        ui::kv(
            "usb link",
            &format!(
                "claims USB {} — negotiated {}",
                u.claimed_version(),
                u.negotiated.label()
            ),
        );
    } else {
        ui::kv("device", "(not identified as USB)");
    }
    let cap = match c.verify {
        Some(v) if v.is_ok() => format!(
            "claimed {} — {} verify passed ({} spanned)",
            human_bytes(c.total),
            v.mode,
            human_bytes(v.bytes_spanned)
        ),
        Some(v) => format!(
            "claimed {} — {} verify FAILED ({} corrupt byte(s))",
            human_bytes(c.total),
            v.mode,
            v.corrupt_bytes
        ),
        None => format!("claimed {} — not verified", human_bytes(c.total)),
    };
    ui::kv("capacity", &cap);
    ui::kv(
        "sequential",
        &format!(
            "write {:.1} MB/s · read {:.1} MB/s",
            c.speed.seq_write_mbps, c.speed.seq_read_mbps
        ),
    );
    ui::kv(
        "random 4K",
        &format!(
            "write {:.0} IOPS · read {:.0} IOPS",
            c.speed.rnd_write_iops, c.speed.rnd_read_iops
        ),
    );
    println!();
    let (tier, summary) = verdict(c);
    match tier {
        "PASS" => ui::ok(&format!("PASS — {summary}")),
        "WARN" => ui::warn(&format!("WARN — {summary}")),
        _ => ui::error(&format!("FAIL — {summary}")),
    }
    if let Some(u) = c.usb {
        if let Some(note) = speed_link_note(u, c.speed) {
            println!("  note: {note}");
        }
    }
}

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

pub fn print_usb(u: &crate::usb::UsbInfo) {
    let device = match (&u.manufacturer, &u.product) {
        (Some(m), Some(p)) => format!("{m} {p}"),
        (None, Some(p)) => p.clone(),
        _ => "(no product string)".to_string(),
    };
    ui::kv("USB device", &device);
    let mut id = format!("{:04X}:{:04X}", u.vid, u.pid);
    if let Some(s) = &u.serial {
        id.push_str(&format!("  s/n {s}"));
    }
    ui::kv("VID:PID", &id);
    ui::kv("claims", &format!("USB {}", u.claimed_version()));
    ui::kv("negotiated link", u.negotiated.label());
    ui::kv("hub port", &u.port.to_string());
    println!();
    if u.link_downgraded() {
        ui::warn(
            "device claims USB 3 but the link negotiated USB 2 speeds — the drive, cable, or port is not delivering (try a blue/SS port)",
        );
    } else if u.claims_usb3() {
        ui::ok("link speed matches the USB 3 claim");
    } else {
        ui::ok(&format!(
            "USB {} device running at its expected link speed",
            u.claimed_version()
        ));
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
