use std::collections::HashMap;

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

#[derive(Clone, Debug)]
struct ResolvedValue {
    value: String,
    complete: bool,
}

fn strip_wrapping_parens(mut expr: &str) -> &str {
    loop {
        let trimmed = expr.trim();
        if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
            return trimmed;
        }

        let mut depth = 0usize;
        let mut balanced = true;
        for (idx, ch) in trimmed.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    if depth == 0 {
                        balanced = false;
                        break;
                    }
                    depth -= 1;
                    if depth == 0 && idx != trimmed.len() - 1 {
                        balanced = false;
                        break;
                    }
                }
                _ => {}
            }
        }

        if balanced && depth == 0 {
            expr = &trimmed[1..trimmed.len() - 1];
        } else {
            return trimmed;
        }
    }
}

fn split_top_level(expr: &str, delimiter: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut paren_depth = 0usize;
    let mut in_single = false;
    let mut in_double = false;
    let bytes = expr.as_bytes();
    let mut idx = 0usize;

    while idx < bytes.len() {
        let ch = bytes[idx] as char;
        match ch {
            '\'' if !in_double && (idx == 0 || bytes[idx - 1] != b'\\') => in_single = !in_single,
            '"' if !in_single && (idx == 0 || bytes[idx - 1] != b'\\') => in_double = !in_double,
            '(' if !in_single && !in_double => paren_depth += 1,
            ')' if !in_single && !in_double && paren_depth > 0 => paren_depth -= 1,
            _ if ch == delimiter && !in_single && !in_double && paren_depth == 0 => {
                parts.push(expr[start..idx].trim());
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
        idx += 1;
    }

    parts.push(expr[start..].trim());
    parts
}

fn extract_call_args(line: &str, needle: &str) -> Option<Vec<String>> {
    let lower = line.to_ascii_lowercase();
    let needle_lower = needle.to_ascii_lowercase();
    let position = lower.find(&needle_lower)?;
    let open_paren = position + needle.len() - 1;
    let bytes = line.as_bytes();
    let mut idx = open_paren + 1;
    let mut paren_depth = 1usize;
    let mut in_single = false;
    let mut in_double = false;
    let mut args = Vec::new();
    let mut arg_start = idx;

    while idx < bytes.len() {
        let ch = bytes[idx] as char;
        match ch {
            '\'' if !in_double && (idx == 0 || bytes[idx - 1] != b'\\') => in_single = !in_single,
            '"' if !in_single && (idx == 0 || bytes[idx - 1] != b'\\') => in_double = !in_double,
            '(' if !in_single && !in_double => paren_depth += 1,
            ')' if !in_single && !in_double => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    args.push(line[arg_start..idx].trim().to_string());
                    return Some(args);
                }
            }
            ',' if !in_single && !in_double && paren_depth == 1 => {
                args.push(line[arg_start..idx].trim().to_string());
                arg_start = idx + 1;
            }
            _ => {}
        }
        idx += 1;
    }

    None
}

fn extract_variable_name(expr: &str) -> Option<&str> {
    let expr = expr.trim();
    let rest = expr.strip_prefix('$')?;
    let end = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '_')
        .last()
        .map(|(idx, ch)| idx + ch.len_utf8())
        .unwrap_or(0);
    if end == 0 { None } else { Some(&rest[..end]) }
}

fn resolve_php_string_expr(
    expr: &str,
    env: &HashMap<String, ResolvedValue>,
) -> Option<ResolvedValue> {
    let expr = strip_wrapping_parens(expr);
    if expr.is_empty() {
        return None;
    }

    if expr.len() >= 2
        && ((expr.starts_with('\'') && expr.ends_with('\''))
            || (expr.starts_with('"') && expr.ends_with('"')))
    {
        return Some(ResolvedValue {
            value: expr[1..expr.len() - 1].to_string(),
            complete: true,
        });
    }

    if let Some(name) = extract_variable_name(expr) {
        return env.get(name).cloned().or_else(|| {
            Some(ResolvedValue {
                value: String::new(),
                complete: false,
            })
        });
    }

    let parts = split_top_level(expr, '.');
    if parts.len() > 1 {
        let mut value = String::new();
        let mut complete = true;
        let mut resolved_any = false;
        for part in parts {
            if let Some(resolved) = resolve_php_string_expr(part, env) {
                if !resolved.value.is_empty() {
                    value.push_str(&resolved.value);
                    resolved_any = true;
                }
                complete &= resolved.complete;
            } else {
                complete = false;
            }
        }
        if resolved_any {
            return Some(ResolvedValue { value, complete });
        }
    }

    None
}

fn tokens_from_dynamic_reference(value: &str) -> Vec<String> {
    parse_uri_tokens(value).unwrap_or_else(|| {
        value
            .split(['.', '/', '\\', '-', ':'])
            .filter(|seg| !seg.is_empty())
            .flat_map(tokenize)
            .filter(|token| token.len() >= 2)
            .collect()
    })
}

fn push_resolved_reference(
    result: &mut Vec<WeightedRef>,
    resolved: Option<ResolvedValue>,
    context: &[&str],
    weight: f64,
) {
    let Some(resolved) = resolved else {
        return;
    };
    if resolved.value.is_empty() {
        return;
    }

    let mut tokens: Vec<String> = context
        .iter()
        .map(|segment| (*segment).to_string())
        .collect();
    tokens.extend(tokens_from_dynamic_reference(&resolved.value));
    tokens.dedup();

    let adjusted_weight = if resolved.complete {
        weight
    } else {
        weight * 0.8
    };
    if let Some(reference) = weighted_ref(tokens, adjusted_weight) {
        result.push(reference);
    }
}

fn assign_resolved_variable(line: &str, env: &mut HashMap<String, ResolvedValue>) {
    let line = line.trim();
    if !line.starts_with('$') {
        return;
    }

    let Some(eq_pos) = line.find('=') else {
        return;
    };
    if matches!(line.as_bytes().get(eq_pos + 1), Some(b'=')) || line[..eq_pos].ends_with('=') {
        return;
    }

    let name = extract_variable_name(&line[..eq_pos]).map(str::to_string);
    let Some(name) = name else {
        return;
    };

    let rhs = line[eq_pos + 1..].trim_end_matches(';').trim();
    if let Some(resolved) = resolve_php_string_expr(rhs, env) {
        env.insert(name, resolved);
    }
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
    let mut env = HashMap::new();

    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with('#') {
            continue;
        }

        assign_resolved_variable(line, &mut env);
        let quoted = extract_quoted_strings(line);

        if line_has_any_hint(line, VIEW_HINTS) {
            for needle in &[
                "view(",
                "view()->first(",
                "view()->make(",
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
            ] {
                if let Some(args) = extract_call_args(line, needle) {
                    let arg_index = if *needle == "Route::view(" || *needle == "@includewhen(" {
                        1
                    } else {
                        0
                    };
                    push_resolved_reference(
                        &mut result,
                        args.get(arg_index)
                            .and_then(|expr| resolve_php_string_expr(expr, &env)),
                        &["views"],
                        1.40,
                    );
                }
            }
            for value in &quoted {
                if let Some(parsed_tokens) = parse_uri_tokens(value) {
                    let mut tokens = vec!["views".to_string()];
                    tokens.extend(parsed_tokens);
                    tokens.dedup();
                    if let Some(reference) = weighted_ref(tokens, 1.40) {
                        result.push(reference);
                    }
                }
            }
        }

        if line_has_any_hint(line, ROUTE_HINTS) {
            for needle in &["route(", "to_route(", "->route(", "redirect()->route("] {
                if let Some(args) = extract_call_args(line, needle) {
                    push_resolved_reference(
                        &mut result,
                        args.first()
                            .and_then(|expr| resolve_php_string_expr(expr, &env)),
                        &["routes"],
                        1.15,
                    );
                }
            }
            for value in &quoted {
                if let Some(parsed_tokens) = parse_uri_tokens(value) {
                    let mut tokens = vec!["routes".to_string()];
                    tokens.extend(parsed_tokens);
                    tokens.dedup();
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
