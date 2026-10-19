# Threat model

This is Panna's security model: what it protects, who and what it protects against, where
untrusted data enters, the known attack classes for PDF software and how each is handled, and
what is still open. [SECURITY.md](../SECURITY.md) is the short policy; this is the detail
behind it. Last reviewed 2026-10-19 against branch `windows/security`.

## 1. What Panna protects

| Asset | Why it matters |
|---|---|
| The user's computer and files | A PDF must never be able to run code, read or write files, or reach the network |
| The user's documents | Saved files must contain what the user meant, and nothing they removed (redaction, cleaning) |
| Private keys and certificates | Signing keys in certificate files or the Windows store must not leak or be misused |
| Passwords | Passwords for encrypted PDFs and certificate files must not be stored or sent anywhere |
| Trust decisions | A signature shown as valid and trusted must really be; names shown must be real |
| Availability | A hostile file may fail to open, but must not crash or freeze the app or the computer |

## 2. Who attacks, and how

The main attacker **sends the user a file**: a PDF, or an FDF/XFDF comment or form-data file,
or a picture to insert. They control every byte of it and can try many variants. They can't
run code on the computer beforehand or watch it while Panna runs.

Out of scope: someone who already runs code as the user, or has administrator rights, or has
the unlocked computer in their hands. Panna can't defend against them, and doesn't try.

Supply-chain attackers (a compromised crate, GitHub Action or build tool) are in scope for
how Panna is built (section 6).

## 3. Trust boundaries and entry points

```
 untrusted                    │ Panna process                                  │ system
                              │                                                │
 PDF file ───────────────────▶│ panna-core: guard → hayro parse → render/text  │
 FDF / XFDF file ────────────▶│ commentdata / formdata importers (no DTDs)     │
 Picture (PNG/JPEG/TIFF) ────▶│ image crate decoders (with allocation limits)  │
 Certificate file (.p12/.pem)▶│ p12-keystore, x509-cert, der                   │
 Settings file (own) ────────▶│ settings.rs (size-capped, line parser)         │
                              │                                                │
                              │ links.rs ── user confirms ──▶ browser / mail   │──▶ rundll32 / open / xdg-open
                              │ certstore.rs ── fixed script, stdin ─────────▶ │──▶ PowerShell (Windows store)
                              │ speech.rs, scan.rs ── fixed args, stdin ─────▶ │──▶ OS speech, scanner
                              │ panna-print (the only unsafe code) ──────────▶ │──▶ Windows print APIs
```

- **Everything in a PDF is hostile**: objects, streams, fonts, images, names, metadata,
  annotations, form fields, signatures, attachments, links and actions.
- Each document is parsed and rendered on its **own worker thread**; the UI thread never
  parses PDF data. Engine panics are caught at that boundary and become errors.
- Panna makes **no network connections**. `cargo-deny` keeps HTTP/TLS crates out of the tree.

## 4. Attack classes and how each is handled

Status key: **Mitigated**: handled and tested · **Partly**: reduced, with a known gap ·
**Open**: not yet handled · **Accepted**: a known, deliberate residual risk.

### 4.1 Code execution and memory corruption

| Threat | Status | How |
|---|---|---|
| Memory bugs in PDF parsing (the main source of PDF-reader CVEs) | Mitigated | `panna` and `panna-core` `#![forbid(unsafe_code)]`. The PDF engine is pure Rust: hayro, hayro-interpret, hayro-jpeg2000, hayro-jbig2, hayro-ccitt, hayro-cmap, skrifa and read-fonts (fonts), vello_cpu, png, miniz_oxide and roxmltree all forbid `unsafe` |
| Memory bugs in the `unsafe` code that untrusted data still reaches | Partly | Checked by reading each crate's source. hayro's SIMD image scaling (`pic-scale`) is **turned off in this review**, for about 25 ms more per full-page photo. What remains: zune-jpeg's SIMD kernels (IDCT, colour conversion, upsampling), switched on by the `image` crate's defaults, which can't be overridden; hayro-syntax's `unsafe` feature (memchr, flate2 over miniz_oxide, JBIG2 SIMD), which hayro-interpret always enables; and one documented lifetime transmute in hayro-syntax. All are widely used and fuzzed upstream, and our fuzzers reach them. Containing them fully needs process isolation (section 7) |
| Memory bugs in other dependencies (GPU stack, windowing, OS dialogs) | Accepted | They don't see PDF data, only the app's own UI and bitmaps already bounded by `Limits` |
| PDF JavaScript, Launch and Run actions, auto-opening attachments | Mitigated | Not supported at all. Form calculations are evaluated without a script engine (`calc.rs`). Link reading keeps only page jumps and http/https/mailto (`links.rs`) |
| Opening a link runs something else | Mitigated | Only http, https and mailto with no quotes, spaces or control characters (`safe_uri`); the user confirms; the address is one argument, never a shell; `rundll32` is run by full System32 path |
| Saving an attachment that is really a program | Mitigated | Attachments are never opened, only saved through a Save dialog. Names are cut to the last path part, control and text-direction characters are removed, and programs or scripts (exe, scr, lnk, js, msi, …) are labelled as such |
| Argument or command injection into helper programs | Mitigated | PowerShell (certificate store, speech, scanning on Windows) runs fixed scripts by full path with data on stdin; the thumbprint is checked as 40 hex digits. SANE and speech on Linux and macOS get fixed arguments or stdin |
| The print code (the only `unsafe` in the project) | Mitigated | Reviewed: handles are freed once, buffer sizes match the bitmap header, page ranges are bounds-checked, bitmaps are capped by `Limits` |

### 4.2 Denial of service

| Threat | Status | How |
|---|---|---|
| Decompression bombs: a small stream that inflates to gigabytes | **Mitigated (fixed in this review)** | Before anything is decoded, `guard.rs` inflates every Flate stream (including chains and ASCII-wrapped data) through a counter that keeps nothing, and refuses the file past `max_decoded_bytes` (512 MiB) per stream or per page. Found by testing: a 2 MB file previously took over 4.6 GB |
| An image whose header claims a huge size (65535 × 65535 JPEG) | **Mitigated (fixed in this review)** | Image size is checked from the dictionary and from the JPEG and JPEG 2000 headers against `max_image_pixels` (256 M). Previously such a 2 KB file took over 4 GB |
| Attachment bombs | **Mitigated (fixed in this review)** | Attachments are size-checked the same way before being unpacked for saving |
| Huge files, page counts and render sizes | Mitigated | `Limits`: 1 GiB files, 100 000 pages, 12 000 px and 64 Mpx bitmaps |
| Reference cycles and deep nesting (page trees, outlines, form XObjects, patterns, fields, xref `/Prev`) | Mitigated | Tested with hostile files (`examples/hostile.rs`): all finish in under 2 s. hayro caps object nesting, and Panna's walkers keep visited sets and depth limits |
| Very long content streams (millions of operators) | Partly | They finish, but can use about 700 MB for a 1 MB file. A memory cap needs process isolation |
| Rendering that takes very long | Partly | The UI stays responsive (the worker is separate), but a page can keep one CPU core busy. A time limit needs process isolation |
| Engine panics | Mitigated | `catch_unwind` around every engine call; release builds keep unwinding (`panic = "unwind"`) |
| Stack overflow from deep recursion | Partly | Not seen in testing (hayro and Panna limit depth), but a stack overflow can't be caught and would end the process |

### 4.3 Integrity, confidentiality and trust

| Threat | Status | How |
|---|---|---|
| Redaction that only covers content | Mitigated | Text, pictures, drawings, nested forms and screen-reader text under the marks are removed, and the file is rewritten in full, so no earlier revision survives. Page thumbnails (`/Thumb`) are dropped. Tests check the removed bytes are gone from the output |
| Hidden information left in a document | Partly | "Remove hidden information" clears metadata, scripts, attachments and comments. Redaction doesn't search bookmarks or comment text: the user must remove those, or use that tool |
| Universal signature forgery (missing or empty signature data) | Mitigated | Missing `ByteRange` or unreadable CMS gives "not valid" or "can't be checked", never valid |
| Signature wrapping (moving the signed byte ranges) | Mitigated | The range must start at 0, and the gap must be exactly one hex `<…>` value of bounded length |
| Incremental saving and shadow attacks (changes after signing) | Mitigated | Any bytes after the signed range are reported: "valid for the version that was signed; the document was changed afterwards", with a warning icon |
| Trusting a certificate that couldn't issue certificates | **Mitigated (fixed in this review)** | A chain link is accepted only if the issuer is marked as a CA (basic constraints), or is a version 1 self-signed root. Before this, a trusted end-user certificate could vouch for anyone |
| SHA-1 collisions in certificate chains | **Mitigated (fixed in this review)** | A SHA-1-signed certificate no longer builds a chain to a trusted one |
| SHA-1 document signatures | Accepted | They're checked and shown, since old documents use them; the `adbe.x509.rsa_sha1` and `pkcs7.sha1` formats are reported as unsupported |
| Revocation (a certificate revoked after signing) | Accepted | OCSP and CRL checks need the network, so they're not done. The panel says who issued the certificate and whether the user trusts it |
| Spoofed names (signer, bookmark, file name) with Unicode direction overrides | **Mitigated (fixed in this review)** | Overrides, embeddings and isolates are removed from PDF text strings, certificate names and attachment names. The marks right-to-left languages use are kept |
| Weak encryption on save | Mitigated | New protection is always AES-256 (revision 6), with keys, salts and IVs from the OS random generator. Saving changes into a file that already uses RC4 or AES-128 keeps that protection: use Protect to upgrade it |
| Passwords stored or leaked | Partly | Passwords are never written to disk or settings, and password fields are masked. They stay in process memory while the document is open, and aren't zeroed afterwards |
| Private keys | Mitigated | Windows-store keys never leave Windows: Panna sends the digest and Windows signs (and asks for a PIN if needed). Certificate-file keys are read only to sign, then dropped. The `rsa` crate's Marvin timing issue (RUSTSEC-2023-0071) needs someone timing many private-key operations from outside, which local signing doesn't allow |
| XML attacks (external entities, entity expansion) on XFDF | Mitigated | `roxmltree` with `allow_dtd: false`: no DTDs, so no entities; it never fetches anything. XMP metadata is only searched as text |
| Network leaks (tracking pixels, phone-home, remote fonts) | Mitigated | No networking code or crates; nothing is fetched while opening or rendering |
| Settings file tampering | Mitigated | Size-capped (64 KiB), line-based, unknown keys ignored, values validated; recent-file paths are only offered, never opened automatically |

## 5. How this was checked

- **Code review** of every place untrusted data enters: links, attachments, signatures and
  certificate chains, redaction, encryption, XML import, process launching, the print FFI,
  settings, OCR model loading (the models are compiled into the binary; nothing is downloaded).
- **Attack files**: `crates/panna-core/examples/hostile.rs` builds 17 hostile PDFs (decompression
  bombs in content, object streams, XMP, attachments and split content; oversized images and
  JPEG headers; reference cycles in XObjects, patterns, page trees, outlines, fields and xref;
  deep nesting; millions of operators) and runs each through open, render, text, attachments,
  signatures, PDF/A and accessibility checks, comments and three kinds of save. A harness
  measured time and peak memory per file in a separate process. Before this review three of
  them took over 4 GB; now every one finishes under 750 MB, and the bombs are refused using at most
  160 MB. `tests/hostile.rs` keeps the same cases as fast regression tests.
- **Fuzzing**: three cargo-fuzz targets run in CI on every pull request (5 minutes each) and
  nightly (1 hour each): `open_render`, `analyse` (checks, redaction, flattening, compaction,
  tagging, reopening each output) and `import_data` (FDF and XFDF).
- **Dependency audit**: `cargo audit` (RustSec) and `cargo deny` (advisories, licences, banned
  crates, sources) on every push and nightly. Status at this review: one advisory, accepted
  with a reason (RUSTSEC-2023-0071 in `rsa`, see 4.3); `lru` is flagged unsound
  (RUSTSEC-2026-0253) but is used only by iced's glyph cache for the app's own text, with no
  fixed release yet; `ttf-parser`, `rustybuzz`, `paste` and the zbus/zvariant family are
  flagged unmaintained, and none of them parse PDF content (PDF fonts go through skrifa).

## 6. Building and releasing

| Threat | Status | How |
|---|---|---|
| A compromised GitHub Action | **Mitigated (fixed in this review)** | Every action is pinned to a full commit SHA, not a tag |
| A tampered build tool or script | **Mitigated (fixed in this review)** | `cargo-fuzz` and the Python packages are pinned to versions; the Flatpak cargo generator is fetched from a fixed commit and its SHA-256 is checked |
| A malicious or vulnerable crate | Mitigated | `Cargo.lock` is committed; `cargo-deny` allows only crates.io and an approved licence list, and bans networking crates; RustSec checks run nightly |
| CI tokens misused by pull requests | Mitigated | The workflow token is read-only (`contents: read`), and there's no `pull_request_target` |
| Unsigned releases | Open | Windows and macOS builds aren't code-signed yet, and macOS builds aren't notarised (that needs an Apple developer account) |

## 7. Open items, in priority order

1. **Process isolation for the render worker.** Run parsing and rendering in a child process
   with a memory limit (a job object on Windows, rlimit or cgroup on Linux), no file-system
   or network access (AppContainer on Windows, App Sandbox on macOS, seccomp and Landlock on
   Linux), and a time limit per page. That turns every remaining denial-of-service item into
   "this page couldn't be shown", and contains any memory bug in a dependency's `unsafe` code.
   Planned in [ARCHITECTURE.md](ARCHITECTURE.md).
2. **Code-signed and notarised releases**, with checksums published beside each download.
3. Zero passwords and decrypted keys in memory after use (`zeroize`).
4. Have redaction also search bookmarks and comments for the redacted text, and offer to
   remove matches.
5. Offer to upgrade old RC4 or AES-128 protection to AES-256 when saving changes.
6. Larger fuzzing corpora: seed the fuzzers with real-world PDFs (pdf.js and PDFBox test
   suites) and keep crash inputs as regression tests.

## 8. Reporting a vulnerability

See [SECURITY.md](../SECURITY.md).
