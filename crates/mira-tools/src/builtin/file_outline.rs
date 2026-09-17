//! Structural outline of a single file — top-level definitions in
//! source order, plus one level of nesting (methods inside classes /
//! impl blocks).
//!
//! Same design ethos as `find_symbol`: a pragmatic regex-based
//! approximation of what an LSP `textDocument/documentSymbol` would
//! give, without the cost of running language servers. Good enough for
//! "give me a table of contents so I know where to jump," which is the
//! main model-facing need. A real LSP-backed variant is a follow-up.
//!
//! Supported: Rust, TypeScript / JavaScript (incl. TSX/JSX), Python,
//! Go. Unknown extensions return an empty outline with a clear note so
//! the model doesn't retry hoping for a different answer.

use std::path::Path;
use std::sync::LazyLock;

use async_trait::async_trait;
use mira_ai::ToolSpec;
use mira_core::{ToolCall, ToolResult};
use regex::Regex;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::fs;

use crate::context::ToolContext;
use crate::tool::{spec, Action, Tool, ToolError};

pub struct FileOutline;

#[derive(Deserialize)]
struct Args {
    path: String,
}

#[async_trait]
impl Tool for FileOutline {
    fn spec(&self) -> ToolSpec {
        spec(
            "file_outline",
            "Return a structural outline of a source file: top-level \
             functions, types, classes, and their methods, in source \
             order. Fast alternative to reading the whole file when you \
             just need a table of contents. Supports Rust, TypeScript/\
             JavaScript, Python, Go.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        )
    }

    fn action(&self) -> Action {
        Action::Read
    }

    async fn invoke(&self, call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let args: Args = call.parse_arguments()?;
        let path = ctx
            .resolve(&args.path)
            .ok_or_else(|| ToolError::Failed(format!("path escapes cwd: {}", args.path)))?;

        let contents = fs::read_to_string(&path)
            .await
            .map_err(|e| ToolError::Failed(format!("cannot read {}: {e}", path.display())))?;

        let lang = Language::detect(&path);
        let symbols = match lang {
            Some(l) => outline(&contents, l),
            None => Vec::new(),
        };

        let body = render(&args.path, lang, &symbols);
        let data = json!({
            "path": args.path,
            "language": lang.map(|l| l.as_str()),
            "symbols": symbols
                .iter()
                .map(|s| json!({
                    "kind": s.kind.as_str(),
                    "name": s.name,
                    "line": s.line,
                    "container": s.container,
                }))
                .collect::<Value>(),
        });
        Ok(ToolResult {
            call_id: call.id.clone(),
            content: body,
            is_error: false,
            data: Some(data),
        })
    }
}

// ---------- IR ----------

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Language {
    Rust,
    Ts,
    Js,
    Python,
    Go,
}

impl Language {
    fn detect(path: &Path) -> Option<Self> {
        match path.extension().and_then(|e| e.to_str()) {
            Some("rs") => Some(Language::Rust),
            Some("ts" | "tsx" | "mts" | "cts") => Some(Language::Ts),
            Some("js" | "jsx" | "mjs" | "cjs") => Some(Language::Js),
            Some("py" | "pyi") => Some(Language::Python),
            Some("go") => Some(Language::Go),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Language::Rust => "rust",
            Language::Ts => "typescript",
            Language::Js => "javascript",
            Language::Python => "python",
            Language::Go => "go",
        }
    }
}

#[derive(Debug, Clone)]
struct Symbol {
    name: String,
    kind: SymbolKind,
    /// 1-based line number where the definition starts.
    line: u32,
    /// If this is a nested member (e.g. a method inside a class),
    /// the parent's name. `None` for top-level.
    container: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Impl,
    Module,
    TypeAlias,
    Const,
}

impl SymbolKind {
    fn as_str(self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Method => "method",
            SymbolKind::Struct => "struct",
            SymbolKind::Enum => "enum",
            SymbolKind::Trait => "trait",
            SymbolKind::Interface => "interface",
            SymbolKind::Class => "class",
            SymbolKind::Impl => "impl",
            SymbolKind::Module => "module",
            SymbolKind::TypeAlias => "type",
            SymbolKind::Const => "const",
        }
    }
}

// ---------- Outline extraction ----------

fn outline(src: &str, lang: Language) -> Vec<Symbol> {
    match lang {
        Language::Rust => outline_rust(src),
        Language::Ts | Language::Js => outline_tsjs(src, lang == Language::Ts),
        Language::Python => outline_python(src),
        Language::Go => outline_go(src),
    }
}

/// Track container context using a stack of `(kind_name, base_indent)`
/// so nested method regexes know which class/impl/trait they belong to.
/// Base indent = the number of leading whitespace columns on the
/// container's own definition line. A member is treated as belonging to
/// the container when its own indent is strictly greater. This is a
/// pragmatic-but-robust heuristic — good enough for well-formatted
/// source, degrades gracefully for weird indent.
struct ContainerStack {
    frames: Vec<(String, usize)>,
}

impl ContainerStack {
    fn new() -> Self {
        Self { frames: Vec::new() }
    }
    fn current(&self) -> Option<String> {
        self.frames.last().map(|(n, _)| n.clone())
    }
    fn push(&mut self, name: String, indent: usize) {
        self.frames.push((name, indent));
    }
    fn narrow_to(&mut self, indent: usize) {
        while let Some((_, base)) = self.frames.last() {
            if indent <= *base {
                self.frames.pop();
            } else {
                break;
            }
        }
    }
}

fn leading_indent(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

// ----- Rust -----

static RE_RUST_FN: LazyLock<Regex> = LazyLock::new(|| {
    // Note: we drop `extern "ABI" fn` (raw-string quoting collides with
    // the outer literal). Falls back to a plain-word match for the
    // rare `extern fn`; a `extern "C"` prefix is skipped, but the
    // outline still catches the fn on the *next* attempt via the
    // trailing `fn <name>` capture regardless of leading modifiers.
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+|const\s+|unsafe\s+|extern\s+)*fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static RE_RUST_STRUCT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static RE_RUST_ENUM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static RE_RUST_TRAIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?trait\s+([A-Za-z_][A-Za-z0-9_]*)")
        .unwrap()
});
static RE_RUST_IMPL: LazyLock<Regex> = LazyLock::new(|| {
    // Matches `impl Foo`, `impl<T> Foo`, `impl Trait for Type`, capturing the
    // most descriptive right-hand side ("Trait for Type" or just "Type").
    Regex::new(r"^\s*impl(?:<[^>]*>)?\s+([^{;\n]+?)\s*(?:\{|$)").unwrap()
});
static RE_RUST_MOD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{").unwrap()
});
static RE_RUST_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?type\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});
static RE_RUST_CONST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static)\s+([A-Z_][A-Z0-9_]*)").unwrap()
});

fn outline_rust(src: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut stack = ContainerStack::new();

    for (i, raw) in src.lines().enumerate() {
        let line_no = (i + 1) as u32;
        let indent = leading_indent(raw);
        stack.narrow_to(indent);

        if let Some(c) = RE_RUST_IMPL.captures(raw) {
            let full = c.get(1).unwrap().as_str().trim().to_owned();
            // For `impl Trait for Type`, the container "name" is the
            // type; for plain `impl Foo`, it's Foo. Keep the full text
            // as the display name.
            let container_name = full
                .rsplit(" for ")
                .next()
                .unwrap_or(&full)
                .trim()
                .to_owned();
            out.push(Symbol {
                name: full.clone(),
                kind: SymbolKind::Impl,
                line: line_no,
                container: stack.current(),
            });
            stack.push(container_name, indent);
            continue;
        }

        for (re, kind) in [
            (&*RE_RUST_STRUCT, SymbolKind::Struct),
            (&*RE_RUST_ENUM, SymbolKind::Enum),
            (&*RE_RUST_TRAIT, SymbolKind::Trait),
        ] {
            if let Some(c) = re.captures(raw) {
                let name = c.get(1).unwrap().as_str().to_owned();
                out.push(Symbol {
                    name: name.clone(),
                    kind,
                    line: line_no,
                    container: stack.current(),
                });
                if kind == SymbolKind::Trait {
                    // trait bodies can contain method decls we'd want
                    // to nest under it.
                    stack.push(name, indent);
                }
                continue;
            }
        }

        if let Some(c) = RE_RUST_MOD.captures(raw) {
            let name = c.get(1).unwrap().as_str().to_owned();
            out.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Module,
                line: line_no,
                container: stack.current(),
            });
            stack.push(name, indent);
            continue;
        }

        if let Some(c) = RE_RUST_FN.captures(raw) {
            let name = c.get(1).unwrap().as_str().to_owned();
            let kind = if stack.current().is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            out.push(Symbol {
                name,
                kind,
                line: line_no,
                container: stack.current(),
            });
            continue;
        }

        if let Some(c) = RE_RUST_TYPE.captures(raw) {
            out.push(Symbol {
                name: c.get(1).unwrap().as_str().to_owned(),
                kind: SymbolKind::TypeAlias,
                line: line_no,
                container: stack.current(),
            });
            continue;
        }
        if let Some(c) = RE_RUST_CONST.captures(raw) {
            out.push(Symbol {
                name: c.get(1).unwrap().as_str().to_owned(),
                kind: SymbolKind::Const,
                line: line_no,
                container: stack.current(),
            });
            continue;
        }
    }

    out
}

// ----- TypeScript / JavaScript -----

static RE_TSJS_FUNCTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:export\s+(?:default\s+)?)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)")
        .unwrap()
});
static RE_TSJS_CLASS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\s*(?:export\s+(?:default\s+)?)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)")
        .unwrap()
});
static RE_TSJS_INTERFACE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:export\s+)?interface\s+([A-Za-z_$][\w$]*)").unwrap());
static RE_TSJS_TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?:export\s+)?type\s+([A-Za-z_$][\w$]*)\s*=").unwrap());
static RE_TSJS_CONST_FN: LazyLock<Regex> = LazyLock::new(|| {
    // Arrow-function-as-const at top level.
    Regex::new(r"^\s*(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*=>").unwrap()
});
static RE_TSJS_METHOD: LazyLock<Regex> = LazyLock::new(|| {
    // Class-body method: identifier followed by `(...)` and optional
    // return type + `{`. Restricted to indented lines so we don't
    // catch top-level function calls.
    Regex::new(r"^\s+(?:public\s+|private\s+|protected\s+|static\s+|readonly\s+|async\s+|\*\s*|get\s+|set\s+)*([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*(?::\s*[^\{]+)?\s*\{").unwrap()
});

fn outline_tsjs(src: &str, is_ts: bool) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut stack = ContainerStack::new();
    for (i, raw) in src.lines().enumerate() {
        let line_no = (i + 1) as u32;
        let indent = leading_indent(raw);
        stack.narrow_to(indent);

        if let Some(c) = RE_TSJS_CLASS.captures(raw) {
            let name = c.get(1).unwrap().as_str().to_owned();
            out.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Class,
                line: line_no,
                container: stack.current(),
            });
            stack.push(name, indent);
            continue;
        }
        if is_ts {
            if let Some(c) = RE_TSJS_INTERFACE.captures(raw) {
                let name = c.get(1).unwrap().as_str().to_owned();
                out.push(Symbol {
                    name,
                    kind: SymbolKind::Interface,
                    line: line_no,
                    container: stack.current(),
                });
                continue;
            }
            if let Some(c) = RE_TSJS_TYPE.captures(raw) {
                out.push(Symbol {
                    name: c.get(1).unwrap().as_str().to_owned(),
                    kind: SymbolKind::TypeAlias,
                    line: line_no,
                    container: stack.current(),
                });
                continue;
            }
        }
        if let Some(c) = RE_TSJS_FUNCTION.captures(raw) {
            out.push(Symbol {
                name: c.get(1).unwrap().as_str().to_owned(),
                kind: SymbolKind::Function,
                line: line_no,
                container: stack.current(),
            });
            continue;
        }
        if let Some(c) = RE_TSJS_CONST_FN.captures(raw) {
            out.push(Symbol {
                name: c.get(1).unwrap().as_str().to_owned(),
                kind: SymbolKind::Function,
                line: line_no,
                container: stack.current(),
            });
            continue;
        }
        // Only try to match class methods when we're inside a class-
        // shaped container to avoid picking up random function-call
        // lines.
        if stack.current().is_some() {
            if let Some(c) = RE_TSJS_METHOD.captures(raw) {
                let name = c.get(1).unwrap().as_str();
                // Skip control-flow keywords that share the shape.
                if matches!(
                    name,
                    "if" | "for" | "while" | "switch" | "catch" | "return" | "function"
                ) {
                    continue;
                }
                out.push(Symbol {
                    name: name.to_owned(),
                    kind: SymbolKind::Method,
                    line: line_no,
                    container: stack.current(),
                });
                continue;
            }
        }
    }
    out
}

// ----- Python -----

static RE_PY_DEF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)(?:async\s+)?def\s+([A-Za-z_][\w]*)").unwrap());
static RE_PY_CLASS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\s*)class\s+([A-Za-z_][\w]*)").unwrap());

fn outline_python(src: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut stack = ContainerStack::new();
    for (i, raw) in src.lines().enumerate() {
        let line_no = (i + 1) as u32;
        let indent = leading_indent(raw);
        stack.narrow_to(indent);

        if let Some(c) = RE_PY_CLASS.captures(raw) {
            let name = c.get(2).unwrap().as_str().to_owned();
            out.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Class,
                line: line_no,
                container: stack.current(),
            });
            stack.push(name, indent);
            continue;
        }
        if let Some(c) = RE_PY_DEF.captures(raw) {
            let name = c.get(2).unwrap().as_str().to_owned();
            let kind = if stack.current().is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            out.push(Symbol {
                name,
                kind,
                line: line_no,
                container: stack.current(),
            });
            continue;
        }
    }
    out
}

// ----- Go -----

static RE_GO_FUNC: LazyLock<Regex> = LazyLock::new(|| {
    // Captures both `func Foo(...)` and `func (r Receiver) Method(...)`.
    Regex::new(r"^func\s+(?:\(([^)]+)\)\s+)?([A-Za-z_][\w]*)").unwrap()
});
static RE_GO_TYPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^type\s+([A-Za-z_][\w]*)\s+(struct|interface|=|[A-Za-z\[])").unwrap()
});

fn outline_go(src: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line_no = (i + 1) as u32;

        if let Some(c) = RE_GO_TYPE.captures(raw) {
            let name = c.get(1).unwrap().as_str().to_owned();
            let kind = match c.get(2).map(|m| m.as_str()) {
                Some("struct") => SymbolKind::Struct,
                Some("interface") => SymbolKind::Interface,
                _ => SymbolKind::TypeAlias,
            };
            out.push(Symbol {
                name,
                kind,
                line: line_no,
                container: None,
            });
            continue;
        }
        if let Some(c) = RE_GO_FUNC.captures(raw) {
            let container = c.get(1).map(|m| {
                // Receiver spec is like `(r *Receiver)` or `(Receiver)`;
                // grab the last identifier as the type name.
                let s = m.as_str();
                s.split_whitespace()
                    .last()
                    .map(|t| t.trim_start_matches('*').to_owned())
                    .unwrap_or_default()
            });
            let name = c.get(2).unwrap().as_str().to_owned();
            let kind = if container.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            out.push(Symbol {
                name,
                kind,
                line: line_no,
                container,
            });
        }
    }
    out
}

// ---------- Rendering ----------

fn render(path: &str, lang: Option<Language>, symbols: &[Symbol]) -> String {
    let Some(lang) = lang else {
        return format!(
            "{path}: unsupported language for outline (supports .rs, .ts/.tsx/.js/.jsx, .py, .go)"
        );
    };
    if symbols.is_empty() {
        return format!("{path} ({}): no top-level definitions found", lang.as_str());
    }
    let mut s = format!("{path} ({}):\n", lang.as_str());
    for sym in symbols {
        let prefix = if sym.container.is_some() { "  " } else { "" };
        s.push_str(&format!(
            "{prefix}L{:<5} {:<9} {}\n",
            sym.line,
            sym.kind.as_str(),
            sym.name
        ));
    }
    s
}

// ---------- Tests ----------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_top_level_and_impl_methods() {
        let src = r#"
pub struct Foo { x: u32 }

impl Foo {
    pub fn new() -> Self { Self { x: 0 } }
    fn helper(&self) -> u32 { self.x }
}

pub trait Bar {
    fn required(&self) -> bool;
}

impl Bar for Foo {
    fn required(&self) -> bool { true }
}

fn free_function() {}

pub const MAX: usize = 10;
"#;
        let syms = outline_rust(src);
        let kinds: Vec<_> = syms.iter().map(|s| (s.kind, s.name.as_str())).collect();
        assert!(kinds.contains(&(SymbolKind::Struct, "Foo")));
        assert!(kinds.contains(&(SymbolKind::Trait, "Bar")));
        assert!(kinds.contains(&(SymbolKind::Function, "free_function")));
        assert!(kinds.contains(&(SymbolKind::Const, "MAX")));
        // Methods must be tagged as such and carry a container.
        let new_method = syms.iter().find(|s| s.name == "new").unwrap();
        assert_eq!(new_method.kind, SymbolKind::Method);
        assert_eq!(new_method.container.as_deref(), Some("Foo"));
        // The trait's own `required` decl nests under the trait; the
        // `impl Bar for Foo` block's `required` nests under Foo. Both
        // are `Method` kind — assert both containers appear.
        let containers: Vec<_> = syms
            .iter()
            .filter(|s| s.name == "required" && s.kind == SymbolKind::Method)
            .filter_map(|s| s.container.as_deref())
            .collect();
        assert!(
            containers.contains(&"Bar"),
            "no trait method container: {containers:?}"
        );
        assert!(
            containers.contains(&"Foo"),
            "no impl method container: {containers:?}"
        );
    }

    #[test]
    fn typescript_class_and_functions() {
        let src = r#"
export interface Base { id: string }
export type Alias = { a: number };

export class Foo extends Base {
  private count: number = 0;
  public greet(name: string): void { console.log(name); }
  async load(id: string): Promise<void> {}
}

export function top() {}
export const arrow = (x: number) => x + 1;
"#;
        let syms = outline_tsjs(src, true);
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Base"));
        assert!(names.contains(&"Alias"));
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"top"));
        assert!(names.contains(&"arrow"));
        let greet = syms.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(greet.kind, SymbolKind::Method);
        assert_eq!(greet.container.as_deref(), Some("Foo"));
    }

    #[test]
    fn python_methods_scoped_to_class() {
        let src = "\
class Foo:
    def a(self):
        pass
    def b(self):
        pass

def free():
    pass
";
        let syms = outline_python(src);
        let free = syms.iter().find(|s| s.name == "free").unwrap();
        assert_eq!(free.kind, SymbolKind::Function);
        let a = syms.iter().find(|s| s.name == "a").unwrap();
        assert_eq!(a.kind, SymbolKind::Method);
        assert_eq!(a.container.as_deref(), Some("Foo"));
    }

    #[test]
    fn go_methods_use_receiver_type() {
        let src = "\
package p

type Foo struct { X int }
type Iface interface { Do() }

func (f *Foo) Do() {}
func Free() {}
";
        let syms = outline_go(src);
        let foo = syms.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.kind, SymbolKind::Struct);
        let iface = syms.iter().find(|s| s.name == "Iface").unwrap();
        assert_eq!(iface.kind, SymbolKind::Interface);
        let do_m = syms
            .iter()
            .find(|s| s.name == "Do" && s.container.is_some())
            .unwrap();
        assert_eq!(do_m.kind, SymbolKind::Method);
        assert_eq!(do_m.container.as_deref(), Some("Foo"));
        let free = syms.iter().find(|s| s.name == "Free").unwrap();
        assert_eq!(free.kind, SymbolKind::Function);
    }

    #[test]
    fn unknown_extension_returns_note() {
        let s = render("x.zzz", None, &[]);
        assert!(s.contains("unsupported language"), "{s}");
    }
}
