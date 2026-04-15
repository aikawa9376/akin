use super::common::{WeightedRef, tokenize_module_path, weighted_ref};

pub(super) fn extract_refs(content: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }

        let module: Option<&str> = if let Some(rest) = line.strip_prefix("from ") {
            let rest = rest.trim_start_matches('.');
            let mut parts = rest.split_whitespace();
            match (parts.next(), parts.next()) {
                (Some("import"), Some(module)) => Some(module.trim_end_matches(',')),
                (Some(module), Some("import")) if module != "import" => Some(module),
                _ => None,
            }
        } else if let Some(rest) = line.strip_prefix("import ") {
            rest.split(',')
                .next()
                .and_then(|module| module.split(" as ").next())
                .map(str::trim)
        } else {
            None
        };

        if let Some(module) = module.filter(|module| !module.is_empty()) {
            if let Some(reference) = weighted_ref(tokenize_module_path(module, &[]), 1.0) {
                result.push(reference);
            }
        }
    }

    result
}
