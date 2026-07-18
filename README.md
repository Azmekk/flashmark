# Flashmark

A flash-drive testing utility for Windows. Verifies the three claims every USB stick makes:

- **Speed** — sequential and random 4K read/write, measured with unbuffered I/O (no OS cache lies)
- **Capacity** — detects fake flash (a "1 TB" stick that is really 32 GB) with quick-probe and full H2testw-style verification
- **USB version** — reads the *negotiated* link speed from the USB hub and compares it against what the device *claims* to support

## Commands

```
flashmark info   <drive>   # device identity, claimed vs negotiated USB speed
flashmark speed  <drive>   # sequential + 4K random benchmark
flashmark verify <drive>   # capacity check: --quick (minutes) or --full (thorough)
flashmark test   <drive>   # everything above, ending in a report card
flashmark clean  <drive>   # remove leftover flashmark test files
```

All commands accept `--json` for machine-readable output.

## Safety

- File-based testing only — Flashmark never writes to the raw device.
- Refuses to run write tests on the system drive.
- Refuses non-removable drives unless `--allow-fixed` is passed.
- Test files are cleaned up automatically, including on Ctrl+C.

## Status

v0.1 — Windows only. Cross-platform support planned.
