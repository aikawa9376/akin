use crate::tokenizer::{NOISE_TOKENS, tokenize};

use super::common::{
    WeightedRef, extract_quoted_strings, parse_uri_tokens, tokenize_component_name,
    tokenize_module_path, weighted_ref,
};

const PHP_NOISE: &[&str] = &[
    "illuminate",
    "laravel",
    "symfony",
    "doctrine",
    "league",
    "monolog",
    "guzzle",
    "psr",
    "zend",
];

const VIEW_HINTS: &[&str] = &[
    "view(",
    "view()->",
    "View::make(",
    "View::first(",
    "response()->view(",
    "redirect()->view(",
    "Route::view(",
    "@extends(",
    "@include(",
    "@includeif(",
    "@includewhen(",
    "@includeunless(",
    "@includefirst(",
    "@component(",
    "@each(",
    "@livewire(",
];

const ROUTE_HINTS: &[&str] = &[
    "route(",
    "to_route(",
    "->route(",
    "redirect()->route(",
    "Route::",
];

fn line_has_any_hint(line: &str, hints: &[&str]) -> bool {
    let lower = line.to_ascii_lowercase();
    hints.iter().any(|hint| lower.contains(hint))
}

fn extract_php_use_refs(content: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with('#') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("use ") {
            let path = rest
                .split(" as ")
                .next()
                .unwrap_or(rest)
                .trim_end_matches(';')
                .trim();
            if path.starts_with('(') {
                continue;
            }
            let tokens: Vec<String> = path
                .split('\\')
                .filter(|segment| !segment.is_empty())
                .flat_map(tokenize)
                .filter(|token| {
                    token.len() >= 2
                        && !NOISE_TOKENS.contains(&token.as_str())
                        && !PHP_NOISE.contains(&token.as_str())
                })
                .collect();
            if let Some(reference) = weighted_ref(tokens, 1.05) {
                result.push(reference);
            }
        }
    }

    result
}

fn extract_class_constant_refs(line: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    let bytes = line.as_bytes();
    let mut index = 0usize;

    while index + 7 <= bytes.len() {
        if bytes[index..].starts_with(b"::class") {
            let mut start = index;
            while start > 0 {
                let ch = bytes[start - 1] as char;
                if ch.is_ascii_alphanumeric() || ch == '_' || ch == '\\' {
                    start -= 1;
                } else {
                    break;
                }
            }
            if start < index {
                if let Some(reference) =
                    weighted_ref(tokenize_module_path(&line[start..index], PHP_NOISE), 1.25)
                {
                    result.push(reference);
                }
            }
            index += 7;
        } else {
            index += 1;
        }
    }

    result
}

fn extract_component_tag_refs(line: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    let mut rest = line;
    while let Some(position) = rest.find("<x-") {
        let tag = &rest[position + 3..];
        let name: String = tag
            .chars()
            .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | ':' | '/'))
            .collect();
        if !name.is_empty() {
            let mut tokens = vec!["components".to_string()];
            tokens.extend(tokenize_component_name(&name));
            if let Some(reference) = weighted_ref(tokens, 1.35) {
                result.push(reference);
            }
        }
        rest = &tag[name.len()..];
    }
    result
}

pub(super) fn extract_refs(content: &str) -> Vec<WeightedRef> {
    let mut result = extract_php_use_refs(content);

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with('#') {
            continue;
        }

        let quoted = extract_quoted_strings(line);

        if line_has_any_hint(line, VIEW_HINTS) {
            for value in &quoted {
                if let Some(tokens) = parse_uri_tokens(value) {
                    if let Some(reference) = weighted_ref(tokens, 1.40) {
                        result.push(reference);
                    }
                }
            }
        }

        if line_has_any_hint(line, ROUTE_HINTS) {
            for value in &quoted {
                if let Some(tokens) = parse_uri_tokens(value) {
                    if let Some(reference) = weighted_ref(tokens, 1.15) {
                        result.push(reference);
                    }
                }
            }
            if let Some(reference) = weighted_ref(vec!["routes".into(), "web".into()], 1.10) {
                result.push(reference);
            }
        }

        result.extend(extract_class_constant_refs(line));
        result.extend(extract_component_tag_refs(line));
    }

    result
}
