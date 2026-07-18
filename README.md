# Flashmark

A flash-drive testing utility for Windows. Verifies the three claims every USB stick makes:

- **Speed** — sequential and random 4K read/write, measured with unbuffered write-through I/O so the Windows cache can't flatter the numbers
- **Capacity** — detects fake flash (a "1 TB" stick that is really 32 GB) with a quick marker probe or a full H2testw-style every-byte verification
- **USB version** — reads the *negotiated* link speed from the USB hub and compares it against the spec the device *claims* in its own descriptor (`bcdUSB`)

## Usage

```
flashmark info   <drive>    # identity, claimed vs negotiated USB link
flashmark speed  <drive>    # sequential + random 4K benchmark
flashmark verify <drive>    # capacity check (quick probe by default)
flashmark test   <drive>    # info + speed + quick verify → report card
flashmark clean  <drive>    # remove leftover flashmark test files
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
- **Quick verify** preallocates files across free space and checks 4K markers at the start/middle/end of each — address-wrapping fakes fail in minutes. A drive with a large RAM cache could in principle fool it; `--full` is the authoritative answer.
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

- USB link detection is implemented but still needs validation against real flash-drive hardware (the development machine had none attached); non-USB drives are correctly identified.
- Quick verify's preallocation is instant on NTFS (sparse) and exFAT (valid-data length), but FAT32 may zero-fill, degrading quick mode toward full-mode duration.
- Sequential read of the just-written test file can be served partly by the drive's own cache on drives with large RAM buffers; use a bigger `--size-mb` if numbers look implausible.
- Windows only. The benchmark/verify core is portable; Linux/macOS need only a platform probe layer (`O_DIRECT`, sysfs).
