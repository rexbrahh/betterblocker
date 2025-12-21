//! Fast URL parsing utilities for the hot path
//!
//! These functions avoid allocations and work directly on string slices.

use crate::types::SchemeMask;
use crate::hash::hash_token;

// =============================================================================
// Scheme Extraction
// =============================================================================

/// Fast scheme extraction without URL parsing.
/// Returns the scheme mask or None if unknown.
#[inline]
pub fn extract_scheme(url: &str) -> Option<SchemeMask> {
    let bytes = url.as_bytes();
    if bytes.len() < 5 {
        return None;
    }

    // Lowercase first char
    let c0 = bytes[0] | 0x20;

    match c0 {
        b'h' => {
            if bytes.len() >= 8 && bytes[..8].eq_ignore_ascii_case(b"https://") {
                Some(SchemeMask::HTTPS)
            } else if bytes.len() >= 7 && bytes[..7].eq_ignore_ascii_case(b"http://") {
                Some(SchemeMask::HTTP)
            } else {
                None
            }
        }
        b'w' => {
            if bytes.len() >= 6 && bytes[..6].eq_ignore_ascii_case(b"wss://") {
                Some(SchemeMask::WSS)
            } else if bytes.len() >= 5 && bytes[..5].eq_ignore_ascii_case(b"ws://") {
                Some(SchemeMask::WS)
            } else {
                None
            }
        }
        b'd' => {
            if bytes.len() >= 5 && bytes[..5].eq_ignore_ascii_case(b"data:") {
                Some(SchemeMask::DATA)
            } else {
                None
            }
        }
        b'f' => {
            if bytes.len() >= 6 && bytes[..6].eq_ignore_ascii_case(b"ftp://") {
                Some(SchemeMask::FTP)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Get the position after "://".
#[inline]
pub fn get_scheme_end(url: &str) -> Option<usize> {
    let bytes = url.as_bytes();
    
    // Find ':'
    let colon_pos = bytes.iter().position(|&b| b == b':')?;
    
    // Check for "://"
    if bytes.len() > colon_pos + 2 
        && bytes[colon_pos + 1] == b'/'
        && bytes[colon_pos + 2] == b'/'
    {
        return Some(colon_pos + 3);
    }

    // Data URLs use ":" not "://"
    if colon_pos >= 4 && bytes[..colon_pos].eq_ignore_ascii_case(b"data") {
        return Some(colon_pos + 1);
    }

    None
}

// =============================================================================
// Host Extraction
// =============================================================================

/// Fast host extraction without allocations.
/// Returns a slice into the original URL.
#[inline]
pub fn extract_host(url: &str) -> Option<&str> {
    let (host_start, host_end) = get_host_position(url)?;
    Some(&url[host_start..host_end])
}

/// Extract host with port if present.
#[inline]
pub fn extract_host_with_port(url: &str) -> Option<&str> {
    let scheme_end = get_scheme_end(url)?;
    let bytes = url.as_bytes();

    // Find host end (first of: '/', '?', '#', or end of string)
    let mut host_end = bytes.len();
    for (i, &b) in bytes[scheme_end..].iter().enumerate() {
        if b == b'/' || b == b'?' || b == b'#' {
            host_end = scheme_end + i;
            break;
        }
    }

    let host_with_port = &url[scheme_end..host_end];

    // Handle userinfo
    if let Some(at_pos) = host_with_port.find('@') {
        Some(&host_with_port[at_pos + 1..])
    } else {
        Some(host_with_port)
    }
}

// =============================================================================
// Path Extraction
// =============================================================================

/// Extract the path portion of a URL.
#[inline]
pub fn extract_path(url: &str) -> &str {
    let scheme_end = match get_scheme_end(url) {
        Some(pos) => pos,
        None => return "/",
    };

    let bytes = url.as_bytes();

    // Find path start (first '/' after host)
    let mut path_start = None;
    for (i, &b) in bytes[scheme_end..].iter().enumerate() {
        if b == b'/' {
            path_start = Some(scheme_end + i);
            break;
        }
        if b == b'?' || b == b'#' {
            return "/";
        }
    }

    let path_start = match path_start {
        Some(pos) => pos,
        None => return "/",
    };

    // Find path end
    let mut path_end = bytes.len();
    for (i, &b) in bytes[path_start..].iter().enumerate() {
        if b == b'?' || b == b'#' {
            path_end = path_start + i;
            break;
        }
    }

    &url[path_start..path_end]
}

// =============================================================================
// URL Tokenization
// =============================================================================

const MIN_TOKEN_LEN: usize = 3;
const MAX_TOKENS: usize = 32;

/// Check if a byte is alphanumeric.
#[inline]
fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

/// Token extracted from a URL.
#[derive(Debug, Clone, Copy)]
pub struct UrlToken {
    pub hash: u32,
    pub start: usize,
    pub len: usize,
}

/// Tokenize a URL for pattern matching.
/// Returns hashed tokens.
pub fn tokenize_url(url: &str) -> Vec<u32> {
    let mut tokens = Vec::with_capacity(MAX_TOKENS);
    let bytes = url.as_bytes();
    
    // Start after scheme
    let start = get_scheme_end(url).unwrap_or(0);
    
    let mut token_start = None;
    
    for i in start..=bytes.len() {
        let is_alpha = i < bytes.len() && is_alnum(bytes[i]);
        
        if is_alpha {
            if token_start.is_none() {
                token_start = Some(i);
            }
        } else if let Some(ts) = token_start {
            let len = i - ts;
            if len >= MIN_TOKEN_LEN && tokens.len() < MAX_TOKENS {
                // Hash the lowercased token
                let token_bytes: Vec<u8> = bytes[ts..i]
                    .iter()
                    .map(|b| b.to_ascii_lowercase())
                    .collect();
                let token_str = unsafe { std::str::from_utf8_unchecked(&token_bytes) };
                tokens.push(hash_token(token_str));
            }
            token_start = None;
        }
    }
    
    tokens
}

/// Tokenize URL into token structs with position info.
pub fn tokenize_url_with_positions(url: &str) -> Vec<UrlToken> {
    let mut tokens = Vec::with_capacity(MAX_TOKENS);
    let bytes = url.as_bytes();
    
    let start = get_scheme_end(url).unwrap_or(0);
    let mut token_start = None;
    
    for i in start..=bytes.len() {
        let is_alpha = i < bytes.len() && is_alnum(bytes[i]);
        
        if is_alpha {
            if token_start.is_none() {
                token_start = Some(i);
            }
        } else if let Some(ts) = token_start {
            let len = i - ts;
            if len >= MIN_TOKEN_LEN && tokens.len() < MAX_TOKENS {
                let token_bytes: Vec<u8> = bytes[ts..i]
                    .iter()
                    .map(|b| b.to_ascii_lowercase())
                    .collect();
                let token_str = unsafe { std::str::from_utf8_unchecked(&token_bytes) };
                tokens.push(UrlToken {
                    hash: hash_token(token_str),
                    start: ts,
                    len,
                });
            }
            token_start = None;
        }
    }
    
    tokens
}

// =============================================================================
// ABP Boundary Check
// =============================================================================

/// Check if a character is an ABP separator (boundary character).
/// ABP ^ matches: end of string, or any non-alphanumeric non-% character.
#[inline]
pub fn is_boundary_char(c: u8) -> bool {
    if c == 0 {
        return true;
    }
    if is_alnum(c) {
        return false;
    }
    // % is not a boundary (URL encoding)
    if c == b'%' {
        return false;
    }
    true
}

/// Check if position in string is at a boundary.
#[inline]
pub fn is_at_boundary(s: &str, pos: usize) -> bool {
    if pos >= s.len() {
        return true;
    }
    is_boundary_char(s.as_bytes()[pos])
}

// =============================================================================
// Query Parameter Handling
// =============================================================================

/// Remove specified parameters from a URL.
/// Returns the modified URL, or None if no changes.
#[cfg(feature = "std")]
pub fn remove_query_params(url: &str, keys_to_remove: &std::collections::HashSet<&str>) -> Option<String> {
    let q_pos = url.find('?')?;
    
    // Find fragment
    let (query_part, fragment) = match url[q_pos + 1..].find('#') {
        Some(hash_pos) => {
            let abs_hash = q_pos + 1 + hash_pos;
            (&url[q_pos + 1..abs_hash], Some(&url[abs_hash..]))
        }
        None => (&url[q_pos + 1..], None),
    };
    
    if query_part.is_empty() {
        return None;
    }
    
    let mut kept = Vec::new();
    let mut changed = false;
    
    for pair in query_part.split('&') {
        let key = match pair.find('=') {
            Some(eq_pos) => &pair[..eq_pos],
            None => pair,
        };
        
        if keys_to_remove.contains(key) {
            changed = true;
        } else {
            kept.push(pair);
        }
    }
    
    if !changed {
        return None;
    }
    
    let base = &url[..q_pos];
    if kept.is_empty() {
        Some(match fragment {
            Some(f) => format!("{}{}", base, f),
            None => base.to_string(),
        })
    } else {
        Some(match fragment {
            Some(f) => format!("{}?{}{}", base, kept.join("&"), f),
            None => format!("{}?{}", base, kept.join("&")),
        })
    }
}

// =============================================================================
// Host Position
// =============================================================================

/// Get the start and end positions of the hostname in a URL.
#[inline]
pub fn get_host_position(url: &str) -> Option<(usize, usize)> {
    let scheme_end = get_scheme_end(url)?;
    let bytes = url.as_bytes();

    // Skip userinfo
    let mut host_start = scheme_end;
    for i in scheme_end..bytes.len() {
        if bytes[i] == b'@' {
            host_start = i + 1;
            break;
        }
        if bytes[i] == b'/' {
            break;
        }
    }

    // Find host end
    let mut host_end = bytes.len();
    for i in host_start..bytes.len() {
        let b = bytes[i];
        if b == b'/' || b == b'?' || b == b'#' || b == b':' {
            host_end = i;
            break;
        }
    }

    Some((host_start, host_end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_scheme() {
        assert_eq!(extract_scheme("https://example.com"), Some(SchemeMask::HTTPS));
        assert_eq!(extract_scheme("http://example.com"), Some(SchemeMask::HTTP));
        assert_eq!(extract_scheme("wss://example.com"), Some(SchemeMask::WSS));
        assert_eq!(extract_scheme("ws://example.com"), Some(SchemeMask::WS));
        assert_eq!(extract_scheme("data:text/html"), Some(SchemeMask::DATA));
        assert_eq!(extract_scheme("ftp://example.com"), Some(SchemeMask::FTP));
        assert_eq!(extract_scheme("invalid"), None);
    }

    #[test]
    fn test_extract_host() {
        assert_eq!(extract_host("https://example.com/path"), Some("example.com"));
        assert_eq!(extract_host("https://example.com:8080/path"), Some("example.com"));
        assert_eq!(extract_host("https://user:pass@example.com/path"), Some("example.com"));
        assert_eq!(extract_host("https://sub.example.com"), Some("sub.example.com"));
    }

    #[test]
    fn test_extract_path() {
        assert_eq!(extract_path("https://example.com/path/to/file"), "/path/to/file");
        assert_eq!(extract_path("https://example.com/"), "/");
        assert_eq!(extract_path("https://example.com"), "/");
        assert_eq!(extract_path("https://example.com?query"), "/");
    }

    #[test]
    fn test_tokenize_url() {
        let tokens = tokenize_url("https://example.com/path/analytics.js");
        assert!(!tokens.is_empty());
        // Should contain tokens for "example", "com", "path", "analytics"
    }

    #[test]
    fn test_is_boundary() {
        assert!(is_at_boundary("abc", 3)); // End of string
        assert!(is_at_boundary("abc/def", 3)); // At '/'
        assert!(!is_at_boundary("abc", 1)); // At 'b'
    }

    #[test]
    fn test_get_host_position() {
        let pos = get_host_position("https://example.com/path");
        assert_eq!(pos, Some((8, 19)));
    }
}
