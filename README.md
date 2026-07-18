# Flashmark

A flash-drive testing utility for Windows. Verifies the three claims every USB stick makes:

- **Speed** — sequential and random 4K read/write, measured with unbuffered write-through I/O so the Windows cache can't flatter the numbers
- **Capacity** — detects fake flash (a "1 TB" stick that is really 32 GB) with a quick marker probe or a full H2testw-style every-byte verification
- **USB version** — reads the *negotiated* link speed from the USB hub and compares it against the spec the device *claims* in its own descriptor (`bcdUSB`)

## Quick start

Install from PowerShell:

```powershell
irm https://raw.githubusercontent.com/Azmekk/flashmark/master/install.ps1 | iex
```

Open a new terminal, plug in the stick (say it mounted as `E:`), and run:

```powershell
flashmark test E:      # full check: USB link + speed + quick capacity probe
```

The installer downloads the latest release binary (a single self-contained exe, no runtime needed) into `%LOCALAPPDATA%\Programs\Flashmark` and adds it to your user PATH. Later, `flashmark update` upgrades in place. Alternatives: clone and run `.\install.ps1` to build from source, `cargo install --path .`, or grab `flashmark.exe` from the latest [release](../../releases).

## Usage

```
flashmark info   <drive>    # identity, claimed vs negotiated USB link
flashmark speed  <drive>    # sequential + random 4K benchmark
flashmark verify <drive>    # capacity check (quick probe by default)
flashmark test   <drive>    # info + speed + quick verify → report card
flashmark clean  <drive>    # remove leftover flashmark test files
flashmark update            # self-update to the latest release (--check to only look)
```

Useful flags:

| Flag | Where | Effect |
|---|---|---|
| `--json` | all | machine-readable output on stdout (progress stays on stderr) |
| `--full` | verify | write and verify every free byte — conclusive, but takes hours on large drives |
| `--limit-gb N` | verify | cap the tested span |
| `--keep` | verify | keep test files for later re-checking |
| `--size-mb N` | speed, test | benchmark file size (default 256) |
| `--allow-fixed` | write tests | permit testing a non-removable drive |

Exit code is non-zero when capacity verification fails, so the tool scripts cleanly.

## Interpreting results

- **MB/s is decimal** (10^6), matching drive marketing and CrystalDiskMark.
- Effective per-link ceilings: USB 1.1 ≈ 1 MB/s · USB 2.0 ≈ 40 MB/s · 5 Gbps ≈ 450 MB/s · 10 Gbps ≈ 900 MB/s. A "USB 3" stick topping out at ~35 MB/s is either on a USB 2 link (the tool tells you) or built from slow flash behind a fast controller (the report card notes this too).
- **Quick verify** alternates large *unwritten* spacer files with tiny fully-written 4K marker files, pushing markers across the device's address space without paying for the writes — address-wrapping and discarding fakes fail in minutes. A drive with a large RAM cache could in principle fool it; `--full` is the authoritative answer. Note: on FAT32 the filesystem zero-fills allocations, so quick mode runs at full-write speed there (the tool warns; exFAT and NTFS are instant).
- On a failed verify, the first corrupt offset yields an **estimated real capacity**.

## Safety

- File-based testing only — Flashmark never writes to the raw device, so it cannot damage partitions or other files.
- Refuses write tests on the system drive, and on fixed drives without `--allow-fixed`.
- Test files are removed on completion and on Ctrl+C; `flashmark clean <drive>` removes any stragglers after a hard kill.

## Building

```
cargo build --release   # → target/release/flashmark.exe
```

Requires stable Rust on Windows. No admin rights needed.

## Known limitations (v0.1)

- FAT32 cannot be probed quickly: the filesystem physically zero-fills every allocation, so quick verify degrades to full-write speed there. exFAT and NTFS preallocate instantly.
- Sequential read of the just-written test file can be served partly by the drive's own cache on drives with large RAM buffers; use a bigger `--size-mb` if numbers look implausible.
- Quick verify has been exercised on real hardware but not yet against an actual counterfeit drive; the detection logic is unit-tested.
- Windows only. The benchmark/verify core is portable; Linux/macOS need only a platform probe layer (`O_DIRECT`, sysfs).

USB link detection (claimed vs negotiated, including the `_V2` SuperSpeed query) is validated against real hardware.
