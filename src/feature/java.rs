use super::common::{WeightedRef, tokenize_module_path, weighted_ref};

const JAVA_NOISE: &[&str] = &[
    "com", "org", "net", "io", "gov", "edu", "java", "javax", "android", "kotlin", "sun", "oracle",
    "apache", "google",
];

pub(super) fn extract_refs(content: &str) -> Vec<WeightedRef> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("import ") {
            let rest = rest.strip_prefix("static ").unwrap_or(rest);
            let path = rest.trim_end_matches(';').trim_end_matches(".*").trim();
            let tokens = tokenize_module_path(path, JAVA_NOISE);
            if tokens.len() >= 2 {
                if let Some(reference) = weighted_ref(tokens, 1.0) {
                    result.push(reference);
                }
            }
        }
    }

    result
}
