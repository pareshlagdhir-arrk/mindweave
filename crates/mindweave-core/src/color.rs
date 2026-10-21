//! `#RRGGBB` colour strings, as stored in canvas settings.

/// Parses six hex digits, with or without a leading `#`, in any case.
/// Surrounding whitespace is ignored.
pub fn parse_hex_color(text: &str) -> Option<(u8, u8, u8)> {
    let text = text.trim();
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).ok();
    Some((channel(0)?, channel(2)?, channel(4)?))
}

/// `rgb` as an uppercase `#RRGGBB` string.
pub fn format_hex_color((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02X}{g:02X}{b:02X}")
}

/// `text` as an uppercase `#RRGGBB` string, or `None` when it is not a
/// colour [`parse_hex_color`] accepts.
pub fn normalize_hex_color(text: &str) -> Option<String> {
    parse_hex_color(text).map(format_hex_color)
}

/// WCAG relative luminance of an sRGB colour, 0 (black) to 1 (white).
pub fn relative_luminance((r, g, b): (u8, u8, u8)) -> f32 {
    let linear = |c: u8| {
        let c = c as f32 / 255.0;
        if c <= 0.03928 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// Black or white, whichever has the higher WCAG contrast ratio against
/// `background`.
pub fn auto_text_color(background: (u8, u8, u8)) -> (u8, u8, u8) {
    let l = relative_luminance(background);
    // Contrast with white is 1.05 / (l + 0.05); with black (l + 0.05) / 0.05.
    let with_white = 1.05 / (l + 0.05);
    let with_black = (l + 0.05) / 0.05;
    if with_white > with_black {
        (255, 255, 255)
    } else {
        (0, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_on_dark_black_on_light() {
        let bg = |s: &str| parse_hex_color(s).unwrap();
        assert_eq!(auto_text_color(bg("#1E1E1E")), (255, 255, 255));
        assert_eq!(auto_text_color(bg("#F5F5F5")), (0, 0, 0));
        let mid = bg("#777777");
        let l = relative_luminance(mid);
        let (white, black) = (1.05 / (l + 0.05), (l + 0.05) / 0.05);
        let expected = if white > black { 255 } else { 0 };
        assert_eq!(auto_text_color(mid), (expected, expected, expected));
        assert_eq!(relative_luminance((0, 0, 0)), 0.0);
        assert!((relative_luminance((255, 255, 255)) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn parses_six_hex_digits_with_or_without_hash() {
        assert_eq!(parse_hex_color("#1A2b3C"), Some((0x1A, 0x2B, 0x3C)));
        assert_eq!(parse_hex_color("ff8000"), Some((255, 128, 0)));
        assert_eq!(parse_hex_color("  #000000 "), Some((0, 0, 0)));
    }

    #[test]
    fn rejects_anything_else() {
        for bad in [
            "", "#", "#fff", "#12345", "#1234567", "#12345g", "##123456", "red",
        ] {
            assert_eq!(parse_hex_color(bad), None, "{bad:?}");
        }
        // Multi-byte characters must not panic on byte slicing.
        assert_eq!(parse_hex_color("#ééé"), None);
    }

    #[test]
    fn formats_uppercase_with_hash() {
        assert_eq!(format_hex_color((0x1a, 0x2b, 0x3c)), "#1A2B3C");
        assert_eq!(format_hex_color((0, 0, 0)), "#000000");
    }

    #[test]
    fn normalizes_to_uppercase_with_hash() {
        assert_eq!(normalize_hex_color("ff8000").as_deref(), Some("#FF8000"));
        assert_eq!(normalize_hex_color("#abcdef").as_deref(), Some("#ABCDEF"));
        assert_eq!(normalize_hex_color("nope"), None);
    }
}
