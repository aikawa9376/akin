use crate::tokenizer::{NOISE_TOKENS, tokenize};

#[derive(Clone, Debug, PartialEq)]
pub(super) struct WeightedRef {
    pub tokens: Vec<String>,
    pub weight: f64,
}

pub(super) fn weighted_ref(tokens: Vec<String>, weight: f64) -> Option<WeightedRef> {
    if tokens.is_empty() {
        None
    } else {
        Some(WeightedRef { tokens, weight })
    }
}

fn looks_like_dot_notation(s: &str) -> bool {
    if !s.contains('.') || s.contains(' ') || s.contains('/') || s.contains('\\') {
        return false;
    }
    let segments: Vec<&str> = s.split('.').collect();
    if segments.len() < 2 {
        return false;
    }
    segments.iter().all(|seg| {
        seg.len() >= 2
            && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !seg.chars().all(|c| c.is_ascii_digit())
    })
}

pub(super) fn parse_uri_tokens(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    for prefix in &[
        "http://",
        "https://",
        "//",
        "mailto:",
        "tel:",
        "javascript:",
        "data:",
        "#",
    ] {
        if s.starts_with(prefix) {
            return None;
        }
    }

    let tokens: Vec<String> = if s.contains('/') {
        let path = s.split('?').next().unwrap_or(s);
        let path = path.split('#').next().unwrap_or(path);
        path.split('/')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| {
                let seg = seg.split('.').next().unwrap_or(seg);
                tokenize(seg)
            })
            .filter(|t| !NOISE_TOKENS.contains(&t.as_str()) && t.len() >= 2)
            .collect()
    } else if s.contains('\\') {
        let s = s.trim_end_matches(".php").trim_end_matches(".PHP");
        s.split('\\')
            .filter(|seg| !seg.is_empty())
            .flat_map(tokenize)
            .filter(|t| !NOISE_TOKENS.contains(&t.as_str()) && t.len() >= 2)
            .collect()
    } else if looks_like_dot_notation(s) {
        s.split('.')
            .filter(|seg| !seg.is_empty())
            .flat_map(tokenize)
            .filter(|t| t.len() >= 2)
            .collect()
    } else {
        return None;
    };

    if tokens.len() < 2 {
        return None;
    }
    Some(tokens)
}

pub(super) fn tokenize_module_path(path: &str, extra_noise: &[&str]) -> Vec<String> {
    path.split(['.', ':', '\\'])
        .filter(|seg| !seg.is_empty())
        .flat_map(tokenize)
        .filter(|t| {
            t.len() >= 2
                && !NOISE_TOKENS.contains(&t.as_str())
                && !extra_noise.contains(&t.as_str())
        })
        .collect()
}

pub(super) fn extract_quoted_strings(content: &str) -> Vec<String> {
    let mut result = Vec::new();
    let bytes = content.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote && bytes[i] != b'\n' {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == quote {
                if let Ok(s) = std::str::from_utf8(&bytes[start..i]) {
                    result.push(s.to_string());
                }
            }
        }
        i += 1;
    }

    result
}

pub(super) fn tokenize_component_name(name: &str) -> Vec<String> {
    name.split(['.', '-', ':', '/'])
        .filter(|seg| !seg.is_empty())
        .flat_map(tokenize)
        .filter(|t| t.len() >= 2)
        .collect()
}
