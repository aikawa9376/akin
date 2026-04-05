use std::path::Path;

/// Noise tokens that should be weighted down (common directory names)
pub const NOISE_TOKENS: &[&str] = &[
    "app", "src", "lib", "resources", "assets", "main", "index",
    "mod", "bin", "pkg", "internal", "common", "shared",
];

/// Architectural role words — indicate *type* of file, not its domain
pub const TYPE_WORDS: &[&str] = &[
    "controller", "model", "view", "service", "repository", "factory",
    "helper", "manager", "handler", "test", "spec", "dto", "dao",
    "middleware", "component", "provider", "resolver", "presenter",
    "interactor", "usecase", "query", "command", "listener", "observer",
];

/// Generic filename stems that carry no domain information on their own.
/// When a file has one of these names, the parent directory is the real domain.
pub const GENERIC_STEMS: &[&str] = &[
    "index", "show", "create", "edit", "delete", "update", "store",
    "main", "home", "base", "default", "init", "bootstrap",
];

/// Split a path string into tokens by separators and word boundaries (camelCase, snake_case)
pub fn tokenize(path: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for segment in path.split(['/', '\\', '.', '-']) {
        if segment.is_empty() {
            continue;
        }
        let mut current = String::new();
        let chars: Vec<char> = segment.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if ch == '_' {
                if !current.is_empty() {
                    tokens.push(current.to_lowercase());
                    current = String::new();
                }
            } else if ch.is_uppercase() && i > 0 && chars[i - 1].is_lowercase() {
                if !current.is_empty() {
                    tokens.push(current.to_lowercase());
                    current = String::new();
                }
                current.push(ch);
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            tokens.push(current.to_lowercase());
        }
    }
    tokens
}

/// Normalize a path: strip all extensions, lowercase, tokenize.
/// e.g. "src/UserController.spec.ts" → ["src", "user", "controller"]
pub fn normalize(path: &Path) -> Vec<String> {
    let mut p = path.to_path_buf();
    while p.extension().is_some() {
        p = p.with_extension("");
    }
    let s = p.to_string_lossy().to_string();
    tokenize(&s)
}

/// Extract the primary stem: the first dot-delimited segment of the filename (lowercased).
/// e.g. "UserController.spec.ts" → "usercontroller"
pub fn primary_stem(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Extract the original-case stem (before lowercasing) for proper camelCase splitting.
/// e.g. "IndexController.php" → "IndexController"
pub fn primary_stem_original(path: &Path) -> &str {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .split('.')
        .next()
        .unwrap_or("")
}

/// Extract the domain tokens — the functional/business identifier of a file.
///
/// Two strategies:
/// - Generic filename (index, show, create…): the parent directory name is the real domain.
///   e.g. `view/search/index.phtml` → domain = ["search"]
///        `view/index/index.phtml`  → domain = ["index"]
/// - Specific filename (IndexController, UserService…): strip TYPE_WORDS from tokens.
///   e.g. `IndexController.php` → tokens ["index","controller"] → domain = ["index"]
///        `UserService.ts`      → tokens ["user","service"]     → domain = ["user"]
///
/// CamelCase splitting uses the *original-case* stem to correctly split "IndexController".
pub fn domain_tokens(path: &Path) -> Vec<String> {
    let stem_original = primary_stem_original(path);
    let stem_lower = stem_original.to_lowercase();

    if GENERIC_STEMS.contains(&stem_lower.as_str()) {
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let parent_tokens: Vec<String> = tokenize(parent)
            .into_iter()
            .filter(|t| !TYPE_WORDS.contains(&t.as_str()))
            .collect();
        if !parent_tokens.is_empty() {
            return parent_tokens;
        }
    }

    tokenize(stem_original)
        .into_iter()
        .filter(|t| !TYPE_WORDS.contains(&t.as_str()))
        .collect()
}
