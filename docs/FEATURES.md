# Feature map: Adobe Acrobat → this project

Scope rule: everything Acrobat does **offline** is in scope eventually. Features that need
Adobe's cloud or any network service are out of scope.

Priority key: **V1** = first release (view, annotate, fill forms) · **Later** = planned ·
**Skip** = needs online services, or is out of scope by design.

## 1. Viewing and navigation

| Feature | Priority | Notes |
|---|---|---|
| Open PDF files (file dialog, drag and drop, command line) | V1 | Done in skeleton |
| Continuous scroll, single page, two-page spread | V1 | Continuous scroll done; spreads later in V1 |
| Zoom in/out, fit width, fit page, actual size | V1 | Done |
| Rotate view | V1 | Done |
| Page thumbnails sidebar | V1 | Done (F4); left or right edge, remembered |
| Bookmarks / outline panel | V1 | Done; nested, click to jump |
| Go to page, next/previous page | V1 | Done |
| Text search (find, find next, match case, whole word) | V1 | Done: search panel in the sidebar listing every result in context, like Word (Ctrl+F, F3/Shift+F3); includes hidden OCR text |
| Select and copy text | V1 | Done; drag or double-click, Ctrl+C; across pages; Linux middle-click paste |
| Copy image / snapshot tool | Later | |
| Internal links and cross-references | V1 | |
| Tabs / multiple documents open | Later | |
| Full-screen / presentation mode | Later | |
| Night mode / custom page colours (accessibility) | Later | |
| Recent files list | V1 | Done; stored locally only |
| Toolbars dock to any edge or float; position remembered | V1 | Done; icon-only (Lucide, ISC) with hover tips |
| Print | Later | Uses OS print dialog |
| Document properties (title, author, page size, PDF version) | V1 | Done (Ctrl+D) |
| Password-protected (encrypted) PDFs | V1 | Done |
| Read-aloud | Later | Uses OS speech engine only |
| Attachments panel (embedded files) | Later | |
| Layers (optional content) toggle | Later | |
| Measuring tools (distance, area, perimeter) | Later | |
| Compare two files | Later | Acrobat's compare is cloud-assisted; ours is local |

## 2. Commenting and annotation

| Feature | Priority | Notes |
|---|---|---|
| Highlight, underline, strikethrough text | V1 | |
| Sticky notes | V1 | |
| Free text / text box comments | V1 | |
| Freehand drawing (ink) | V1 | |
| Shapes: rectangle, ellipse, line, arrow, polygon, cloud | V1 | |
| Stamps (Approved, Draft, custom image stamps) | Later | |
| Comment list panel, replies, status (accepted/rejected) | V1 | Replies later |
| Edit / move / delete annotations, colour and opacity | V1 | |
| Undo / redo | V1 | |
| Save annotations back into the PDF (standard annotations) | V1 | Readable by Acrobat and other viewers |
| Import / export comments (FDF, XFDF) | Later | |
| Flatten annotations | Later | |
| Shared review / cloud commenting | Skip | Online |

## 3. Forms

| Feature | Priority | Notes |
|---|---|---|
| Fill AcroForm fields: text, checkbox, radio, list, dropdown | V1 | |
| Tab between fields, field validation and formatting | V1 | Basic formats (number, date) |
| Save filled form | V1 | |
| Reset form | V1 | |
| Flatten form | Later | |
| Import / export form data (FDF, XFDF) | Later | |
| Create and edit form fields | Later | |
| Auto-detect form fields | Later | Offline heuristic only |
| Form JavaScript calculations | Later | **Sandboxed** JS subset only, or skip. JavaScript is a major attack surface in PDF readers |
| XFA forms (dynamic LiveCycle forms) | Skip | Deprecated, huge attack surface |
| Fill & Sign: add typed/drawn signature image, initials, dates | Later | |
| Send for signature / e-sign workflows | Skip | Online |

## 4. Page organisation

| Feature | Priority | Notes |
|---|---|---|
| Insert, delete, reorder, rotate pages | Later | |
| Extract pages, split document | Later | |
| Merge / combine files | Later | |
| Crop pages | Later | |
| Headers, footers, page numbers, Bates numbering | Later | |
| Watermarks and backgrounds | Later | |

## 5. Editing content

| Feature | Priority | Notes |
|---|---|---|
| Edit existing text (reflow within a text box) | Later | Hardest feature; font subsetting issues |
| Add text, images, links | Later | |
| Replace / move / resize images | Later | |
| Redaction (true content removal, search and redact) | Later | Must remove underlying content, not just cover it |
| Remove hidden information / sanitise document | Later | |

## 6. Create and convert

| Feature | Priority | Notes |
|---|---|---|
| Images (PNG, JPEG, TIFF) → PDF | Later | |
| PDF → images | Later | |
| PDF → plain text | Later | |
| PDF → Word / Excel / PowerPoint | Later | Acrobat does this offline; quality is a large project |
| Word / Office → PDF | Skip | Needs Office or LibreOffice installed; could shell out later |
| Web page → PDF | Skip | Needs network |
| Scan to PDF (from scanner) | Later | OS scanner APIs |
| OCR (make scanned PDF searchable) | Later | Local OCR engine; the Tesseract C++ dependency would need sandboxing |
| Optimise / reduce file size | Later | |
| PDF/A, PDF/X conversion and preflight checks | Later | |

## 7. Security and signatures

| Feature | Priority | Notes |
|---|---|---|
| Open encrypted PDFs | V1 | |
| Encrypt with password, set permissions | Later | AES-256 only |
| Digitally sign with a local certificate (PKCS#12 / OS store) | Later | |
| Validate signatures | Later | Revocation checks (OCSP/CRL) are online, so they stay optional and off by default |
| Certify document | Later | |
| Adobe Sign, cloud ID | Skip | Online |

## 8. Accessibility

| Feature | Priority | Notes |
|---|---|---|
| Keyboard navigation for all actions | V1 | |
| Screen reader support for the app UI | Later | Depends on iced accessibility support |
| Tag PDF, reading order, accessibility checker | Later | |

## 9. Online-only (always Skip)

Adobe Document Cloud storage and sync, sharing links, shared review, Adobe Sign, the AI Assistant,
generative summaries, online export/convert services, Acrobat web, and any telemetry. **The app
makes no network connections.** CI enforces this by making sure no networking crates are in the
dependency tree.
