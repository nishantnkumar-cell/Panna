//! Form fields made here: their widget dictionaries and what they show until filled in.

use super::annot::{literal, n, text_string};

/// The fonts new fields' appearances name, for the form's resources.
pub(crate) const RESOURCES: &str = "<< /Font << \
    /Helv << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> \
    /ZaDb << /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >> >> >>";

/// Field flags (PDF 32000 table 226 and on).
const REQUIRED: u32 = 1 << 1;
const MULTILINE: u32 = 1 << 12;
const NO_TOGGLE_TO_OFF: u32 = 1 << 14;
const RADIO: u32 = 1 << 15;
const PUSH_BUTTON: u32 = 1 << 16;
const COMBO: u32 = 1 << 17;

/// A string in PDF syntax: a literal for plain ASCII, UTF-16 otherwise.
fn string(s: &str) -> String {
    if s.bytes().all(|b| (0x20..0x7f).contains(&b)) {
        format!("({})", literal(s.as_bytes()))
    } else {
        text_string(s)
    }
}

fn form_stream(width: f64, height: f64, data: &str) -> Vec<u8> {
    format!(
        "<< /Type /XObject /Subtype /Form /BBox [0 0 {} {}] /Resources {RESOURCES} /Length {} >>\nstream\n{data}\nendstream",
        n(width),
        n(height),
        data.len()
    )
    .into_bytes()
}

/// A thin grey border inside a `width` by `height` box, on white.
fn border(width: f64, height: f64) -> String {
    format!(
        "1 g 0 0 {w} {h} re f 0.6 G 1 w 0.5 0.5 {iw} {ih} re S",
        w = n(width),
        h = n(height),
        iw = n((width - 1.0).max(0.0)),
        ih = n((height - 1.0).max(0.0))
    )
}

/// The widget dictionary (field and widget in one) for `field` at `rect` (in the page's own
/// space) on page object `page`, storing the appearance streams it needs with `store`.
pub(crate) fn widget(
    field: &super::NewField,
    rect: [f64; 4],
    page: u32,
    pages: &[u32],
    rotate: i64,
    store: &mut dyn FnMut(Vec<u8>) -> u32,
) -> Vec<u8> {
    let [x0, y0, x1, y1] = rect;
    // The appearance is drawn upright on the page as shown; /MK /R turns it with the page.
    let (width, height) = if rotate % 180 == 0 {
        (x1 - x0, y1 - y0)
    } else {
        (y1 - y0, x1 - x0)
    };
    let mut flags = if field.required { REQUIRED } else { 0 };
    let mut extra = String::new();
    let off = store(form_stream(width, height, &border(width, height)));
    let kind = match &field.kind {
        super::NewFieldKind::Text { multiline } => {
            if *multiline {
                flags |= MULTILINE;
            }
            extra.push_str(&format!("/AP << /N {off} 0 R >> /DA (/Helv 0 Tf 0 g) "));
            "/Tx"
        }
        super::NewFieldKind::Date(pattern) => {
            // Only the pattern's letters and separators, so it can't break out of the script.
            let pattern: String = pattern
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || " /.-:,".contains(*c))
                .take(40)
                .collect();
            let pattern = if pattern.is_empty() {
                "dd/mm/yyyy".to_owned()
            } else {
                pattern
            };
            extra.push_str(&format!(
                "/AP << /N {off} 0 R >> /DA (/Helv 0 Tf 0 g) /AA << \
                 /F << /S /JavaScript /JS (AFDate_FormatEx\\(\"{pattern}\"\\);) >> \
                 /K << /S /JavaScript /JS (AFDate_KeystrokeEx\\(\"{pattern}\"\\);) >> >> "
            ));
            "/Tx"
        }
        super::NewFieldKind::CheckBox => {
            let size = (height.min(width) * 0.8).max(1.0);
            let check = format!(
                "{} 0 g BT /ZaDb {} Tf {} {} Td (4) Tj ET",
                border(width, height),
                n(size),
                n((width - size * 0.75) / 2.0),
                n((height - size * 0.7) / 2.0)
            );
            let on = store(form_stream(width, height, &check));
            extra.push_str(&format!(
                "/AP << /N << /Yes {on} 0 R /Off {off} 0 R >> >> /AS /Off /V /Off /DA (/ZaDb 0 Tf 0 g) "
            ));
            "/Btn"
        }
        super::NewFieldKind::Dropdown(options) | super::NewFieldKind::List(options) => {
            // A list box shows several rows, so its type has a set size (Acrobat's 12 pt)
            // rather than one fitted to the box.
            let size = if matches!(field.kind, super::NewFieldKind::Dropdown(_)) {
                flags |= COMBO;
                0
            } else {
                12
            };
            let options: Vec<String> = options.iter().map(|o| string(o)).collect();
            extra.push_str(&format!(
                "/Opt [{}] /AP << /N {off} 0 R >> /DA (/Helv {size} Tf 0 g) ",
                options.join(" ")
            ));
            "/Ch"
        }
        // Radio groups are written by `radio_group` and `radio_widget`.
        super::NewFieldKind::Radio(_) => "/Btn",
        super::NewFieldKind::Button { caption, action } => {
            flags |= PUSH_BUTTON;
            let face = store(form_stream(width, height, &button_face(width, height, caption)));
            extra.push_str(&format!("/AP << /N {face} 0 R >> /DA (/Helv 0 Tf 0 g) "));
            if let Some(a) = action_dict(*action, pages) {
                extra.push_str(&a);
            }
            "/Btn"
        }
        super::NewFieldKind::Image => {
            // As Acrobat makes them: a button showing only its picture, which a script
            // asks for when it's clicked (Panna asks without running it).
            flags |= PUSH_BUTTON;
            extra.push_str(&format!(
                "/AP << /N {off} 0 R >> /DA (/Helv 0 Tf 0 g) /AA << /U << /S /JavaScript \
                 /JS (event.target.buttonImportIcon\\(\\);) >> >> "
            ));
            "/Btn"
        }
        super::NewFieldKind::Signature => {
            let face = store(form_stream(width, height, &signature_face(width, height)));
            extra.push_str(&format!("/AP << /N {face} 0 R >> "));
            "/Sig"
        }
    };
    let tooltip = tooltip(field);
    let mk = match &field.kind {
        super::NewFieldKind::CheckBox => "/MK << /BC [0.6 0.6 0.6] /BG [1 1 1] /CA (4) /R ROT >>".to_owned(),
        super::NewFieldKind::Button { caption, .. } => {
            format!(
                "/MK << /BC [0.5 0.5 0.5] /BG [0.85 0.85 0.85] /CA {} /R ROT >>",
                string(caption)
            )
        }
        // Icon only, scaled to fit (Acrobat's image field layout).
        super::NewFieldKind::Image => {
            "/MK << /BC [0.6 0.6 0.6] /BG [1 1 1] /TP 1 /IF << /SW /A /S /P >> /R ROT >>".to_owned()
        }
        _ => "/MK << /BC [0.6 0.6 0.6] /BG [1 1 1] /R ROT >>".to_owned(),
    }
    .replace("ROT", &rotate.to_string());
    format!(
        "<< /Type /Annot /Subtype /Widget /FT {kind} /T {} {tooltip}/Ff {flags} /Rect [{} {} {} {}] /F 4 /P {page} 0 R {mk} {extra}>>",
        string(&field.name),
        n(x0),
        n(y0),
        n(x1),
        n(y1),
    )
    .into_bytes()
}

/// A raised grey button with `caption` centred on it.
fn button_face(width: f64, height: f64, caption: &str) -> String {
    let caption = caption.replace(['\r', '\n'], " ");
    let size = ((height - 4.0) * 0.6)
        .min((width - 6.0) / super::annot::text_width(&caption).max(0.01))
        .clamp(1.0, 12.0);
    let x = (width - super::annot::text_width(&caption) * size) / 2.0;
    format!(
        "0.85 g 0 0 {w} {h} re f 1 g 1 1 m 1 {h1} l {w1} {h1} l {w2} {h2} l 2 {h2} l 2 2 l f \
         0.6 g {w1} {h1} m {w1} 1 l 1 1 l 2 2 l {w2} 2 l {w2} {h2} l f \
         0.5 G 1 w 0.5 0.5 {iw} {ih} re S \
         BT /Helv {size} Tf 0 g {x} {y} Td ({text}) Tj ET",
        w = n(width),
        h = n(height),
        w1 = n(width - 1.0),
        h1 = n(height - 1.0),
        w2 = n(width - 2.0),
        h2 = n(height - 2.0),
        iw = n((width - 1.0).max(0.0)),
        ih = n((height - 1.0).max(0.0)),
        size = n(size),
        x = n(x.max(2.0)),
        y = n((height - size * 0.7) / 2.0),
        text = literal(&super::annot::win_ansi(&caption)),
    )
}

/// An empty signature field: a pale box with a line to sign on.
fn signature_face(width: f64, height: f64) -> String {
    let size = (height * 0.25).clamp(4.0, 9.0);
    format!(
        "{} 0.55 G 0.75 w 6 {line} m {right} {line} l S \
         BT /Helv {} Tf 0.45 g 6 {} Td (Sign here) Tj ET",
        border(width, height),
        n(size),
        n(height * 0.3 + 2.0),
        line = n(height * 0.3),
        right = n((width - 6.0).max(6.0)),
    )
}

/// The /A entry doing `action`, with pages named by their objects in `pages`.
fn action_dict(action: crate::ButtonAction, pages: &[u32]) -> Option<String> {
    use crate::ButtonAction as A;
    let named = |n: &str| Some(format!("/A << /S /Named /N /{n} >> "));
    match action {
        A::Nothing => None,
        A::ResetForm => Some("/A << /S /ResetForm >> ".into()),
        A::GoTo(page) => pages
            .get(page)
            .map(|num| format!("/A << /S /GoTo /D [{num} 0 R /Fit] >> ")),
        A::Print => named("Print"),
        A::NextPage => named("NextPage"),
        A::PreviousPage => named("PrevPage"),
        A::FirstPage => named("FirstPage"),
        A::LastPage => named("LastPage"),
    }
}

fn tooltip(field: &super::NewField) -> String {
    if field.tooltip.trim().is_empty() {
        String::new()
    } else {
        format!("/TU {} ", string(field.tooltip.trim()))
    }
}

/// A circle of radius `r` around (`x`, `y`), as a path.
fn circle(x: f64, y: f64, r: f64) -> String {
    // Four Bézier quarters; k puts the control points so each is close to a true arc.
    let k = r * 0.552_284_75;
    format!(
        "{x1} {y} m {x1} {yk} {xk} {y1} {x} {y1} c {xm} {y1} {x0} {yk} {x0} {y} c {x0} {ym} {xm} {y0} {x} {y0} c {xk} {y0} {x1} {ym} {x1} {y} c",
        x = n(x),
        y = n(y),
        x0 = n(x - r),
        x1 = n(x + r),
        y0 = n(y - r),
        y1 = n(y + r),
        xk = n(x + k),
        xm = n(x - k),
        yk = n(y + k),
        ym = n(y - k),
    )
}

/// The field dictionary of a radio group whose buttons are the widgets `kids`.
pub(crate) fn radio_group(field: &super::NewField, kids: &[u32]) -> Vec<u8> {
    let mut flags = RADIO | NO_TOGGLE_TO_OFF;
    if field.required {
        flags |= REQUIRED;
    }
    let kids: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
    format!(
        "<< /FT /Btn /T {} {}/Ff {flags} /V /Off /DA (/ZaDb 0 Tf 0 g) /Kids [{}] >>",
        string(&field.name),
        tooltip(field),
        kids.join(" ")
    )
    .into_bytes()
}

/// The widget for one button of the radio group `parent`, at `rect` (in the page's own
/// space) on page object `page`, storing its appearances with `store`.
pub(crate) fn radio_widget(
    button: &super::RadioButton,
    rect: [f64; 4],
    parent: u32,
    page: u32,
    rotate: i64,
    store: &mut dyn FnMut(Vec<u8>) -> u32,
) -> Vec<u8> {
    let [x0, y0, x1, y1] = rect;
    let (width, height) = if rotate % 180 == 0 {
        (x1 - x0, y1 - y0)
    } else {
        (y1 - y0, x1 - x0)
    };
    let (cx, cy) = (width / 2.0, height / 2.0);
    let r = (width.min(height) / 2.0 - 0.5).max(0.5);
    let ring = format!("1 g 0.6 G 1 w {} B", circle(cx, cy, r));
    let off = store(form_stream(width, height, &ring));
    let dot = format!("{ring} 0 g {} f", circle(cx, cy, (r * 0.45).max(0.5)));
    let on = store(form_stream(width, height, &dot));
    let state = super::field::name(&button.choice);
    format!(
        "<< /Type /Annot /Subtype /Widget /Parent {parent} 0 R /Rect [{} {} {} {}] /F 4 /P {page} 0 R \
         /MK << /BC [0.6 0.6 0.6] /BG [1 1 1] /CA (l) /R {rotate} >> \
         /AP << /N << {state} {on} 0 R /Off {off} 0 R >> >> /AS /Off >>",
        n(x0),
        n(y0),
        n(x1),
        n(y1),
    )
    .into_bytes()
}
