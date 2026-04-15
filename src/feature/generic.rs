use super::common::{WeightedRef, extract_quoted_strings, parse_uri_tokens, weighted_ref};

pub(super) fn extract_refs(content: &str) -> Vec<WeightedRef> {
    extract_quoted_strings(content)
        .into_iter()
        .filter_map(|quoted| parse_uri_tokens(&quoted).and_then(|tokens| weighted_ref(tokens, 1.0)))
        .collect()
}
