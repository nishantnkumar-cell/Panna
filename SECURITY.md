# Security policy

PDF files are untrusted input. This project treats every byte of a PDF, and of every other
file it reads, as hostile. The full model (what's protected, from whom, each known attack
class and how it's handled, and what's still open) is in
[docs/THREAT_MODEL.md](docs/THREAT_MODEL.md).

## What we guarantee

- **Memory safety in our own code.** `panna` and `panna-core` use `#![forbid(unsafe_code)]`.
  The only `unsafe` we wrote is the Windows print dialog in `panna-print`, kept small and
  commented.
- **No network access.** The app never connects to the internet. There are no updates, no
  telemetry and no cloud features. Links in documents open in your browser only after you
  agree.
- **No active content.** PDF JavaScript, launch actions and opening attached files are not
  supported. Attachments can only be saved, and programs among them are labelled.
- **Hostile files are refused, not obeyed.** Files built to exhaust memory (decompression
  bombs, impossible image sizes) are refused when opened, with an explanation.

## What we can't guarantee

No software can promise zero unknown vulnerabilities. Some dependencies contain `unsafe` code
we don't control. The JPEG decoder's SIMD kernels, a few in the PDF engine's syntax layer,
the GPU stack and OS dialogs are listed in the threat model. We reduce the risk by:

- choosing memory-safe, `unsafe`-free crates for nearly everything that parses PDF data, and
  turning off optional SIMD code that would touch it,
- capping resource use (file size, page count, decoded stream size, image size, bitmap size),
- catching panics from the engine on its worker thread,
- fuzzing the open, render, analysis, save and import paths in CI,
- auditing dependencies against the RustSec advisory database on every change and nightly,
  and pinning every CI action and build tool,
- planning to isolate rendering in a sandboxed child process (threat model, section 7).

## Reporting a vulnerability

Please report security bugs privately through GitHub: on the repository's **Security** tab,
choose **Report a vulnerability**. Don't open a public issue for a security bug. Include the
file that triggers it if you can. We'll confirm receipt, and credit you in the fix unless you
prefer not to be named.
