mod cleanup;
mod drive;
mod pattern;
mod report;
mod speed;
mod ui;
mod usb;
mod verify;

use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "flashmark",
    version,
    about = "Flash-drive testing: speed, capacity, and USB link claims"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show device identity and claimed vs negotiated USB link speed
    Info(InfoArgs),
    /// Benchmark sequential and random 4K read/write
    Speed(SpeedArgs),
    /// Verify real capacity (fake-flash detection)
    Verify(VerifyArgs),
    /// Run info + speed + quick verify and print a report card
    Test(TestArgs),
    /// Remove leftover flashmark test files
    Clean(CleanArgs),
}

#[derive(Args)]
struct DriveArg {
    /// Drive letter of the flash drive (e.g. "E" or "E:")
    drive: String,
}

#[derive(Args)]
struct WriteGuards {
    /// Allow testing a non-removable (fixed) drive
    #[arg(long)]
    allow_fixed: bool,
}

#[derive(Args)]
struct InfoArgs {
    #[command(flatten)]
    drive: DriveArg,
    /// Machine-readable JSON output
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct SpeedArgs {
    #[command(flatten)]
    drive: DriveArg,
    #[command(flatten)]
    guards: WriteGuards,
    /// Test file size in MiB
    #[arg(long, default_value_t = 256)]
    size_mb: u64,
    /// Seconds to run each random 4K phase
    #[arg(long, default_value_t = 5)]
    duration_s: u64,
    /// Override directory for the test file (default: drive root)
    #[arg(long, hide = true)]
    dir: Option<String>,
    /// Machine-readable JSON output
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct VerifyArgs {
    #[command(flatten)]
    drive: DriveArg,
    #[command(flatten)]
    guards: WriteGuards,
    /// Quick probe: preallocate across free space, verify spaced markers (minutes)
    #[arg(long, conflicts_with = "full")]
    quick: bool,
    /// Full verify: write and read back all free space (hours, thorough)
    #[arg(long)]
    full: bool,
    /// Limit the amount of space tested, in GiB
    #[arg(long)]
    limit_gb: Option<u64>,
    /// Keep test files after the run (useful for re-verification)
    #[arg(long)]
    keep: bool,
    /// Override directory for test files — must be on the named drive (dev use)
    #[arg(long, hide = true)]
    dir: Option<String>,
    /// Machine-readable JSON output
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct TestArgs {
    #[command(flatten)]
    drive: DriveArg,
    #[command(flatten)]
    guards: WriteGuards,
    /// Test file size in MiB for the speed phase
    #[arg(long, default_value_t = 256)]
    size_mb: u64,
    /// Skip the capacity verification phase
    #[arg(long)]
    skip_verify: bool,
    /// Machine-readable JSON output
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct CleanArgs {
    #[command(flatten)]
    drive: DriveArg,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    cleanup::install_ctrlc_handler();

    let result = match cli.command {
        Command::Info(args) => cmd_info(args),
        Command::Speed(args) => cmd_speed(args),
        Command::Verify(args) => cmd_verify(args),
        Command::Test(args) => cmd_test(args),
        Command::Clean(args) => cmd_clean(args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) if cleanup::aborted() => {
            ui::warn("interrupted — flashmark test files removed");
            ExitCode::from(130)
        }
        Err(e) => {
            ui::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

/// FAT32 can't allocate space without physically zero-filling it, so the
/// quick probe loses its speed advantage there.
fn warn_if_fat32(drive: &drive::Drive) {
    if drive
        .fs_name()
        .is_some_and(|f| f.eq_ignore_ascii_case("FAT32"))
    {
        ui::warn(
            "FAT32 zero-fills allocations — quick mode will run at full-write speed here; \
             prefer --full (byte-exact, same duration) or reformat the drive to exFAT",
        );
    }
}

type CmdResult = Result<(), Box<dyn std::error::Error>>;

fn cmd_info(args: InfoArgs) -> CmdResult {
    let drive = drive::Drive::parse(&args.drive.drive)?;
    if drive.kind() == drive::DriveKind::NoRootDir {
        return Err(format!("drive {} does not exist", drive.display()).into());
    }
    let (free, total) = drive.space()?;
    let usb = usb::for_drive(&drive);
    if args.json {
        let usb_value = match &usb {
            Ok(u) => serde_json::to_value(u)?,
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        };
        let doc = serde_json::json!({
            "command": "info",
            "drive": drive.display(),
            "drive_type": drive.kind().label(),
            "volume_total_bytes": total,
            "volume_free_bytes": free,
            "usb": usb_value,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        ui::header(&format!("Drive info — {}", drive.display()));
        ui::kv("drive type", drive.kind().label());
        ui::kv(
            "filesystem",
            &drive.fs_name().unwrap_or_else(|| "unknown".into()),
        );
        ui::kv("volume size", &report::human_bytes(total));
        ui::kv("free space", &report::human_bytes(free));
        match &usb {
            Ok(u) => report::print_usb(u),
            Err(e) => ui::warn(&format!("USB details unavailable: {e}")),
        }
    }
    Ok(())
}

fn cmd_speed(args: SpeedArgs) -> CmdResult {
    let drive = drive::Drive::parse(&args.drive.drive)?;
    let dir = match &args.dir {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            drive.guard_writes(args.guards.allow_fixed)?;
            drive.root()
        }
    };
    let file_size = args.size_mb.max(16) * (1 << 20);
    if args.dir.is_none() {
        let (free, _) = drive.space()?;
        if free < file_size + (64 << 20) {
            return Err(format!(
                "not enough free space on {} for a {} MiB test file",
                drive.display(),
                file_size >> 20
            )
            .into());
        }
    }
    if !args.json {
        ui::header(&format!("Speed test — {}", drive.display()));
        ui::kv("drive type", drive.kind().label());
        ui::kv("test file", &format!("{} MiB, unbuffered I/O", file_size >> 20));
        println!();
    }
    let result = speed::run(
        &dir,
        file_size,
        std::time::Duration::from_secs(args.duration_s.max(1)),
    )?;
    if args.json {
        let doc = serde_json::json!({
            "command": "speed",
            "drive": drive.display(),
            "result": result,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        report::print_speed(&result);
    }
    Ok(())
}

fn cmd_verify(args: VerifyArgs) -> CmdResult {
    let drive = drive::Drive::parse(&args.drive.drive)?;
    let root = match &args.dir {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            drive.guard_writes(args.guards.allow_fixed)?;
            drive.root()
        }
    };
    let (free, total) = drive.space()?;
    if !args.json {
        ui::header(&format!(
            "Capacity verify ({}) — {}",
            if args.full { "full" } else { "quick" },
            drive.display()
        ));
        ui::kv(
            "filesystem",
            &drive.fs_name().unwrap_or_else(|| "unknown".into()),
        );
        ui::kv("volume size", &report::human_bytes(total));
        ui::kv("free space", &report::human_bytes(free));
        if args.full {
            ui::warn("full mode writes all free space — this can take hours on large drives");
        } else {
            warn_if_fat32(&drive);
        }
        println!();
    }
    let outcome = if args.full {
        verify::full(&root, free, args.limit_gb, args.keep)?
    } else {
        verify::quick(&root, free, args.limit_gb, args.keep)?
    };
    let used_before = total - free;
    if args.json {
        let doc = serde_json::json!({
            "command": "verify",
            "drive": drive.display(),
            "volume_total_bytes": total,
            "result": outcome,
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        report::print_verify(&outcome, used_before, total);
    }
    if !outcome.is_ok() {
        return Err("capacity verification FAILED — the drive is misreporting its capacity".into());
    }
    Ok(())
}

fn cmd_test(args: TestArgs) -> CmdResult {
    let drive = drive::Drive::parse(&args.drive.drive)?;
    drive.guard_writes(args.guards.allow_fixed)?;
    let (free, total) = drive.space()?;
    let used_before = total - free;
    let file_size = args.size_mb.max(16) * (1 << 20);
    if free < file_size + (64 << 20) {
        return Err(format!(
            "not enough free space on {} for a {} MiB test file",
            drive.display(),
            file_size >> 20
        )
        .into());
    }

    if !args.json {
        ui::header(&format!("Flashmark test — {}", drive.display()));
        ui::kv(
            "filesystem",
            &drive.fs_name().unwrap_or_else(|| "unknown".into()),
        );
        ui::kv("volume size", &report::human_bytes(total));
        ui::kv("free space", &report::human_bytes(free));
    }

    let usb = match usb::for_drive(&drive) {
        Ok(u) => Some(u),
        Err(e) => {
            if !args.json {
                ui::warn(&format!("USB details unavailable: {e}"));
            }
            None
        }
    };
    if !args.json {
        if let Some(u) = &usb {
            report::print_usb(u);
        }
        println!();
    }

    let speed_result = speed::run(&drive.root(), file_size, std::time::Duration::from_secs(5))?;
    if !args.json {
        report::print_speed(&speed_result);
        println!();
    }

    let verify_result = if args.skip_verify {
        None
    } else {
        if !args.json {
            warn_if_fat32(&drive);
        }
        Some(verify::quick(&drive.root(), free, None, false)?)
    };

    let card = report::CardInput {
        drive_display: drive.display(),
        total,
        used_before,
        usb: usb.as_ref(),
        speed: &speed_result,
        verify: verify_result.as_ref(),
    };

    if args.json {
        let (tier, summary) = report::verdict(&card);
        let doc = serde_json::json!({
            "command": "test",
            "drive": drive.display(),
            "volume_total_bytes": total,
            "usb": usb,
            "speed": speed_result,
            "verify": verify_result,
            "verdict": { "tier": tier, "summary": summary },
        });
        println!("{}", serde_json::to_string_pretty(&doc)?);
    } else {
        report::print_card(&card);
    }

    let (tier, _) = report::verdict(&card);
    if tier == "FAIL" {
        return Err("test FAILED — see report above".into());
    }
    Ok(())
}

fn cmd_clean(args: CleanArgs) -> CmdResult {
    let drive = drive::Drive::parse(&args.drive.drive)?;
    if drive.kind() == drive::DriveKind::NoRootDir {
        return Err(format!("drive {} does not exist", drive.display()).into());
    }
    let removed = verify::clean(&drive.root())?;
    ui::ok(&format!(
        "removed {removed} flashmark test file(s) from {}",
        drive.display()
    ));
    Ok(())
}
