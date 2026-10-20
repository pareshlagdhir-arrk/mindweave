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

#[cfg(test)]
mod tests {
    use super::*;

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
