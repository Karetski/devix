//! `file://` URI codec. LSP servers identify documents by URI; we round-
//! trip them through paths to attach buffers, diagnostics, and locations.
//!
//! `lsp_types::Uri::from_str` is strict: it rejects raw spaces, `#`, `?`,
//! and non-ASCII bytes. This module owns the percent-encoding required to
//! get arbitrary filesystem paths through that gate. Encode/decode are
//! exact inverses for paths we built ourselves.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use lsp_types::Uri;

/// String key used to address documents in the coordinator's docs map.
/// Slice-1 simplification: `Uri::to_string()`. Round-trip stable for URIs
/// we produced via [`path_to_uri`]; not normalized against arbitrary
/// inputs.
pub fn uri_key(uri: &Uri) -> String {
    uri.as_str().to_string()
}

pub fn path_to_uri(path: &Path) -> Result<Uri> {
    use std::str::FromStr;
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.to_string_lossy();
    let encoded = percent_encode_path(&s);
    let normalized = if s.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{}", encoded.replace('\\', "/"))
    };
    Uri::from_str(&normalized).with_context(|| format!("building Uri from {normalized:?}"))
}

pub fn uri_to_path(uri: &Uri) -> Result<PathBuf> {
    let s = uri.as_str();
    let stripped = s
        .strip_prefix("file://")
        .ok_or_else(|| anyhow!("non-file URI: {s}"))?;
    let decoded = percent_decode(stripped);
    // POSIX: file:///foo → /foo. Windows: file:///C:/foo → C:/foo.
    let stripped = decoded.strip_prefix('/').unwrap_or(&decoded);
    if stripped.starts_with(|c: char| c.is_ascii_alphabetic())
        && stripped.chars().nth(1) == Some(':')
    {
        Ok(PathBuf::from(stripped))
    } else {
        Ok(PathBuf::from(format!("/{stripped}")))
    }
}

/// Percent-encode a path string for use in a `file://` URI. Encodes every
/// byte that's not in the URI "unreserved" set (`A-Z a-z 0-9 - . _ ~`) or
/// a path-safe punctuation char (`/ : \\`). Without this, paths containing
/// spaces, `#`, `?`, or non-ASCII produce URIs that `Uri::from_str` rejects
/// — silently breaking LSP attach for the affected document.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let safe = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/' | b':' | b'\\');
        if safe {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(hex_nibble(b >> 4));
            out.push(hex_nibble(b & 0x0f));
        }
    }
    out
}

fn hex_nibble(n: u8) -> char {
    match n {
        0..=9 => (b'0' + n) as char,
        10..=15 => (b'A' + (n - 10)) as char,
        _ => '0',
    }
}

/// Reverse of [`percent_encode_path`]. Decodes `%xx` byte triplets back into
/// the original UTF-8 string. Malformed escapes are passed through verbatim
/// so we never panic on a hostile or odd server-supplied URI.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    // Path bytes are UTF-8 (or near enough); fall back to lossy on invalid
    // sequences so a single bad byte doesn't drop the whole URI on the floor.
    String::from_utf8(out)
        .unwrap_or_else(|e| String::from_utf8_lossy(&e.into_bytes()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_to_uri_round_trips_through_uri_to_path_with_spaces() {
        // Build a path under tmp with spaces; round-tripping should produce
        // the same path back. Pre-fix, building the URI failed outright
        // (Uri::from_str rejects raw spaces).
        let dir = std::env::temp_dir().join(format!("devix uri test {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("file with spaces.rs");
        std::fs::write(&p, "fn main(){}").unwrap();
        let u = path_to_uri(&p).expect("should encode spaces");
        assert!(!u.as_str().contains(' '), "raw space leaked into URI: {}", u.as_str());
        assert!(u.as_str().contains("%20"), "missing percent-encoded space: {}", u.as_str());
        let back = uri_to_path(&u).expect("decode");
        assert_eq!(std::fs::canonicalize(&back).unwrap(), std::fs::canonicalize(&p).unwrap());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn percent_decode_passes_through_malformed_escape() {
        // % followed by non-hex must not crash and should be passed through.
        assert_eq!(percent_decode("a%2Gb"), "a%2Gb");
        assert_eq!(percent_decode("%"), "%");
        assert_eq!(percent_decode("a%20b"), "a b");
    }
}
