//! Text decoding for the viewer: guessing a file's encoding and decoding it.

use encoding_rs::Encoding;

/// Encodings offered in the viewer's menu.
pub const ENCODINGS: &[&Encoding] = &[
    encoding_rs::UTF_8,
    encoding_rs::UTF_16LE,
    encoding_rs::UTF_16BE,
    encoding_rs::WINDOWS_1250,
    encoding_rs::WINDOWS_1251,
    encoding_rs::WINDOWS_1252,
    encoding_rs::ISO_8859_2,
    encoding_rs::ISO_8859_15,
    encoding_rs::KOI8_R,
    encoding_rs::SHIFT_JIS,
    encoding_rs::EUC_JP,
    encoding_rs::ISO_2022_JP,
    encoding_rs::GBK,
    encoding_rs::GB18030,
    encoding_rs::BIG5,
    encoding_rs::EUC_KR,
];

/// Guess the encoding of `bytes`: a byte-order mark wins, then valid UTF-8, then a UTF-16 pattern
/// of zero bytes, then a statistical guess (chardetng).
pub fn detect_encoding(bytes: &[u8]) -> &'static Encoding {
    if let Some((enc, _)) = Encoding::for_bom(bytes) {
        return enc;
    }
    // A file cut at an arbitrary length may end in the middle of a character.
    let head = &bytes[..bytes.len().min(1 << 20)];
    // Before UTF-8: ASCII-heavy UTF-16 is valid UTF-8 too (zero bytes are legal there).
    if let Some(enc) = utf16_without_bom(head) {
        return enc;
    }
    match std::str::from_utf8(head) {
        Ok(_) => return encoding_rs::UTF_8,
        Err(e) if e.error_len().is_none() && e.valid_up_to() + 4 > head.len() => return encoding_rs::UTF_8,
        Err(_) => {}
    }
    let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Allow);
    det.feed(head, head.len() == bytes.len());
    det.guess(None, chardetng::Utf8Detection::Allow)
}

/// Mostly-ASCII UTF-16 text has a zero in every other byte.
fn utf16_without_bom(bytes: &[u8]) -> Option<&'static Encoding> {
    let sample = &bytes[..bytes.len().min(4096) & !1];
    if sample.len() < 8 {
        return None;
    }
    let pairs = sample.len() / 2;
    let zero_even = sample.iter().step_by(2).filter(|b| **b == 0).count();
    let zero_odd = sample.iter().skip(1).step_by(2).filter(|b| **b == 0).count();
    if zero_odd * 10 > pairs * 7 && zero_even * 10 < pairs {
        Some(encoding_rs::UTF_16LE)
    } else if zero_even * 10 > pairs * 7 && zero_odd * 10 < pairs {
        Some(encoding_rs::UTF_16BE)
    } else {
        None
    }
}

/// Whether the bytes look like a binary file rather than text (zero bytes outside UTF-16).
pub fn looks_binary(bytes: &[u8], encoding: &'static Encoding) -> bool {
    if encoding == encoding_rs::UTF_16LE || encoding == encoding_rs::UTF_16BE {
        return false;
    }
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// Decode with `encoding` (a matching byte-order mark is removed).
pub fn decode(bytes: &[u8], encoding: &'static Encoding) -> String {
    encoding.decode_with_bom_removal(bytes).0.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection() {
        assert_eq!(detect_encoding("zażółć gęślą jaźń".as_bytes()), encoding_rs::UTF_8);
        assert_eq!(detect_encoding(b"\xEF\xBB\xBFbom"), encoding_rs::UTF_8);
        assert_eq!(detect_encoding(b"\xFF\xFEh\0i\0"), encoding_rs::UTF_16LE);
        let utf16: Vec<u8> = "hello world, plain".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(detect_encoding(&utf16), encoding_rs::UTF_16LE);
        let (sjis, _, _) = encoding_rs::SHIFT_JIS.encode("日本語のテキストファイルです。これはテストです。");
        assert_eq!(detect_encoding(&sjis), encoding_rs::SHIFT_JIS);
        let (cp1250, _, _) = encoding_rs::WINDOWS_1250.encode("Zażółć gęślą jaźń, to jest polski tekst w kodowaniu Windows.");
        let guess = detect_encoding(&cp1250);
        assert_eq!(decode(&cp1250, guess), "Zażółć gęślą jaźń, to jest polski tekst w kodowaniu Windows.");
        // Truncated in the middle of a multi-byte character: still UTF-8.
        let s = "abcż".as_bytes();
        assert_eq!(detect_encoding(&s[..s.len() - 1]), encoding_rs::UTF_8);
    }

    #[test]
    fn binary() {
        assert!(looks_binary(b"MZ\x90\0\x03\0", encoding_rs::UTF_8));
        assert!(!looks_binary(b"plain text", encoding_rs::UTF_8));
    }
}
