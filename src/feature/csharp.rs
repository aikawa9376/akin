use super::common::{WeightedRef, tokenize_module_path, weighted_ref};

const CSHARP_NOISE: &[&str] = &["system", "microsoft", "windows", "azure"];

pub(super) fn extract_refs(content: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("using ") {
            let rest = rest.strip_prefix("static ").unwrap_or(rest);
            let rest = rest.splitn(2, " = ").nth(1).unwrap_or(rest);
            let path = rest.trim_end_matches(';').trim();
            let tokens = tokenize_module_path(path, CSHARP_NOISE);
            if tokens.len() >= 2 {
                if let Some(reference) = weighted_ref(tokens, 1.0) {
                    result.push(reference);
                }
            }
        }
    }

    result
}
