// TriProxy — a modern multi-core proxy client.
// Copyright (C) 2026 luoqingci
// SPDX-License-Identifier: GPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, either version 3 of the License, or (at your option) any later
// version. It is distributed in the hope that it will be useful, but WITHOUT ANY
// WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS FOR A
// PARTICULAR PURPOSE. See LICENSE for the full text.
//
// Additional terms under GNU GPL v3 section 7 apply: see ADDITIONAL-TERMS.md.

//! Small text codecs used by the URI and subscription parsers.
//!
//! Subscription payloads in the wild are inconsistent: base64 with or without
//! padding, standard or URL-safe alphabet, and sometimes with line breaks in the
//! middle. All of those are accepted rather than rejected.

use base64::engine::general_purpose::{
    GeneralPurpose, STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD,
};
use base64::Engine;

/// Decode base64 tolerantly.
///
/// Tries the four common alphabets/padding combinations and ignores embedded
/// whitespace, because subscriptions are frequently wrapped by the provider.
pub fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let compact: String = input.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return None;
    }

    let engines: [&GeneralPurpose; 4] = [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD];
    for engine in engines {
        if let Ok(bytes) = engine.decode(&compact) {
            return Some(bytes);
        }
    }
    None
}

/// Decode base64 into a `String`, rejecting non-UTF-8 payloads.
pub fn decode_base64_string(input: &str) -> Option<String> {
    let bytes = decode_base64(input)?;
    String::from_utf8(bytes).ok()
}

/// Percent-decode `%XX` sequences.
///
/// `+` is deliberately **not** treated as a space: in proxy URIs it appears
/// literally in passwords and base64 fragments far more often than as form
/// encoding, and every mainstream client keeps it literal.
pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_value(bytes[index + 1]);
            let low = hex_value(bytes[index + 2]);
            if let (Some(high), Some(low)) = (high, low) {
                out.push(high * 16 + low);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_all_four_base64_flavours() {
        // "hello" in each encoding
        assert_eq!(decode_base64_string("aGVsbG8=").as_deref(), Some("hello"));
        assert_eq!(decode_base64_string("aGVsbG8").as_deref(), Some("hello"));
        // URL-safe alphabet: bytes that are `+`/`/` in standard form.
        assert!(decode_base64("-_8=").is_some());
    }

    #[test]
    fn ignores_embedded_whitespace() {
        assert_eq!(decode_base64_string("aGVs\nbG8=").as_deref(), Some("hello"));
        assert_eq!(
            decode_base64_string("  aGVsbG8=  ").as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn rejects_non_base64() {
        assert!(decode_base64("not base64 at all!").is_none());
        assert!(decode_base64("").is_none());
    }

    #[test]
    fn percent_decoding_handles_utf8_and_keeps_plus_literal() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%E6%97%A5%E6%9C%AC"), "日本");
        assert_eq!(percent_decode("a+b"), "a+b");
        // A stray percent sign must not swallow the next characters.
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
    }
}
