//! Client-side syntax highlighting over a tree-sitter parse.
//!
//! A capture is a byte range of a code block's text and the class the parser
//! assigned to it — `keyword`, `string`, `number`, and so on. The classes are the
//! names a theme tokenizes, so a renderer maps a name to a colour and never parses
//! a grammar itself.
//!
//! This lives on the client, not in the session. Which code to highlight, which
//! grammar to reach for, and what a class looks like are all presentation
//! decisions; the session sends the code and its authored fence label and nothing
//! more. A client that cannot or does not want to parse simply draws the text.
//!
//! The walk is query-free, as the previous system's was: node kinds are classified
//! by name rather than by a per-grammar query, so a new grammar needs no query
//! file. Parsing is bounded, and an unknown language or an over-large source
//! produces no captures rather than partial colour.

use tree_sitter::{Language, Node, Parser, Tree};

/// A byte range of a code block's text and the class covering it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Capture {
    pub start: u32,
    pub end: u32,
    pub token: String,
}

/// The largest source highlighted. Past this the block is shown plain.
pub const MAX_SOURCE_BYTES: usize = 1024 * 1024;
const MAX_LANGUAGE_BYTES: usize = 64;
const MAX_AST_DEPTH: usize = 512;
const MAX_AST_NODES: usize = 250_000;

/// Captures for `source` in `language`, ordered by byte offset and
/// non-overlapping. Empty when the language is unknown or the source is too big.
pub fn captures(language: &str, source: &str) -> Vec<Capture> {
    if source.len() > MAX_SOURCE_BYTES || language.is_empty() || language.len() > MAX_LANGUAGE_BYTES
    {
        return Vec::new();
    }
    let Some(language) = grammar(language) else {
        return Vec::new();
    };
    let mut parser = Parser::new();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    walk(&tree, source.len())
}

/// A language guessed from a file path's extension, for code that arrived without
/// an authored fence label — a tool result whose argument named the file.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let extension = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "rs" => "rust",
        "py" | "pyi" => "python",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "tsx" => "typescript",
        "json" => "json",
        "sh" | "bash" | "zsh" => "bash",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => return None,
    })
}

/// The tree-sitter grammar for a language label, or `None` when it is not one we
/// ship. Aliases follow the previous system's list.
fn grammar(label: &str) -> Option<Language> {
    let name = label.to_ascii_lowercase();
    Some(match name.as_str() {
        "rust" | "rs" => tree_sitter_rust::LANGUAGE.into(),
        "python" | "py" => tree_sitter_python::LANGUAGE.into(),
        "javascript" | "js" | "jsx" | "node" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" | "ts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        "bash" | "sh" | "shell" | "zsh" => tree_sitter_bash::LANGUAGE.into(),
        "go" | "golang" => tree_sitter_go::LANGUAGE.into(),
        "c" => tree_sitter_c::LANGUAGE.into(),
        "cpp" | "c++" | "cc" | "cxx" | "h" | "hpp" => tree_sitter_cpp::LANGUAGE.into(),
        "toml" => tree_sitter_toml_ng::LANGUAGE.into(),
        "yaml" | "yml" => tree_sitter_yaml::LANGUAGE.into(),
        _ => return None,
    })
}

struct Pending<'tree> {
    node: Node<'tree>,
    inherited: Option<&'static str>,
    depth: usize,
}

/// Walk the tree depth-first in source order, classifying leaves.
///
/// A lexical ancestor (a comment, a string, an embedded fragment) is inherited by
/// its leaves, so a string's punctuation is string-coloured rather than punctuated.
/// An escape inside a string is the one thing allowed to break that inheritance.
fn walk(tree: &Tree, source_len: usize) -> Vec<Capture> {
    let mut captures = Vec::new();
    let mut pending = vec![Pending {
        node: tree.root_node(),
        inherited: None,
        depth: 0,
    }];
    let mut nodes = 0usize;
    while let Some(current) = pending.pop() {
        nodes += 1;
        if nodes > MAX_AST_NODES || current.depth > MAX_AST_DEPTH {
            // Too complex to trust: fall back to plain text rather than partial
            // colours that depend on where the walk stopped.
            return Vec::new();
        }
        let kind = current.node.kind();
        let own = classify(kind);
        let children = current.node.child_count();
        if children == 0 {
            let semantic = match current.inherited {
                Some("string") if own == Some("escape") => Some("escape"),
                Some(outer) => Some(outer),
                None => own.or_else(|| classify_context(&current.node)),
            };
            if let Some(token) = semantic {
                let start = current.node.start_byte();
                let end = current.node.end_byte();
                if end > start && end <= source_len {
                    captures.push(Capture {
                        start: start as u32,
                        end: end as u32,
                        token: token.to_string(),
                    });
                }
            }
            continue;
        }
        let inherited = current
            .inherited
            .or_else(|| own.filter(|token| is_lexical(token)));
        // Reverse insertion so the stack yields children in source order.
        for index in (0..children).rev() {
            if let Some(child) = current.node.child(index) {
                pending.push(Pending {
                    node: child,
                    inherited,
                    depth: current.depth + 1,
                });
            }
        }
    }
    captures
}

fn is_lexical(token: &str) -> bool {
    matches!(token, "comment" | "string" | "embedded")
}

/// A leaf with no classifying kind of its own takes its role from its parent.
fn classify_context(node: &Node) -> Option<&'static str> {
    let parent = node.parent()?;
    let kind = parent.kind();
    if contains(kind, "call")
        || contains(kind, "function")
        || contains(kind, "method")
        || contains(kind, "constructor")
        || contains(kind, "declarator")
    {
        return Some("function");
    }
    if contains(kind, "type")
        || contains(kind, "class")
        || contains(kind, "interface")
        || contains(kind, "trait")
    {
        return Some("type");
    }
    if contains(kind, "field") || contains(kind, "property") || contains(kind, "member") {
        return Some("property");
    }
    if contains(kind, "parameter") || contains(kind, "variable") {
        return Some("variable");
    }
    None
}

/// The generic class of a node kind, by name. A kind that names no class stays
/// plain rather than being guessed at.
fn classify(kind: &str) -> Option<&'static str> {
    if contains(kind, "comment") || contains(kind, "shebang") {
        return Some("comment");
    }
    if contains(kind, "escape") {
        return Some("escape");
    }
    if contains(kind, "string")
        || contains(kind, "character")
        || contains(kind, "heredoc")
        || contains(kind, "regex")
    {
        return Some("string");
    }
    if contains(kind, "number") || contains(kind, "integer") || contains(kind, "float") {
        return Some("number");
    }
    if contains(kind, "keyword") || contains(kind, "modifier") {
        return Some("keyword");
    }
    if contains(kind, "type") || contains(kind, "class_name") || contains(kind, "interface_name") {
        return Some("type");
    }
    if contains(kind, "function") || contains(kind, "method") || contains(kind, "constructor") {
        return Some("function");
    }
    if contains(kind, "constant") || contains(kind, "boolean") || kind == "null" || kind == "nil" {
        return Some("constant");
    }
    if contains(kind, "property") || contains(kind, "field") {
        return Some("property");
    }
    if contains(kind, "attribute") {
        return Some("attribute");
    }
    if contains(kind, "tag_name") {
        return Some("tag");
    }
    if contains(kind, "variable") || contains(kind, "parameter") {
        return Some("variable");
    }
    if contains(kind, "operator") {
        return Some("operator");
    }
    if contains(kind, "punctuation") {
        return Some("punctuation");
    }
    if contains(kind, "embedded") {
        return Some("embedded");
    }
    if is_keyword(kind) {
        return Some("keyword");
    }
    if is_operator(kind) {
        return Some("operator");
    }
    if is_punctuation(kind) {
        return Some("punctuation");
    }
    None
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.contains(needle)
}

fn is_keyword(kind: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "if",
        "else",
        "then",
        "for",
        "while",
        "do",
        "end",
        "return",
        "break",
        "continue",
        "switch",
        "case",
        "default",
        "match",
        "let",
        "var",
        "const",
        "fn",
        "fun",
        "function",
        "class",
        "struct",
        "enum",
        "union",
        "interface",
        "trait",
        "impl",
        "import",
        "from",
        "export",
        "as",
        "in",
        "of",
        "new",
        "try",
        "catch",
        "throw",
        "async",
        "await",
        "yield",
        "pub",
        "private",
        "protected",
        "static",
        "where",
        "with",
        "def",
        "lambda",
        "package",
        "namespace",
        "using",
    ];
    KEYWORDS.contains(&kind)
}

fn is_operator(kind: &str) -> bool {
    if kind.is_empty() || kind.len() > 4 {
        return false;
    }
    kind.bytes()
        .all(|byte| "+-*/%=!<>|&^~?:".contains(byte as char))
}

fn is_punctuation(kind: &str) -> bool {
    kind.len() == 1 && "()[]{}.,;".contains(kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_names(language: &str, source: &str) -> Vec<String> {
        captures(language, source)
            .into_iter()
            .map(|capture| capture.token)
            .collect()
    }

    #[test]
    fn a_python_definition_produces_ordered_captures() {
        let found = captures("py", "def answer():\n  # note\n  return 42\n");
        assert!(!found.is_empty());
        let mut previous = 0u32;
        for capture in &found {
            assert!(capture.start >= previous, "{found:?}");
            assert!(capture.end > capture.start);
            previous = capture.end;
        }
        let names = token_names("py", "def answer():\n  # note\n  return 42\n");
        assert!(names.iter().any(|name| name == "comment"));
        assert!(names.iter().any(|name| name == "number"));
        assert!(names.iter().any(|name| name == "keyword"));
    }

    #[test]
    fn a_rust_function_is_a_keyword_and_a_name() {
        let names = token_names("rust", "fn main() { let x = \"hi\"; }");
        assert!(names.iter().any(|name| name == "keyword"));
        assert!(names.iter().any(|name| name == "string"));
    }

    #[test]
    fn an_escape_inside_a_string_breaks_the_inherited_class() {
        let found = captures("rust", "let s = \"a\\nb\";");
        assert!(
            found.iter().any(|capture| capture.token == "escape"),
            "{found:?}"
        );
        assert!(found.iter().any(|capture| capture.token == "string"));
    }

    #[test]
    fn a_path_extension_selects_a_language() {
        assert_eq!(language_for_path("src/main.rs"), Some("rust"));
        assert_eq!(language_for_path("/a/b/script.PY"), Some("python"));
        assert_eq!(language_for_path("a/b/c.tar.gz"), None);
        assert_eq!(language_for_path("Makefile"), None);
    }

    #[test]
    fn an_unknown_language_or_a_huge_source_has_no_captures() {
        assert!(captures("brainfuck", "++++").is_empty());
        assert!(captures("", "x").is_empty());
        let huge = "a".repeat(MAX_SOURCE_BYTES + 1);
        assert!(captures("rust", &huge).is_empty());
    }

    #[test]
    fn a_deeply_nested_source_falls_back_to_plain() {
        let mut source = "(".repeat(MAX_AST_DEPTH + 32);
        source.push('1');
        source.push_str(&")".repeat(MAX_AST_DEPTH + 32));
        assert!(captures("python", &source).is_empty());
    }
}
