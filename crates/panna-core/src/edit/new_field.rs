//! Form fields made here: their widget dictionaries and what they show until filled in.

use super::annot::{literal, n, text_string};

/// The fonts new fields' appearances name, for the form's resources.
pub(crate) const RESOURCES: &str = "<< /Font << \
    /Helv << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> \
    /ZaDb << /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats >> >> >>";

/// Field flags (PDF 32000 table 226 and on).
const REQUIRED: u32 = 1 << 1;
const MULTILINE: u32 = 1 << 12;
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
        super::NewFieldKind::Dropdown(options) => {
            flags |= COMBO;
            let options: Vec<String> = options.iter().map(|o| string(o)).collect();
            extra.push_str(&format!(
                "/Opt [{}] /AP << /N {off} 0 R >> /DA (/Helv 0 Tf 0 g) ",
                options.join(" ")
            ));
            "/Ch"
        }
    };
    let tooltip = if field.tooltip.trim().is_empty() {
        String::new()
    } else {
        format!("/TU {} ", string(field.tooltip.trim()))
    };
    let mk = match &field.kind {
        super::NewFieldKind::CheckBox => "/MK << /BC [0.6 0.6 0.6] /BG [1 1 1] /CA (4) /R ROT >>",
        _ => "/MK << /BC [0.6 0.6 0.6] /BG [1 1 1] /R ROT >>",
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
