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
| Memory bugs in the `unsafe` code that untrusted data still reaches | Partly | Checked by reading each crate's source. hayro's SIMD image scaling (`pic-scale`) is **turned off in this review**, for about 25 ms more per full-page photo. What remains: zune-jpeg's SIMD kernels (IDCT, colour conversion, upsampling), switched on by the `image` crate's defaults, which can't be overridden; hayro-syntax's `unsafe` feature (memchr, flate2 over miniz_oxide, JBIG2 SIMD), which hayro-interpret always enables; and one documented lifetime transmute in hayro-syntax. All are widely used and fuzzed upstream, and our fuzzers reach them. Pages on screen are now drawn in a locked-down renderer process (section 7), so a bug there can't reach files or the network on Linux, or write to the user's files on Windows. Text extraction, export and editing still parse in the app's own process |
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
| Very long content streams (millions of operators) | Mitigated for display | They finish, but can use about 700 MB for a 1 MB file. On screen they're drawn in the renderer process, which is capped at 4 GB (Linux address space, Windows job object) and replaced if it fails |
| Rendering that takes very long | Mitigated for display | The UI stays responsive, and a page that takes over 30 seconds to draw has its renderer process stopped; the next page starts a fresh one |
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

1. **Process isolation: done for drawing pages on screen; still to do for the rest.** The app
   starts a copy of itself as a renderer (`panna_core::isolate`), which locks itself down
   before reading PDF data (`panna-sandbox`): on Linux, Landlock takes away all file access,
   seccomp refuses sockets, starting programs and reaching into other processes, and the
   address space is capped at 4 GB; on Windows it drops to low integrity and joins a job
   object with a 4 GB memory cap, no child processes and no clipboard or desktop access; on
   macOS only the memory cap applies so far (the Seatbelt sandbox is still to do). It draws a
   test page once locked down; if that fails, pages are drawn in the app as before. Each page
   has 30 seconds. Still in the app's process: text extraction, thumbnails' text, export,
   printing, editing and the checks. Moving those needs the same protocol for more requests.
2. **Code-signed and notarised releases**, with checksums published beside each download.
3. **Wiping passwords and keys from memory: done where Panna holds them.** Open and
   permissions passwords, certificate-file passwords, the file key of an encrypted PDF and
   the passwords typed into the app are kept in `zeroize::Zeroizing`, so they are
   overwritten when replaced or dropped; each keystroke's copy is wiped once read, and the
   renderer wipes the password from its load request once the file is open. Signing keys
   from certificate files (RSA, P-256, P-384) wipe themselves in their own crates. Not
   covered: the copies iced keeps inside a text box while it is on screen, the decoded key
   that `p12-keystore` holds briefly while reading a certificate file, and buffers the
   operating system keeps for pipes.
4. **Redaction also searches bookmarks and comments: done.** Before redacting, Panna reads
   the text under each marked area and counts the bookmarks and comments elsewhere in the
   file that mention it (ignoring case and spacing). The question offers to remove those
   too, ticked by default: matching comments are deleted with their pop-ups and replies,
   and the text is blacked out in bookmark titles. Not searched: form field values,
   attached files, the document's title and metadata (Remove hidden information covers
   those), and text the page draws without a font Panna can read.
5. **Upgrading old RC4 or AES-128 protection: done.** A file with RC4 or AES-128 protection
   that was opened with its owner password (which now works for those files too: the user
   password is worked out from it) is offered AES-256 the first time it is saved, keeping
   both passwords and the permissions (any that AES-256 files here can't express come out
   stricter). The document's properties name the method. Opened with only the user
   password, the owner password is unknown, so the offer isn't made.
6. **Fuzzing corpora: seeded.** Each fuzzer starts from committed seeds (PDFs covering text,
   pictures, forms, comments, bookmarks, attachments, object streams, incremental updates
   and hostile structures, plus FDF, XFDF and CSV files), and the nightly run adds the
   pdf.js test suite's PDFs. Crash inputs go in `tests/fuzz-regressions/`, which `cargo
   test` runs along with the seeds. Still to add: the PDFBox suite.

## 8. Reporting a vulnerability

See [SECURITY.md](../SECURITY.md).
