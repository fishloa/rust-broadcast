//! Guard: the public API of `broadcast-core` is frozen.
//!
//! `broadcast-core` is the stable floor the wire-format crates build on, so
//! its public API must not change without a reviewed edit of the committed
//! pin. This scanner lists every public item of `broadcast-core/src` — each
//! `pub fn`/`struct`/`enum`/`trait`/`const`/`type`/`static`/`mod`, every
//! public enum variant, every public struct field and every
//! `#[macro_export]` macro, each prefixed with its module path — and compares
//! it to `core_api_pin.txt`:
//!
//! - a committed line the scanner no longer produces means an item was removed
//!   or renamed (a breaking change to a frozen crate);
//! - a produced line that is not committed means an item was added (still a
//!   reviewed change before `1.0`).
//!
//! The comparison is lexical, not a proof: a `macro_rules!`-generated item, or
//! an item reachable only through a `pub use` re-export in a third crate, is
//! not listed. Names are matched as whole tokens; a renamed type is invisible.
//! Run with `CORE_API_PIN_BLESS=1` to rewrite the committed file from the
//! current tree (a reviewable diff).
//!
//! Two companion checks live here because they concern the same boundary:
//! `broadcast-core` must have no non-optional dependency, and
//! `broadcast-common` must not re-export anything from it.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use workspace_guards::{crate_dir, rs_files, src_files};

/// The committed listing.
fn pin_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/guards/core_api_pin.txt")
}

/// Blank out comments and string/char literals (keeping newlines) so brace
/// counting and the item scan see only code. A lifetime (`'a`) is not a char
/// literal and is kept.
fn strip_noncode(src: &str) -> String {
    let bytes: Vec<char> = src.chars().collect();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let n = bytes.len();
    while i < n {
        let c = bytes[i];
        // line comment
        if c == '/' && i + 1 < n && bytes[i + 1] == '/' {
            while i < n && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        }
        // block comment (nested)
        if c == '/' && i + 1 < n && bytes[i + 1] == '*' {
            let mut depth = 1;
            i += 2;
            while i < n && depth > 0 {
                if bytes[i] == '/' && i + 1 < n && bytes[i + 1] == '*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == '*' && i + 1 < n && bytes[i + 1] == '/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == '\n' {
                        out.push('\n');
                    }
                    i += 1;
                }
            }
            continue;
        }
        // raw string: r"..." / r#"..."# (also br"...")
        if c == 'r' {
            let mut j = i + 1;
            while j < n && bytes[j] == '#' {
                j += 1;
            }
            if j < n && bytes[j] == '"' {
                let hashes = j - (i + 1);
                out.push(' ');
                let mut k = j + 1;
                loop {
                    if k >= n {
                        break;
                    }
                    if bytes[k] == '"' {
                        let mut h = 0;
                        while h < hashes && k + 1 + h < n && bytes[k + 1 + h] == '#' {
                            h += 1;
                        }
                        if h == hashes {
                            k += 1 + hashes;
                            break;
                        }
                    }
                    if bytes[k] == '\n' {
                        out.push('\n');
                    }
                    k += 1;
                }
                i = k;
                continue;
            }
        }
        // string literal
        if c == '"' {
            out.push(' ');
            i += 1;
            while i < n {
                if bytes[i] == '\\' && i + 1 < n {
                    i += 2;
                    continue;
                }
                if bytes[i] == '"' {
                    i += 1;
                    break;
                }
                if bytes[i] == '\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }
        // char literal vs lifetime
        if c == '\'' {
            let is_char = if i + 1 < n && bytes[i + 1] == '\\' {
                true
            } else {
                i + 2 < n && bytes[i + 2] == '\''
            };
            if is_char {
                out.push(' ');
                i += 1;
                while i < n {
                    if bytes[i] == '\\' && i + 1 < n {
                        i += 2;
                        continue;
                    }
                    if bytes[i] == '\'' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

fn module_path(rel: &str) -> String {
    let rel = rel.strip_prefix("src/").unwrap_or(rel);
    let rel = rel.strip_suffix(".rs").unwrap_or(rel);
    if rel == "lib" || rel == "main" {
        return String::new();
    }
    let rel = rel.strip_suffix("/mod").unwrap_or(rel);
    rel.replace('/', "::")
}

#[derive(Clone, Debug)]
enum Kind {
    Enum,
    Struct,
    Impl,
    Trait,
    Module,
}

#[derive(Clone, Debug)]
struct Scope {
    body_depth: usize,
    kind: Kind,
    name: String,
}

fn ident_at_start(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            break;
        }
    }
    out
}

/// The resolved item name for a `fn`, `const`, etc.: an associated item names
/// its impl type, otherwise it sits in the module.
fn qualify(module: &str, scopes: &[Scope], depth: usize, name: &str) -> String {
    let owner = scopes
        .iter()
        .rev()
        .find(|s| s.body_depth == depth && matches!(s.kind, Kind::Impl));
    match owner {
        Some(s) => {
            if module.is_empty() {
                format!("{}::{name}", s.name)
            } else {
                format!("{module}::{}::{name}", s.name)
            }
        }
        None => {
            if module.is_empty() {
                name.to_string()
            } else {
                format!("{module}::{name}")
            }
        }
    }
}

fn impl_type_name(t: &str) -> Option<String> {
    // `impl<T> Trait for Type` -> Type; `impl<T> Type` -> Type.
    let after_impl = t.strip_prefix("impl")?;
    let after = match after_impl.split_once(" for ") {
        Some((_, rhs)) => rhs,
        None => {
            let mut rest = after_impl.trim_start();
            if rest.starts_with('<') {
                let mut depth = 0;
                let mut idx = 0;
                for (i, c) in rest.char_indices() {
                    if c == '<' {
                        depth += 1;
                    } else if c == '>' {
                        depth -= 1;
                        if depth == 0 {
                            idx = i + 1;
                            break;
                        }
                    }
                }
                rest = &rest[idx..];
            }
            rest
        }
    };
    let trimmed = after.trim_start();
    let name = ident_at_start(trimmed);
    (!name.is_empty()).then_some(name)
}

fn scan_file(rel: &str, text: &str, out: &mut BTreeSet<String>) {
    let module = module_path(rel);
    let stripped = strip_noncode(text);
    let lines: Vec<&str> = stripped.lines().collect();
    let mut scopes: Vec<Scope> = Vec::new();
    let mut depth = 0usize;
    // The previous non-empty, non-attribute line, for `#[macro_export]`.
    let mut pending_export = false;

    for raw in lines.iter() {
        let line = *raw;
        let t = line.trim();
        let depth_before = depth;

        if t.starts_with("#[macro_export]") {
            pending_export = true;
        }

        // --- declaration detection (before depth update) ---
        if let Some(rest) = t.strip_prefix("pub fn ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(qualify(&module, &scopes, depth_before, &name));
            }
        } else if let Some(rest) = t
            .strip_prefix("pub const fn ")
            .or_else(|| t.strip_prefix("pub async fn "))
            .or_else(|| t.strip_prefix("pub unsafe fn "))
            .or_else(|| t.strip_prefix("pub async unsafe fn "))
            .or_else(|| t.strip_prefix("pub const unsafe fn "))
        {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(qualify(&module, &scopes, depth_before, &name));
            }
        } else if let Some(rest) = t.strip_prefix("pub struct ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(if module.is_empty() {
                    name.clone()
                } else {
                    format!("{module}::{name}")
                });
                if t.contains('{') {
                    scopes.push(Scope {
                        body_depth: depth_before + 1,
                        kind: Kind::Struct,
                        name,
                    });
                }
            }
        } else if let Some(rest) = t.strip_prefix("pub enum ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(if module.is_empty() {
                    name.clone()
                } else {
                    format!("{module}::{name}")
                });
                if t.contains('{') {
                    scopes.push(Scope {
                        body_depth: depth_before + 1,
                        kind: Kind::Enum,
                        name,
                    });
                }
            }
        } else if let Some(rest) = t.strip_prefix("pub trait ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(if module.is_empty() {
                    name.clone()
                } else {
                    format!("{module}::{name}")
                });
                if t.contains('{') {
                    scopes.push(Scope {
                        body_depth: depth_before + 1,
                        kind: Kind::Trait,
                        name,
                    });
                }
            }
        } else if let Some(rest) = t.strip_prefix("pub type ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(if module.is_empty() {
                    name
                } else {
                    format!("{module}::{name}")
                });
            }
        } else if let Some(rest) = t.strip_prefix("pub const ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(qualify(&module, &scopes, depth_before, &name));
            }
        } else if let Some(rest) = t.strip_prefix("pub static ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(qualify(&module, &scopes, depth_before, &name));
            }
        } else if let Some(rest) = t.strip_prefix("pub mod ") {
            let name = ident_at_start(rest);
            if !name.is_empty() {
                out.insert(if module.is_empty() {
                    format!("mod {name}")
                } else {
                    format!("mod {module}::{name}")
                });
                if t.contains('{') {
                    scopes.push(Scope {
                        body_depth: depth_before + 1,
                        kind: Kind::Module,
                        name,
                    });
                }
            }
        } else if let Some(rest) = t.strip_prefix("macro_rules! ") {
            let name = ident_at_start(rest);
            if !name.is_empty() && pending_export {
                out.insert(format!("macro {name}"));
            }
        }

        // A `impl ... {` opens a method scope (its methods are `pub fn`).
        if t.starts_with("impl")
            && t.contains('{')
            && let Some(name) = impl_type_name(t)
        {
            scopes.push(Scope {
                body_depth: depth_before + 1,
                kind: Kind::Impl,
                name,
            });
        }

        // --- enum variants and struct fields (directly inside the body) ---
        if let Some(scope) = scopes.last()
            && scope.body_depth == depth_before
        {
            match scope.kind {
                Kind::Enum => {
                    let skip = t.is_empty()
                        || t.starts_with('#')
                        || t.starts_with("//")
                        || t.starts_with('}')
                        || t.starts_with("pub ");
                    if !skip {
                        let name = ident_at_start(t);
                        if name
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
                        {
                            out.insert(if module.is_empty() {
                                format!("{}::{name}", scope.name)
                            } else {
                                format!("{module}::{}::{name}", scope.name)
                            });
                        }
                    }
                }
                Kind::Struct => {
                    if let Some(rest) = t.strip_prefix("pub ") {
                        let name = ident_at_start(rest);
                        if !name.is_empty() && rest[name.len()..].trim_start().starts_with(':') {
                            out.insert(if module.is_empty() {
                                format!("{}::{name}", scope.name)
                            } else {
                                format!("{module}::{}::{name}", scope.name)
                            });
                        }
                    }
                }
                _ => {}
            }
        }

        // --- depth update and scope pops ---
        let opens = line.matches('{').count();
        let closes = line.matches('}').count();
        depth = depth.saturating_add(opens).saturating_sub(closes);
        while let Some(top) = scopes.last() {
            if top.body_depth > depth {
                scopes.pop();
            } else {
                break;
            }
        }

        if !t.is_empty() && !t.starts_with('#') && !t.starts_with("//") {
            pending_export = false;
        }
    }
}

fn scan() -> BTreeSet<String> {
    let root = crate_dir("broadcast-core").join("src");
    let mut out = BTreeSet::new();
    for path in rs_files(&root) {
        let rel = path
            .strip_prefix(crate_dir("broadcast-core"))
            .expect("under crate dir")
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).expect("read broadcast-core source");
        scan_file(&rel, &text, &mut out);
    }
    out
}

#[test]
fn core_api_is_pinned() {
    let produced = scan();
    if std::env::var_os("CORE_API_PIN_BLESS").is_some() {
        let mut body = String::from(
            "# broadcast-core public API pin (generated by core_api_pin.rs; \
             CORE_API_PIN_BLESS=1 rewrites it)\n",
        );
        for line in &produced {
            body.push_str(line);
            body.push('\n');
        }
        fs::write(pin_path(), body).expect("write pin file");
        return;
    }
    let committed_text = fs::read_to_string(pin_path()).expect("read core_api_pin.txt");
    let committed: BTreeSet<String> = committed_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();

    let removed: Vec<&String> = committed.difference(&produced).collect();
    let added: Vec<&String> = produced.difference(&committed).collect();
    let mut problems = Vec::new();
    for line in removed {
        problems.push(format!(
            "removed or renamed (a committed public item is no longer produced): {line}"
        ));
    }
    for line in added {
        problems.push(format!(
            "added (a new public item must be a reviewed edit of core_api_pin.txt): {line}"
        ));
    }
    assert!(
        problems.is_empty(),
        "broadcast-core's public API changed; review the diff and, if intended, \
         regenerate the pin with CORE_API_PIN_BLESS=1:\n{}",
        problems.join("\n")
    );
    assert!(
        !committed.is_empty(),
        "core_api_pin.txt is empty; the scanner must list the public API"
    );
}

#[test]
fn broadcast_core_has_no_required_dependency() {
    let text = fs::read_to_string(crate_dir("broadcast-core").join("Cargo.toml"))
        .expect("read broadcast-core/Cargo.toml");
    let mut in_deps = false;
    let mut problems = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            in_deps = t == "[dependencies]";
            continue;
        }
        if !in_deps || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if !t.contains("optional = true") {
            problems.push(t.to_string());
        }
    }
    assert!(
        problems.is_empty(),
        "broadcast-core must have no non-optional dependency (G1); these are not optional:\n{}",
        problems.join("\n")
    );
}

#[test]
fn broadcast_common_does_not_reexport_broadcast_core() {
    let mut problems = Vec::new();
    for (path, text) in src_files("broadcast-common") {
        for (line_no, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("pub use broadcast_core")
                || t.starts_with("pub(crate) use broadcast_core")
            {
                problems.push(format!("{}:{}: {t}", path, line_no + 1));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "broadcast-common must not re-export anything from broadcast-core:\n{}",
        problems.join("\n")
    );
}
