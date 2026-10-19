# Architecture

## Decisions

| Area | Choice | Why |
|---|---|---|
| UI toolkit | **iced 0.14** | Pure Rust, MIT license, cross-platform, good custom canvas, no web engine |
| PDF rendering | **hayro 0.8** | Pure Rust, the crate itself forbids `unsafe`, ~1000-file regression suite (pdf.js + PDFBox corpora), used by Typst |
| PDF writing (annotations, forms) | **Own incremental writer** (`panna-core/src/edit`) | Appends changes after the original bytes, like Acrobat's "save". The original revision is never rewritten, so signatures and anything we don't understand survive. Changed objects are copied byte for byte apart from the keys we edit. We chose this over lopdf, which would mean a second full parser of untrusted input and rewriting the whole file |
| File dialogs | **rfd** | Native OS dialogs; on Linux it talks to the XDG portal over D-Bus (pure Rust, no GTK) |
| Alternatives rejected | MuPDF (AGPL, C), PDFium (C++; would need process sandboxing), pdf.js (needs a JS engine) | |

### Known engine limitations (hayro 0.8)

- No blend modes or knockout groups yet. Some PDFs with complex transparency render slightly wrong.
- It's CPU-only rendering with little performance tuning so far. Very large pages render slowly.
- No text extraction API. Search and copy use our own text layer (`panna-core/src/text.rs`), built as a `hayro-interpret` device that records glyphs instead of drawing them.
- Annotations are drawn only from their appearance streams, so every annotation we create gets one.

Saving into encrypted PDFs isn't supported yet, because new objects would need to be encrypted with the file's key.

If hayro can't keep up, the fallback is PDFium running in a separate sandboxed process
(see "Process isolation" below). That way a C++ memory bug can't touch the UI process.

## Layout

```
crates/
  panna-core/   document loading, limits, rendering, background render worker (no GUI deps)
  panna/    iced desktop app (binary: panna)
fuzz/         cargo-fuzz targets that feed random bytes through open, render, text extraction and save
docs/         this file, the feature map
```

`panna-core` has no GUI dependencies. It can be tested, fuzzed and reused (for example in a CLI)
without iced.

## Threading model

- The UI thread never parses or renders PDF data.
- Each open document gets one **render worker thread**. The worker owns the file bytes and the
  parsed document. The UI sends it page-render jobs and gets back RGBA bitmaps.
- Every render runs inside `catch_unwind`. If the engine panics on a malformed file, you get an
  error message instead of a crash.
- Only pages near the viewport are rendered. Bitmaps far from the viewport are dropped to keep
  memory use bounded.

## Security model

See [SECURITY.md](../SECURITY.md) for the policy and [THREAT_MODEL.md](THREAT_MODEL.md) for the full\nthreat model. The technical controls are:

1. `#![forbid(unsafe_code)]` in every crate we write, except `panna-print`, which calls the
   Windows and macOS print dialogs. It never sees the original file, only the password-free
   copy made for printing, and each `unsafe` block there says why it is sound.
2. Input limits: maximum file size, maximum page count, a cap on render bitmap dimensions and
   total pixels. These stop "decompression bomb" style PDFs from exhausting memory.
3. Panic containment around all engine calls.
4. No network: the app has no networking code. `cargo-deny` bans HTTP and TLS crates from the
   dependency tree.
5. No PDF JavaScript, no launching external programs from PDF actions, no auto-opening of
   embedded files.
6. Fuzzing: `fuzz/` targets run in CI on a schedule.
7. Supply chain: `cargo-audit` (RustSec advisories) and `cargo-deny` (licenses, bans, sources)
   run on every push. `Cargo.lock` is committed.

### Process isolation (planned, before 1.0)

Move the render worker into a child process that has no file system or network access
(seccomp on Linux, App Sandbox on macOS, an AppContainer / restricted token on Windows). The
UI sends it the file bytes and gets back bitmaps. That way even a bug in a dependency's
`unsafe` code (SIMD, image decoders) can't reach the user's files.
