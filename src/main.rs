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
        Err(e) => {
            ui::error(&format!("{e:#}"));
            ExitCode::FAILURE
        }
    }
}

type CmdResult = Result<(), Box<dyn std::error::Error>>;

fn cmd_info(_args: InfoArgs) -> CmdResult {
    Err("not implemented yet".into())
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

fn cmd_verify(_args: VerifyArgs) -> CmdResult {
    Err("not implemented yet".into())
}

fn cmd_test(_args: TestArgs) -> CmdResult {
    Err("not implemented yet".into())
}

fn cmd_clean(_args: CleanArgs) -> CmdResult {
    Err("not implemented yet".into())
}
