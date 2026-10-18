# Feature map: Adobe Acrobat → this project

Scope rule: everything Acrobat does **offline** is in scope eventually. Features that need
Adobe's cloud or any network service are out of scope.

Priority key: **V1** = first release (view, annotate, fill forms) · **Later** = planned ·
**Skip** = needs online services, or is out of scope by design.

## 1. Viewing and navigation

| Feature | Priority | Notes |
|---|---|---|
| Open PDF files (file dialog, drag and drop, command line) | V1 | Done in skeleton |
| Continuous scroll, single page, two-page spread | V1 | Done: continuous scroll, with one page or two side by side (optionally with the cover alone) from the toolbar |
| Zoom in/out, fit width, fit page, actual size | V1 | Done |
| Rotate view | V1 | Done |
| Page thumbnails sidebar | V1 | Done (F4); left or right edge, remembered |
| Bookmarks / outline panel | V1 | Done; nested, click to jump |
| Go to page, next/previous page | V1 | Done |
| Text search (find, find next, match case, whole word) | V1 | Done: search panel in the sidebar listing every result in context, like Word (Ctrl+F, F3/Shift+F3); includes hidden OCR text |
| Select and copy text | V1 | Done; drag or double-click, Ctrl+C; across pages; Linux middle-click paste |
| Copy image / snapshot tool | Later | Done: the camera button turns on the snapshot tool; dragging over a page copies a picture of the area, as shown and with unsaved changes, to the clipboard at 144 dpi or sharper. Esc turns it off |
| Internal links and cross-references | V1 | Done: page links jump straight there; web and mail links ask first, then go to the system browser or mail program (Panna itself stays offline). Links that launch programs, open files or run scripts are ignored |
| Tabs / multiple documents open | Done | Opening a file while another is open adds a tab, and each tab keeps its own zoom, place, search, comments and form entries. Ctrl+Tab and Ctrl+Shift+Tab move between tabs, Ctrl+W closes one (asking about unsaved changes), and closing the window asks about every tab with unsaved changes. Files named on the command line open in tabs of their own |
| Full-screen / presentation mode | Later | Done: full screen keeping the tools (F11), and presenting one page at a time on black (Ctrl+L, as in Acrobat); click, Space or arrows to move, Escape to leave |
| Night mode / custom page colours (accessibility) | Later | Done: original, dark (hues kept, lightness inverted) and sepia, cycled from the moon button or Ctrl+Shift+D; remembered |
| Recent files list | V1 | Done; stored locally only |
| Toolbars dock to any edge or float; position remembered | V1 | Done; icon-only (Lucide, ISC) with hover tips |
| Print | Later | Done: Ctrl+P or the printer button opens the system print dialog: KDE or GNOME through the desktop print portal (also in Flatpak), Windows' print dialog, and macOS's print panel with preview. Prints unsaved changes too, and honours a file's "no printing" permission |
| Document properties (title, author, page size, PDF version) | V1 | Done (Ctrl+D) |
| Password-protected (encrypted) PDFs | V1 | Done |
| Read-aloud | Later | Uses OS speech engine only |
| Attachments panel (embedded files) | Later | Done: a paperclip tab in the sidebar, shown when a document has attached files (its own or on pages), lists each with size, page and description and saves a copy; files are never opened |
| Layers (optional content) toggle | Later | Done: a Layers tab in the sidebar, shown when a document has layers, with a box to show or hide each one. Only the view changes, never the file |
| Measuring tools (distance, area, perimeter) | Later | Done: the ruler button measures the distance between two points, or the perimeter and area of a shape (double-click its last corner), in millimetres, centimetres, inches or points. Measurements are shown, not saved |
| Compare two files | Done | "Compare with another file…" in All tools compares the text word by word, all on this computer: differences are listed in the sidebar (green only in this file, red struck through only in the other) and marked on the pages. Text only; pictures and layout are not compared yet |

## 2. Commenting and annotation

| Feature | Priority | Notes |
|---|---|---|
| Highlight, underline, strikethrough text | V1 | Done; select text with the tool chosen, across pages |
| Sticky notes | V1 | Done |
| Free text / text box comments | V1 | Done; Helvetica, wrapped to the box |
| Freehand drawing (ink) | V1 | Done |
| Shapes: rectangle, ellipse, line, arrow, polygon, cloud | V1 | Done: click each corner of a polygon or cloud, then double-click, press Enter or click the first corner to finish; Escape drops it |
| Stamps (Approved, Draft, custom image stamps) | Later | Done: the 14 standard stamps, stamps with your own words (S key), and pictures such as a signature or logo (PNG or JPEG, transparency kept), movable and resizable. Picture stamps can't be moved again after the file is saved and reopened |
| Comment list panel, replies, status (accepted/rejected) | V1 | Done: Comments tab in the navigation pane, quotes highlighted text, click to jump. Saved comments can be replied to and given a review status (accepted, rejected, cancelled, completed); replies show in the comment's thread and are saved as standard /IRT reply annotations, which other PDF viewers read as threads |
| Edit / move / delete annotations, colour and opacity | V1 | Six-colour palette per tool done; any comment, saved or new, can be deleted from the list (undoable; removed from the file on save, with its pop-up). Drag any comment to move it; recolour it or set its opacity (slider) while selected; Delete key removes it. Saved comments of kinds Panna draws are redrawn on change. Drag a selected shape, drawing or text box by a corner handle to resize it; double-click a comment to edit its text |
| Undo / redo | V1 | Done (Ctrl+Z, Ctrl+Y) |
| Save annotations back into the PDF (standard annotations) | V1 | Done (Ctrl+S, Ctrl+Shift+S) as an incremental update with appearance streams, so other viewers show them; asks before closing with unsaved changes. Also saves into password-protected PDFs (RC4, AES-128, AES-256), keeping their encryption |
| Import / export comments (FDF, XFDF) | Later | Done: from All tools. Saves every comment to XFDF (with replies) or FDF, and adds comments from either kind of file to the open document as unsaved, undoable changes. Comments on pages the document doesn't have are left out, with a notice |
| Flatten annotations | Later | Done: comments are drawn into the pages (links stay links), from the page tools |
| Shared review / cloud commenting | Skip | Online |

## 3. Forms

| Feature | Priority | Notes |
|---|---|---|
| Fill AcroForm fields: text, checkbox, radio, list, dropdown | V1 | Done: fields drawn as inputs over the page (single and multiline text, password, max length, checkboxes, radio groups, drop-downs and lists). Multi-select lists keep their first choice only. Saved appearances use Helvetica, so characters outside Western European show as ? in other viewers until they redraw the field |
| Tab between fields, field validation and formatting | V1 | Done: Tab / Shift+Tab; the standard Acrobat formats (number, currency, percent, date, ZIP, phone, SSN) are recognised from their script calls without running any script. Typing is filtered, values are formatted when you move on, and invalid ones are outlined in red. Time formats and custom scripts are not supported |
| Save filled form | V1 | Done: values and fresh appearances written as an incremental update |
| Reset form | V1 | Done: eraser button in the toolbar, with a confirmation; fields return to their /DV defaults |
| Flatten form | Later | Done: fields are drawn with their values and the form is removed |
| Import / export form data (FDF, XFDF) | Later | Done: All tools fills in a form from an FDF or XFDF file, or saves its data as either. Scripts in data files are ignored |
| Create and edit form fields | Done | "Prepare a form" in All tools: click or drag to add text fields (one or several lines), check boxes and drop-down lists, with a name, a hover tip and "must be filled in"; click a field to delete it. Each change is a page change, undone from the page tools. Radio buttons, list boxes and moving or resizing fields are not yet here, and fields inside a group can't be deleted yet |
| Auto-detect form fields | Later | Offline heuristic only |
| Form JavaScript calculations | Later | **Sandboxed** JS subset only, or skip. JavaScript is a major attack surface in PDF readers |
| XFA forms (dynamic LiveCycle forms) | Skip | Deprecated, huge attack surface |
| Fill & Sign: add typed/drawn signature image, initials, dates | Later | Done: the Fill & Sign comment tool (F). Draw a signature or initials on a pad, or type them in a handwriting font (Dancing Script, OFL); click to place them as picture stamps, which can be moved and resized. Also places today's date (editable text), check marks and crosses. Signatures are kept until Panna closes, not stored on disk |
| Send for signature / e-sign workflows | Skip | Online |

## 4. Page organisation

| Feature | Priority | Notes |
|---|---|---|
| Insert, delete, reorder, rotate pages | Later | Done: tools above the page thumbnails, with undo (Ctrl+Z). Saving writes a fresh file, so deleted pages can't be recovered from it |
| Extract pages, split document | Later | Done: pick pages such as 1-3, 5, 8- and save them as one PDF or each as its own PDF |
| Merge / combine files | Later | Done: insert another PDF's pages after the current page |
| Crop pages | Later | Done: trim margins in millimetres from the edges as shown, on this page or all pages |
| Headers, footers, page numbers, Bates numbering | Later | Done: left, centre and right text at the top and bottom, with {page}, {pages}, {page:6} for Bates numbers and {date}, on chosen pages. Drawn into the pages, so changing them later is Undo or adding new ones |
| Watermarks and backgrounds | Later | Done: text watermark (colour, size or fit to page, angle, opacity, over or behind the content) and a page colour, on chosen pages. Picture watermarks not yet |

## 5. Editing content

| Feature | Priority | Notes |
|---|---|---|
| Edit existing text (reflow within a text box) | Later | Hardest feature; font subsetting issues |
| Add text, images, links | Later | Done: "Add text, pictures or links to pages" in All tools. Text (Helvetica, size and colour) and PNG or JPEG pictures become part of the page content; links go to a page or open a web or email address. Each addition is a page change, undone from the page tools |
| Replace / move / resize images | Done | All tools › Move, replace or delete pictures: pick a picture, drag it or its corner, or replace or delete it; undoable as a page change |
| Redaction (true content removal, search and redact) | Done | Text, pictures, inline images and enclosed drawings in marked areas are removed from the content (forms are copied and redacted too); comments over them are removed; drawings only partly inside stay, covered by the black box. Areas over form fields are refused until the form is flattened |
| Remove hidden information / sanitise document | Later | Done: document information and XMP metadata, scripts and automatic actions, attached files, and optionally comments. Scripts on form field parents (not widgets) are not yet removed |

## 6. Create and convert

| Feature | Priority | Notes |
|---|---|---|
| Images (PNG, JPEG, TIFF) → PDF | Later | Done for PNG and JPEG (JPEGs go in unchanged), a picture a page fitted to A4; TIFF not yet |
| PDF → images | Later | Done: PNG or JPEG per page at a chosen resolution |
| PDF → plain text | Later | Done: chosen pages into one text file |
| PDF → Word / Excel / PowerPoint | Later | Acrobat does this offline; quality is a large project |
| Word / Office → PDF | Skip | Needs Office or LibreOffice installed; could shell out later |
| Web page → PDF | Skip | Needs network |
| Scan to PDF (from scanner) | Later | OS scanner APIs |
| OCR (make scanned PDF searchable) | Later | Local OCR engine; the Tesseract C++ dependency would need sandboxing |
| Optimise / reduce file size | Later | Done: "Save a smaller copy…" in All tools writes a copy with uncompressed and lightly compressed streams compressed harder, identical streams (fonts, pictures) kept once, unused objects dropped, and small objects packed into compressed object streams. Pages look exactly the same; pictures are not yet downsampled |
| PDF/A, PDF/X conversion and preflight checks | Later | |

## 7. Security and signatures

| Feature | Priority | Notes |
|---|---|---|
| Open encrypted PDFs | V1 | Done (see above); changes save back into them encrypted |
| Encrypt with password, set permissions | Later | Done: AES-256 (revision 6) with an open password, a permissions password and print/change/copy limits; removing or changing it needs the owner password. Checked against poppler |
| Digitally sign with a local certificate (PKCS#12 / OS store) | Later | |
| Validate signatures | Later | Revocation checks (OCSP/CRL) are online, so they stay optional and off by default |
| Certify document | Later | |
| Adobe Sign, cloud ID | Skip | Online |

## 8. Accessibility

| Feature | Priority | Notes |
|---|---|---|
| Keyboard navigation for all actions | V1 | Done: the document actions have shortcuts shown in their tooltips (Ctrl+Shift+N goes to a page, Ctrl+Shift+C shows the comment tools, Ctrl+Shift+T changes the page layout, Home and End jump to the first and last page), and with the comment tools open a single letter picks each tool (V, H, U, K, N, T, P, R, E, A, L, G, C). Tab moves between form fields |
| Screen reader support for the app UI | Later | Depends on iced accessibility support |
| Tag PDF, reading order, accessibility checker | Later | |

## 9. Online-only (always Skip)

Adobe Document Cloud storage and sync, sharing links, shared review, Adobe Sign, the AI Assistant,
generative summaries, online export/convert services, Acrobat web, and any telemetry. **The app
makes no network connections.** CI enforces this by making sure no networking crates are in the
dependency tree.
