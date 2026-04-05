use std::collections::HashSet;
use std::path::Path;

use crate::tokenizer::{normalize, tokenize, NOISE_TOKENS};

// ── Language detection ───────────────────────────────────────────────────────

#[derive(PartialEq, Eq)]
enum Language {
    Python,
    Rust,
    Java,   // also Kotlin
    CSharp,
    Php,
    Generic,
}

fn detect_language(path: &Path) -> Language {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "py" | "pyw" | "pyi" => Language::Python,
        "rs" => Language::Rust,
        "java" | "kt" | "kts" => Language::Java,
        "cs" => Language::CSharp,
        "php" | "phtml" => Language::Php,
        _ => Language::Generic,
    }
}

// ── Quoted-string scanner (language-agnostic) ────────────────────────────────

/// Return true if `s` looks like a dot-notation path reference (not a filename,
/// version string, or object property expression).
///
/// Valid:   `detail.index`  `application.search.index`  `home.create`
/// Invalid: `1.0.0`  `index.php`  `foo`
fn looks_like_dot_notation(s: &str) -> bool {
    if !s.contains('.') || s.contains(' ') || s.contains('/') || s.contains('\\') {
        return false;
    }
    let segments: Vec<&str> = s.split('.').collect();
    if segments.len() < 2 { return false; }
    segments.iter().all(|seg| {
        seg.len() >= 2
            && seg.chars().all(|c| c.is_alphanumeric() || c == '_')
            && !seg.chars().all(|c| c.is_ascii_digit())
    })
}

/// Try to parse a quoted string as an internal path reference.
///
/// Handles three styles:
/// 1. **Slash paths** — `/application/search`, `../views/user`
/// 2. **Backslash paths** — `App\Http\Controllers\HomeController` (PHP)
/// 3. **Dot-notation** — `detail.index`, `application.search.index` (Blade/ZF)
///
/// External schemes (http/https/mailto/tel/data/javascript/`//`/#) are filtered out.
fn parse_uri_tokens(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    if s.is_empty() { return None; }

    for prefix in &["http://", "https://", "//", "mailto:", "tel:", "javascript:", "data:", "#"] {
        if s.starts_with(prefix) { return None; }
    }

    let tokens: Vec<String> = if s.contains('/') {
        // Style 1: slash-separated path
        let path = s.split('?').next().unwrap_or(s);
        let path = path.split('#').next().unwrap_or(path);
        path.split('/')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| {
                let seg = seg.split('.').next().unwrap_or(seg); // strip extension
                tokenize(seg)
            })
            .filter(|t| !NOISE_TOKENS.contains(&t.as_str()) && t.len() >= 2)
            .collect()
    } else if s.contains('\\') {
        // Style 2: backslash-separated (PHP namespace / Windows path)
        let s = s.trim_end_matches(".php").trim_end_matches(".PHP");
        s.split('\\')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| tokenize(seg))
            .filter(|t| !NOISE_TOKENS.contains(&t.as_str()) && t.len() >= 2)
            .collect()
    } else if looks_like_dot_notation(s) {
        // Style 3: dot-notation view/route names (Blade, ZF2, etc.)
        // NOISE_TOKENS not filtered here — each segment is a domain concept
        s.split('.')
            .filter(|seg| !seg.is_empty())
            .flat_map(|seg| tokenize(seg))
            .filter(|t| t.len() >= 2)
            .collect()
    } else {
        return None;
    };

    if tokens.len() < 2 { return None; }
    Some(tokens)
}

/// Scan all quoted strings in `content` and return token sets for those that
/// look like internal path references.
fn extract_quoted_refs(content: &str) -> Vec<Vec<String>> {
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
                    if let Some(tokens) = parse_uri_tokens(s) {
                        result.push(tokens);
                    }
                }
            }
        }
        i += 1;
    }
    result
}

// ── Unquoted reference extractors (language-specific) ────────────────────────

/// Tokenize a module path (dot / `::` / backslash separated) into meaningful
/// tokens, filtering NOISE_TOKENS and any language-specific `extra_noise`.
fn tokenize_module_path(path: &str, extra_noise: &[&str]) -> Vec<String> {
    path.split(['.', ':', '\\'])
        .filter(|seg| !seg.is_empty())
        .flat_map(|seg| tokenize(seg))
        .filter(|t| {
            t.len() >= 2
                && !NOISE_TOKENS.contains(&t.as_str())
                && !extra_noise.contains(&t.as_str())
        })
        .collect()
}

fn push_if_nonempty(result: &mut Vec<Vec<String>>, tokens: Vec<String>) {
    if !tokens.is_empty() { result.push(tokens); }
}

/// Python: `import pkg.module` and `from pkg.module import X`
///
/// Leading dots (relative imports) are stripped before tokenizing.
fn extract_python_refs(content: &str) -> Vec<Vec<String>> {
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') { continue; }

        let module: Option<&str> = if let Some(rest) = line.strip_prefix("from ") {
            // Strip relative dots: `from ..pkg.mod import X` → `pkg.mod`
            let rest = rest.trim_start_matches('.');
            let mut parts = rest.split_whitespace();
            match (parts.next(), parts.next()) {
                // `from . import module` → module is after "import"
                (Some("import"), Some(m)) => Some(m.trim_end_matches(',')),
                // `from pkg.mod import X` → module is the first token
                (Some(m), Some("import")) if m != "import" => Some(m),
                _ => None,
            }
        } else if let Some(rest) = line.strip_prefix("import ") {
            // `import pkg.mod` or `import pkg.mod as alias`
            rest.split(',').next()
                .and_then(|m| m.split(" as ").next())
                .map(str::trim)
        } else {
            None
        };

        if let Some(m) = module.filter(|m| !m.is_empty()) {
            push_if_nonempty(&mut result, tokenize_module_path(m, &[]));
        }
    }
    result
}

/// Rust: `use crate::module::Item;` and `mod name;`
///
/// For `use` paths, both the module path (all-but-last segment) and the full
/// path are pushed so that `use crate::scorer::fn_name` still matches `scorer.rs`.
fn extract_rust_refs(content: &str) -> Vec<Vec<String>> {
    const RUST_NOISE: &[&str] = &[
        "crate", "super", "self", "std", "core", "alloc", "proc_macro",
    ];
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") { continue; }

        if let Some(rest) = line.strip_prefix("use ") {
            // Strip group import suffix: `use a::b::{X, Y};` → `a::b`
            let path_str = rest
                .trim_end_matches(';')
                .split('{').next().unwrap_or(rest)
                .trim_end_matches("::")
                .trim();

            // Module path = all-but-last "::" segment
            // e.g. `crate::scorer::similarity_score` → module = `crate::scorer`
            let segments: Vec<&str> =
                path_str.split("::").filter(|s| !s.is_empty()).collect();
            if segments.len() >= 2 {
                let module_str = segments[..segments.len() - 1].join("::");
                push_if_nonempty(&mut result, tokenize_module_path(&module_str, RUST_NOISE));
            }

            // Full path (handles `use crate::module;` where module == imported item)
            let full = tokenize_module_path(path_str, RUST_NOISE);
            if !full.is_empty() {
                let is_dup = result.last().map(|l| l == &full).unwrap_or(false);
                if !is_dup { result.push(full); }
            }
        } else if let Some(rest) = line.strip_prefix("mod ") {
            let name = rest.trim_end_matches(';').trim();
            if !name.contains('{') {
                push_if_nonempty(
                    &mut result,
                    tokenize(name).into_iter().filter(|t| t.len() >= 2).collect(),
                );
            }
        }
    }
    result
}

/// Java / Kotlin: `import com.example.Class;`
///
/// Common TLD and vendor prefixes are stripped via JAVA_NOISE.
/// Requires ≥2 meaningful tokens after filtering to suppress stdlib false-positives.
fn extract_java_refs(content: &str) -> Vec<Vec<String>> {
    const JAVA_NOISE: &[&str] = &[
        "com", "org", "net", "io", "gov", "edu",
        "java", "javax", "android", "kotlin",
        "sun", "oracle", "apache", "google",
    ];
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') { continue; }

        if let Some(rest) = line.strip_prefix("import ") {
            let rest = rest.strip_prefix("static ").unwrap_or(rest);
            let path = rest
                .trim_end_matches(';')
                .trim_end_matches(".*") // wildcard: com.example.*
                .trim();
            let tokens = tokenize_module_path(path, JAVA_NOISE);
            if tokens.len() >= 2 { result.push(tokens); }
        }
    }
    result
}

/// C#: `using Company.Product.Class;`
///
/// System.* and Microsoft.* prefixes are stripped.
/// Alias form `using Alias = Company.Product;` is also handled.
fn extract_csharp_refs(content: &str) -> Vec<Vec<String>> {
    const CSHARP_NOISE: &[&str] = &["system", "microsoft", "windows", "azure"];
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') { continue; }

        if let Some(rest) = line.strip_prefix("using ") {
            let rest = rest.strip_prefix("static ").unwrap_or(rest);
            // `using Alias = Namespace.Class;` → take part after `=`
            let rest = rest
                .splitn(2, " = ")
                .nth(1)
                .unwrap_or(rest);
            let path = rest.trim_end_matches(';').trim();
            let tokens = tokenize_module_path(path, CSHARP_NOISE);
            if tokens.len() >= 2 { result.push(tokens); }
        }
    }
    result
}

/// PHP: `use App\Controllers\HomeController;` (unquoted use-statement form)
///
/// Common framework vendor prefixes are stripped.
/// Note: PHP `require`/`include` with quoted paths are already handled by
/// the language-agnostic quoted-string scanner.
fn extract_php_use_refs(content: &str) -> Vec<Vec<String>> {
    const PHP_NOISE: &[&str] = &[
        "illuminate", "laravel", "symfony", "doctrine",
        "league", "monolog", "guzzle", "psr", "zend",
    ];
    let mut result = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("//") || line.starts_with('*') || line.starts_with('#') { continue; }

        if let Some(rest) = line.strip_prefix("use ") {
            // `use App\Model\User as U;` → strip alias and semicolon
            let path = rest
                .split(" as ").next().unwrap_or(rest)
                .trim_end_matches(';')
                .trim();
            // Closure `use ($var)` starts with `(` — skip
            if path.starts_with('(') { continue; }
            let tokens: Vec<String> = path
                .split('\\')
                .filter(|seg| !seg.is_empty())
                .flat_map(|seg| tokenize(seg))
                .filter(|t| {
                    t.len() >= 2
                        && !NOISE_TOKENS.contains(&t.as_str())
                        && !PHP_NOISE.contains(&t.as_str())
                })
                .collect();
            push_if_nonempty(&mut result, tokens);
        }
    }
    result
}

fn extract_unquoted_refs(content: &str, lang: &Language) -> Vec<Vec<String>> {
    match lang {
        Language::Python  => extract_python_refs(content),
        Language::Rust    => extract_rust_refs(content),
        Language::Java    => extract_java_refs(content),
        Language::CSharp  => extract_csharp_refs(content),
        Language::Php     => extract_php_use_refs(content),
        Language::Generic => vec![],
    }
}

// ── Content similarity ───────────────────────────────────────────────────────

/// Read file content and return a set of word tokens (length >= 2)
pub fn content_tokens(path: &Path) -> HashSet<String> {
    let Ok(bytes) = std::fs::read(path) else { return HashSet::new() };
    let text = String::from_utf8_lossy(&bytes);
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| s.len() >= 2)
        .map(|s| s.to_lowercase())
        .collect()
}

/// Jaccard similarity between file contents using word tokens
pub fn content_similarity(a: &Path, b: &Path) -> f64 {
    let a_tokens = content_tokens(a);
    let b_tokens = content_tokens(b);
    if a_tokens.is_empty() && b_tokens.is_empty() { return 0.0; }
    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.union(&b_tokens).count();
    if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Bonus score [0.0, 1.0] based on whether `target_full` explicitly references
/// `candidate_rel` via path literals, import statements, or use declarations.
///
/// Combines:
/// - Language-agnostic quoted-string scan (href, src, require strings, etc.)
/// - Language-specific unquoted import parsing (Python/Rust/Java/C#/PHP)
///
/// Overlap ≥ 50% of URI tokens is required to suppress accidental matches.
pub fn link_bonus(target_full: &Path, candidate_rel: &Path) -> f64 {
    let Ok(bytes) = std::fs::read(target_full) else { return 0.0 };
    let content = String::from_utf8_lossy(&bytes);
    let lang = detect_language(target_full);

    let mut all_refs = extract_quoted_refs(&content);
    all_refs.extend(extract_unquoted_refs(&content, &lang));

    if all_refs.is_empty() { return 0.0; }

    // Candidate path tokens (noise-filtered) for overlap calculation
    let c_tokens: HashSet<String> = normalize(candidate_rel)
        .into_iter()
        .filter(|t| !NOISE_TOKENS.contains(&t.as_str()))
        .collect();

    let mut best = 0.0f64;
    for uri_tokens in &all_refs {
        let total = uri_tokens.len();
        if total == 0 { continue; }
        let overlap = uri_tokens.iter().filter(|t| c_tokens.contains(*t)).count();
        let score = overlap as f64 / total as f64;
        if score >= 0.5 { best = best.max(score); }
    }
    best
}
