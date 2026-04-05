use std::collections::HashSet;
use std::path::Path;

use crate::tokenizer::{normalize, tokenize, NOISE_TOKENS};

/// Read file content and return a set of word tokens (length >= 2)
pub fn content_tokens(path: &Path) -> HashSet<String> {
    let Ok(bytes) = std::fs::read(path) else { return HashSet::new() };
    let text = String::from_utf8_lossy(&bytes);
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Jaccard similarity between file contents using word tokens
pub fn content_similarity(a: &Path, b: &Path) -> f64 {
    let a_tokens = content_tokens(a);
    let b_tokens = content_tokens(b);
    if a_tokens.is_empty() && b_tokens.is_empty() {
        return 0.0;
    }
    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.union(&b_tokens).count();
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}

/// Try to parse a quoted string as an internal URI path.
/// Returns path tokens if the string is a plausible internal path, None otherwise.
///
/// Handles three styles of internal path reference:
///
/// 1. **Slash paths** (`/application/search`, `../views/user`)
///    - Standard URL/filesystem paths
///    - Query string and fragment are stripped
///
/// 2. **Backslash paths** (`App\Http\Controllers\HomeController`)
///    - PHP namespace / Windows-style paths
///    - Trailing `.php` extension is stripped
///
/// 3. **Dot-notation** (`detail.index`, `application.search.index`)
///    - Laravel-style view names, ZF2 route names, etc.
///    - Must be ≥2 segments of word-chars, no spaces, not version numbers
///
/// Filters out external schemes (http/https/mailto/tel/data/javascript/`//`/#).
fn parse_uri_tokens(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    if s.is_empty() { return None; }

    // Skip external and non-path schemes
    for prefix in &["http://", "https://", "//", "mailto:", "tel:", "javascript:", "data:", "#"] {
        if s.starts_with(prefix) { return None; }
    }

    let tokens: Vec<String> = if s.contains('/') {
        // --- Style 1: slash-separated path ---
        let path = s.split('?').next().unwrap_or(s);
        let path = path.split('#').next().unwrap_or(path);
        path.split('/')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| {
                // Strip extension from last segment (e.g. index.phtml → index)
                let seg = seg.split('.').next().unwrap_or(seg);
                tokenize(seg)
            })
            .filter(|t| !NOISE_TOKENS.contains(&t.as_str()) && t.len() >= 2)
            .collect()
    } else if s.contains('\\') {
        // --- Style 2: backslash-separated (PHP namespace / Windows path) ---
        // e.g. App\Http\Controllers\HomeController  or  App\Http\Controllers\Home.php
        let s = s.trim_end_matches(".php").trim_end_matches(".PHP");
        s.split('\\')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| tokenize(seg))
            .filter(|t| !NOISE_TOKENS.contains(&t.as_str()) && t.len() >= 2)
            .collect()
    } else if looks_like_dot_notation(s) {
        // --- Style 3: dot-notation (view/route names) ---
        // e.g. detail.index  application.search.index  home.create
        // NOTE: NOISE_TOKENS are *not* filtered here — in dot-notation every
        // segment is a domain concept (action/module name), not a structural dir.
        s.split('.')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| tokenize(seg))
            .filter(|t| t.len() >= 2)
            .collect()
    } else {
        return None;
    };

    // Require at least 2 meaningful tokens to avoid over-broad matches
    if tokens.len() < 2 { return None; }

    Some(tokens)
}

/// Return true if `s` looks like a dot-notation path reference (not a filename,
/// version string, or object property expression).
///
/// Valid examples:   `detail.index`  `application.search.index`  `home.create`
/// Invalid examples: `1.0.0`  `user.name`  `index.php`  `foo`
fn looks_like_dot_notation(s: &str) -> bool {
    if !s.contains('.') || s.contains(' ') || s.contains('/') || s.contains('\\') {
        return false;
    }
    let segments: Vec<&str> = s.split('.').collect();
    if segments.len() < 2 { return false; }
    segments.iter().all(|seg| {
        seg.len() >= 2
            && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !seg.chars().all(|c| c.is_ascii_digit())
    })
}

/// Extract internal path-reference token sets from file content.
///
/// Scans all single- and double-quoted strings and extracts those that look
/// like internal path references: slash paths, PHP backslash namespaces,
/// and dot-notation view/route names (e.g. `detail.index`).
/// Returns one token set per plausible reference found.
pub fn extract_uri_paths(content: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let start = i;
            // Scan to closing quote; bail on newline to avoid runaway matches
            while i < bytes.len() && bytes[i] != quote && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == quote {
                // Safety: start..i is within a valid UTF-8 string
                if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                    if let Some(tokens) = parse_uri_tokens(s) {
                        result.push(tokens);
                    }
                }
            }
        }
        i += 1;
    }

    result
}

/// Bonus score [0.0, 1.0] reflecting whether `target_full` contains explicit
/// URI references that resolve to `candidate_rel`.
///
/// Overlap is measured as: matched_tokens / uri_tokens.
/// A threshold of 0.5 is required to suppress coincidental partial matches.
pub fn link_bonus(target_full: &Path, candidate_rel: &Path) -> f64 {
    let Ok(bytes) = std::fs::read(target_full) else { return 0.0 };
    let content = String::from_utf8_lossy(&bytes);
    let uri_token_sets = extract_uri_paths(&content);
    if uri_token_sets.is_empty() { return 0.0; }

    // Build candidate token set (noise-filtered) for fast lookup
    let c_tokens: HashSet<String> = normalize(candidate_rel)
        .into_iter()
        .filter(|t| !NOISE_TOKENS.contains(&t.as_str()))
        .collect();

    let mut best = 0.0f64;
    for uri_tokens in &uri_token_sets {
        let total = uri_tokens.len();
        let overlap = uri_tokens.iter().filter(|t| c_tokens.contains(*t)).count();
        let score = overlap as f64 / total as f64;
        if score >= 0.5 {
            best = best.max(score);
        }
    }
    best
}
