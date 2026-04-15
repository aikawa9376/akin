mod common;
mod csharp;
mod generic;
mod java;
mod php;
mod python;
mod rust;

use std::collections::HashSet;
use std::path::Path;

use self::common::WeightedRef;

use crate::tokenizer::normalize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Language {
    Python,
    Rust,
    Java,
    CSharp,
    Php,
    Generic,
}

pub(crate) fn detect_language(path: &Path) -> Language {
    match path.extension().and_then(|ext| ext.to_str()).unwrap_or("") {
        "py" | "pyw" | "pyi" => Language::Python,
        "rs" => Language::Rust,
        "java" | "kt" | "kts" => Language::Java,
        "cs" => Language::CSharp,
        "php" | "phtml" => Language::Php,
        _ => {
            let filename = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");
            if filename.ends_with(".blade.php") {
                Language::Php
            } else {
                Language::Generic
            }
        }
    }
}

fn collect_refs(content: &str, lang: Language) -> Vec<WeightedRef> {
    let mut refs = generic::extract_refs(content);
    refs.extend(match lang {
        Language::Python => python::extract_refs(content),
        Language::Rust => rust::extract_refs(content),
        Language::Java => java::extract_refs(content),
        Language::CSharp => csharp::extract_refs(content),
        Language::Php => php::extract_refs(content),
        Language::Generic => vec![],
    });
    refs
}

pub(crate) fn reference_bonus_for_content(
    content: &str,
    lang: Language,
    candidate_rel: &Path,
) -> f64 {
    let all_refs = collect_refs(content, lang);
    if all_refs.is_empty() {
        return 0.0;
    }

    let candidate_tokens: HashSet<String> = normalize(candidate_rel).into_iter().collect();

    let mut best = 0.0f64;
    for reference in &all_refs {
        let total = reference.tokens.len();
        if total == 0 {
            continue;
        }
        let overlap = reference
            .tokens
            .iter()
            .filter(|token| candidate_tokens.contains(*token))
            .count();
        let score = overlap as f64 / total as f64;
        if score >= 0.5 {
            best = best.max((score * reference.weight).min(1.5));
        }
    }

    best
}

pub(crate) fn feature_bonus(target_full: &Path, candidate_rel: &Path) -> f64 {
    let Ok(bytes) = std::fs::read(target_full) else {
        return 0.0;
    };
    let content = String::from_utf8_lossy(&bytes);
    reference_bonus_for_content(&content, detect_language(target_full), candidate_rel)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{Language, reference_bonus_for_content};

    #[test]
    fn laravel_controller_view_refs_prefer_matching_blade() {
        let content = "<?php\nreturn view('users.index');\n";
        let blade = Path::new("resources/views/users/index.blade.php");
        let other = Path::new("resources/views/users/show.blade.php");

        let blade_bonus = reference_bonus_for_content(content, Language::Php, blade);
        let other_bonus = reference_bonus_for_content(content, Language::Php, other);

        assert!(blade_bonus > other_bonus);
        assert!(blade_bonus > 1.0);
    }

    #[test]
    fn blade_component_tags_match_component_views() {
        let content = "<x-alert />\n";
        let component = Path::new("resources/views/components/alert.blade.php");
        let unrelated = Path::new("resources/views/components/modal.blade.php");

        let component_bonus = reference_bonus_for_content(content, Language::Php, component);
        let unrelated_bonus = reference_bonus_for_content(content, Language::Php, unrelated);

        assert!(component_bonus > unrelated_bonus);
    }

    #[test]
    fn laravel_route_calls_boost_route_files() {
        let content = "<?php\nreturn to_route('users.show');\n";
        let routes = Path::new("routes/web.php");
        let controller = Path::new("app/Http/Controllers/UserController.php");

        let routes_bonus = reference_bonus_for_content(content, Language::Php, routes);
        let controller_bonus = reference_bonus_for_content(content, Language::Php, controller);

        assert!(routes_bonus > 0.5);
        assert!(routes_bonus > controller_bonus);
    }

    #[test]
    fn php_class_constant_refs_match_controllers() {
        let content = "<?php\nRoute::get('/users', [UserController::class, 'index']);\n";
        let controller = Path::new("app/Http/Controllers/UserController.php");
        let model = Path::new("app/Models/User.php");

        let controller_bonus = reference_bonus_for_content(content, Language::Php, controller);
        let model_bonus = reference_bonus_for_content(content, Language::Php, model);

        assert!(controller_bonus > model_bonus);
        assert!(controller_bonus > 1.0);
    }
}
