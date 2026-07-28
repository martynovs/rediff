//! Map a file path to a language id used later by the highlighter.
//!
//! The id is resolved in two steps: a bundled tree-sitter grammar if one matches,
//! otherwise syntect's syntax set (bat's assets, via `two-face`), which knows a
//! few hundred languages by extension or name.
//!
//! So this table only carries the cases syntect would get *wrong* — the
//! tree-sitter ids, and extensions registered under a different syntax. Everything
//! else falls through as the bare extension for syntect to resolve. The default
//! must be the fallthrough rather than `None`, because `None` means "render as
//! plain text": an unlisted extension is far likelier to be a language bat knows
//! than one nobody does.

/// Best-effort language id from a path.
///
/// Returns `None` only when there is no file name to go on.
pub fn detect(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().filter(|n| !n.is_empty())?;
    // A dotless name is its own token, so `Makefile` and `Dockerfile` resolve too.
    let token = match name.rsplit_once('.') {
        Some((_, ext)) if !ext.is_empty() => ext,
        _ => name,
    };
    let token = token.to_ascii_lowercase();

    let mapped = match token.as_str() {
        // Bundled tree-sitter grammars: these ids are what `ts_key` matches.
        "rs" => "rust",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "jsx",
        // Extensions whose syntect syntax is not registered under the extension.
        "h" => "c",
        "hpp" | "hh" | "hxx" | "cc" => "cpp",
        "md" => "markdown",
        "yml" => "yaml",
        // Everything else: hand syntect the extension and let it decide.
        _ => return Some(token),
    };
    Some(mapped.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::{Engine, Highlight};

    #[test]
    fn tree_sitter_ids_are_mapped_not_passed_through() {
        assert_eq!(detect("src/main.rs").as_deref(), Some("rust"));
        assert_eq!(detect("a/b/app.tsx").as_deref(), Some("tsx"));
        assert_eq!(detect("x.mts").as_deref(), Some("typescript"));
        assert_eq!(detect("x.cjs").as_deref(), Some("javascript"));
        assert_eq!(detect("x.jsx").as_deref(), Some("jsx"));
    }

    #[test]
    fn unlisted_extensions_fall_through_lowercased() {
        assert_eq!(detect("Program.cs").as_deref(), Some("cs"));
        assert_eq!(detect("Main.JAVA").as_deref(), Some("java"));
        assert_eq!(detect("q.sql").as_deref(), Some("sql"));
    }

    #[test]
    fn dotless_names_are_their_own_token() {
        assert_eq!(detect("Makefile").as_deref(), Some("makefile"));
        assert_eq!(detect("build/Dockerfile").as_deref(), Some("dockerfile"));
        assert_eq!(detect(".gitignore").as_deref(), Some("gitignore"));
    }

    #[test]
    fn a_path_with_no_name_yields_nothing() {
        assert_eq!(detect(""), None);
        assert_eq!(detect("some/dir/"), None);
    }

    /// Whether the highlighter tokenizes `src` into more than one span — i.e.
    /// whether the path's language resolved to a real syntax.
    fn highlights(path: &str, src: &str) -> bool {
        let engine = Engine::new();
        let lang = detect(path);
        let lines = engine.highlight(
            src,
            lang.as_deref(),
            two_face::theme::EmbeddedThemeName::Nord,
        );
        lines.first().is_some_and(|spans| spans.len() > 1)
    }

    #[test]
    fn the_languages_this_table_claims_to_support_actually_highlight() {
        // Each snippet carries at least one keyword, so a resolved syntax must
        // produce more than one undifferentiated span. This is the test that would
        // have caught `.cs` rendering as plain text.
        let cases: &[(&str, &str)] = &[
            ("a.rs", "fn main() { let x = 1; }"),
            ("a.ts", "const x: number = 1;"),
            ("a.tsx", "const A = () => 1;"),
            ("a.js", "function f() { return 1; }"),
            ("a.py", "def f():\n    return 1"),
            ("a.go", "func main() { return }"),
            ("a.c", "int main(void) { return 0; }"),
            ("a.h", "#define X 1"),
            ("a.cpp", "int main() { return 0; }"),
            ("a.hpp", "class X { public: int y; };"),
            (
                "a.cs",
                "public class Foo { public int Bar() { return 1; } }",
            ),
            ("a.java", "public class Foo { int bar() { return 1; } }"),
            ("a.rb", "def foo\n  1\nend"),
            ("a.php", "<?php function f() { return 1; }"),
            ("a.swift", "func f() -> Int { return 1 }"),
            ("a.scala", "object Foo { def bar = 1 }"),
            ("a.lua", "local function f() return 1 end"),
            ("a.sql", "SELECT * FROM t WHERE x = 1;"),
            ("a.xml", "<root><child a=\"b\"/></root>"),
            ("a.html", "<div class=\"x\">hi</div>"),
            ("a.css", "body { color: red; }"),
            ("a.json", "{\"a\": 1}"),
            ("a.toml", "[x]\ny = 1"),
            ("a.yaml", "a: 1\nb: two"),
            ("a.yml", "a: 1"),
            ("a.md", "# Title\n\nsome *text*"),
            ("a.sh", "if [ -n \"$x\" ]; then echo hi; fi"),
            ("a.bash", "echo \"$HOME\""),
            ("a.hs", "main :: IO ()\nmain = return ()"),
            ("a.erl", "-module(f).\nf() -> 1."),
            ("a.dart", "void main() { var x = 1; }"),
            ("a.pl", "sub f { return 1; }"),
            ("a.ml", "let f () = 1"),
            ("a.clj", "(defn f [] 1)"),
            ("a.groovy", "def f() { return 1 }"),
            ("a.tex", "\\section{Hi}"),
            ("a.diff", "@@ -1 +1 @@\n-a\n+b"),
            ("a.kt", "fun main() { val x = 1 }"),
            ("a.ex", "defmodule F do\n  def f, do: 1\nend"),
            ("a.jl", "function f()\n  1\nend"),
            ("a.r", "f <- function() { 1 }"),
            ("a.nix", "{ pkgs }: { x = 1; }"),
            ("a.zig", "pub fn main() void {}"),
            ("a.proto", "message M { int32 a = 1; }"),
            ("a.vim", "function! F()\nendfunction"),
            ("Makefile", "all:\n\techo hi"),
            ("Dockerfile", "FROM alpine\nRUN echo hi"),
        ];

        let plain: Vec<&str> = cases
            .iter()
            .filter(|(path, src)| !highlights(path, src))
            .map(|(path, _)| *path)
            .collect();
        assert!(
            plain.is_empty(),
            "these resolved to no syntax and rendered as plain text: {plain:?}"
        );
    }

    #[test]
    fn a_genuinely_unknown_extension_is_plain_but_harmless() {
        // The fallthrough hands syntect a token it cannot match; the highlighter
        // falls to plain rather than erroring.
        assert_eq!(detect("a.zzzznope").as_deref(), Some("zzzznope"));
        assert!(!highlights("a.zzzznope", "some text here"));
    }
}
