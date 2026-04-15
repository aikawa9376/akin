use crate::tokenizer::tokenize;

use super::common::{WeightedRef, tokenize_module_path, weighted_ref};

const RUST_NOISE: &[&str] = &[
    "crate",
    "super",
    "self",
    "std",
    "core",
    "alloc",
    "proc_macro",
];

pub(super) fn extract_refs(content: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }

        if let Some(rest) = line.strip_prefix("use ") {
            let path_str = rest
                .trim_end_matches(';')
                .split('{')
                .next()
                .unwrap_or(rest)
                .trim_end_matches("::")
                .trim();

            let segments: Vec<&str> = path_str.split("::").filter(|seg| !seg.is_empty()).collect();
            if segments.len() >= 2 {
                let module_str = segments[..segments.len() - 1].join("::");
                if let Some(reference) =
                    weighted_ref(tokenize_module_path(&module_str, RUST_NOISE), 1.0)
                {
                    result.push(reference);
                }
            }

            let full = tokenize_module_path(path_str, RUST_NOISE);
            if let Some(reference) = weighted_ref(full, 1.0) {
                let is_dup = result
                    .iter()
                    .any(|existing| existing.tokens == reference.tokens);
                if !is_dup {
                    result.push(reference);
                }
            }
        } else if let Some(rest) = line.strip_prefix("mod ") {
            let name = rest.trim_end_matches(';').trim();
            if !name.contains('{') {
                if let Some(reference) = weighted_ref(
                    tokenize(name)
                        .into_iter()
                        .filter(|token| token.len() >= 2)
                        .collect(),
                    1.0,
                ) {
                    result.push(reference);
                }
            }
        }
    }

    result
}
