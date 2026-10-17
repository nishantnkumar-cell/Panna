# Security policy

PDF files are untrusted input. This project treats every byte of a PDF as hostile.

## What we guarantee

- **Memory safety in our own code.** Every crate in this repository uses `#![forbid(unsafe_code)]`.
- **No network access.** The app never connects to the internet. There are no updates, no
  telemetry and no cloud features.
- **No active content.** PDF JavaScript, launch actions and auto-opening attachments are not
  supported.

## What we can't guarantee

No software can promise zero unknown vulnerabilities. Some dependencies (the GPU stack, SIMD
code, OS dialogs) contain `unsafe` code that we don't control. We reduce that risk by:

- choosing pure-Rust, `unsafe`-free crates for everything that parses PDF data,
- capping resource use (file size, page count, bitmap size),
- catching panics from the rendering engine,
- running fuzzers against the open and render paths,
- auditing dependencies against the RustSec advisory database on every change,
- planning to isolate rendering in a sandboxed child process (see `docs/ARCHITECTURE.md`).

## Reporting a vulnerability

Until this project has a public home, report issues privately to the maintainer. Please
don't open public issues for security bugs.
