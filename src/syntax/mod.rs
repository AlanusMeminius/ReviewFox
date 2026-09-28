//! Syntax highlighting for Diff code text. Pure (no GPUI). See ADR-0010.
//!
//! Each Diff side is a read-only snapshot (ADR-0005), so a text is parsed and
//! highlighted once into sorted spans over the whole text; rows then take
//! their slice with [`spans_in`]. No incremental reparse, no injections.
//!
//! Capture names are shared across Languages: every query is configured with
//! the union of all queries' names, so a [`CaptureId`] means the same name
//! whatever the Language and a palette can resolve it once per id.

use std::ops::Range;
use std::path::Path;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

/// Bytes / lines / longest-line limits; a side over any stays plain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeGuard {
    pub max_bytes: usize,
    pub max_lines: usize,
    pub max_line_bytes: usize,
}

/// Defaults from issue 04 measurements (see issue Comments).
pub const DEFAULT_SIZE_GUARD: SizeGuard = SizeGuard {
    max_bytes: 512 * 1024,
    max_lines: 20_000,
    max_line_bytes: 8_192,
};

/// `true` when `text` should stay plain (any limit exceeded).
pub fn exceeds_size_guard(text: &str, guard: SizeGuard) -> bool {
    if text.len() > guard.max_bytes {
        return true;
    }
    let mut lines = 0usize;
    let mut max_line = 0usize;
    for line in text.lines() {
        lines += 1;
        max_line = max_line.max(line.len());
        if lines > guard.max_lines || max_line > guard.max_line_bytes {
            return true;
        }
    }
    false
}

/// Force query compile on a background thread before the first open.
pub fn warm() {
    let _ = capture_names();
}

/// How many times [`highlight`] has run (acceptance: fold / ignore-ws must not bump).
pub fn highlight_count() -> u64 {
    HIGHLIGHT_COUNT.load(Ordering::Relaxed)
}

static HIGHLIGHT_COUNT: AtomicU64 = AtomicU64::new(0);

/// A language with a compiled-in grammar and highlights query.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Language {
    Rust = 0,
    Cpp = 1,
    CMake = 2,
}

/// A highlight capture, e.g. `keyword` or `function.method`; an index into
/// [`capture_names`], shared by every Language.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CaptureId(pub u16);

/// One highlighted byte range of the text given to [`highlight`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub range: Range<usize>,
    pub capture: CaptureId,
}

impl Language {
    const ALL: [Language; 3] = [Language::Rust, Language::Cpp, Language::CMake];

    /// Grammar and highlights query. `None` if the query fails to compile;
    /// that Language then stays plain without poisoning the others.
    fn config(self) -> Option<HighlightConfiguration> {
        let (grammar, query) = match self {
            Language::Rust => (
                tree_sitter_rust::LANGUAGE.into(),
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_string(),
            ),
            // tree-sitter-cpp's query inherits C's; alone it misses keywords,
            // strings and comments. C first, as the tree-sitter CLI does.
            Language::Cpp => (
                tree_sitter_cpp::LANGUAGE.into(),
                format!(
                    "{}\n{}",
                    tree_sitter_c::HIGHLIGHT_QUERY,
                    tree_sitter_cpp::HIGHLIGHT_QUERY
                ),
            ),
            // Upstream's query is Neovim-flavored; ours is repo-owned.
            Language::CMake => (
                tree_sitter_cmake::LANGUAGE.into(),
                include_str!("../../assets/queries/cmake/highlights.scm").to_string(),
            ),
        };
        match HighlightConfiguration::new(grammar, format!("{self:?}"), &query, "", "") {
            Ok(c) => Some(c),
            Err(e) => {
                log::warn!("{self:?} highlights query: {e}");
                None
            }
        }
    }
}

/// Compiled queries for every Language plus the shared capture-name list.
struct Registry {
    /// Indexed by [`Language`] as `usize`. `None` means that Language's query
    /// failed to compile; highlighting falls back to plain text.
    configs: Vec<Option<HighlightConfiguration>>,
    names: Vec<String>,
}

static REGISTRY: LazyLock<Registry> = LazyLock::new(Registry::new);

impl Registry {
    fn new() -> Self {
        let mut configs: Vec<_> = Language::ALL.iter().map(|l| l.config()).collect();
        // `_`-prefixed captures are query-internal (predicate helpers).
        let mut names: Vec<String> = configs
            .iter()
            .flatten()
            .flat_map(|c| c.names())
            .filter(|n| !n.starts_with('_'))
            .map(|n| n.to_string())
            .collect();
        names.sort();
        names.dedup();
        assert!(
            names.len() <= usize::from(u16::MAX),
            "too many capture names"
        );
        // Every capture is in `names`, so each resolves to exactly itself.
        for c in configs.iter_mut().flatten() {
            c.configure(&names);
        }
        Self { configs, names }
    }

    fn config(&self, lang: Language) -> Option<&HighlightConfiguration> {
        self.configs[lang as usize].as_ref()
    }
}

/// Every capture name any query can emit, sorted; indexed by [`CaptureId`].
pub fn capture_names() -> &'static [String] {
    &REGISTRY.names
}

pub fn capture_name(id: CaptureId) -> &'static str {
    &REGISTRY.names[usize::from(id.0)]
}

/// Highlight all of `text` as `lang`: sorted, non-overlapping byte ranges;
/// where captures nest, the innermost wins. Adjacent ranges with the same
/// capture are merged. Broken syntax still yields spans (tree-sitter recovers);
/// a highlighter error keeps the spans found so far. Unknown / failed query
/// Languages yield no spans (plain text).
pub fn highlight(lang: Language, text: &str) -> Vec<Span> {
    HIGHLIGHT_COUNT.fetch_add(1, Ordering::Relaxed);
    let Some(config) = REGISTRY.config(lang) else {
        return Vec::new();
    };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(config, text.as_bytes(), None, None, |_| None) else {
        log::debug!("highlighter failed for {lang:?}");
        return Vec::new();
    };
    let mut spans: Vec<Span> = Vec::new();
    let mut stack: Vec<CaptureId> = Vec::new();
    for event in events {
        match event {
            Ok(HighlightEvent::HighlightStart(h)) => stack.push(CaptureId(h.0 as u16)),
            Ok(HighlightEvent::HighlightEnd) => {
                stack.pop();
            }
            Ok(HighlightEvent::Source { start, end }) => {
                let Some(&capture) = stack.last() else {
                    continue;
                };
                match spans.last_mut() {
                    Some(last) if last.capture == capture && last.range.end == start => {
                        last.range.end = end;
                    }
                    _ => spans.push(Span {
                        range: start..end,
                        capture,
                    }),
                }
            }
            Err(e) => {
                log::debug!("highlight event error for {lang:?}: {e}");
                break;
            }
        }
    }
    spans
}

/// The parts of `spans` (from [`highlight`]) inside `line`, a byte range of
/// the same text, clipped and relative to `line.start`. Binary-searches for
/// the first span, so a row costs O(log spans + spans on the row).
pub fn spans_in(
    spans: &[Span],
    line: Range<usize>,
) -> impl Iterator<Item = (Range<usize>, CaptureId)> + '_ {
    let first = spans.partition_point(|s| s.range.end <= line.start);
    spans[first..]
        .iter()
        .take_while(move |s| s.range.start < line.end)
        .map(move |s| {
            let start = s.range.start.max(line.start) - line.start;
            let end = s.range.end.min(line.end) - line.start;
            (start..end, s.capture)
        })
        // An empty `line` inside a span would otherwise yield `0..0`.
        .filter(|(r, _)| !r.is_empty())
}

/// Language of the file at `path`: extension, then special file name, then
/// the `first_line` shebang. `None` renders as plain text.
pub fn detect(path: &Path, first_line: &str) -> Option<Language> {
    let name = path.file_name()?.to_str()?;
    let ext = path.extension().and_then(|e| e.to_str());
    let by_ext = match ext {
        Some("rs") => Some(Language::Rust),
        Some("c" | "h" | "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "inl" | "ipp") => {
            Some(Language::Cpp)
        }
        Some("cmake") => Some(Language::CMake),
        _ => None,
    };
    by_ext
        .or_else(|| match name {
            "CMakeLists.txt" => Some(Language::CMake),
            _ if name.ends_with(".cmake.in") => Some(Language::CMake),
            _ => None,
        })
        .or_else(|| shebang_interpreter(first_line).and_then(from_interpreter))
}

/// Interpreter named by a `#!` line: the program's file name, or the first
/// argument of `env` (`#!/usr/bin/env -S python3 -u` → `python3`).
fn shebang_interpreter(first_line: &str) -> Option<&str> {
    let mut words = first_line.strip_prefix("#!")?.split_whitespace();
    let program = words.next()?.rsplit('/').next()?;
    if program != "env" {
        return Some(program);
    }
    words.find(|w| !w.starts_with('-'))
}

/// Shebang interpreter → Language. Spec hook for `bash` / `sh` / `python` /
/// `node`; none have a v1 grammar yet, so every name resolves to `None`.
fn from_interpreter(_name: &str) -> Option<Language> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(text, capture name)` of every span.
    fn named(text: &str, spans: &[Span]) -> Vec<(String, &'static str)> {
        spans
            .iter()
            .map(|s| (text[s.range.clone()].to_string(), capture_name(s.capture)))
            .collect()
    }

    fn has(text: &str, spans: &[Span], token: &str, capture: &str) -> bool {
        named(text, spans)
            .iter()
            .any(|(t, c)| t == token && *c == capture)
    }

    #[test]
    fn every_bundled_query_compiles() {
        for lang in Language::ALL {
            assert!(
                REGISTRY.config(lang).is_some(),
                "{lang:?} highlights query failed to compile"
            );
        }
    }

    #[test]
    fn rust_keywords_strings_and_comments() {
        let text = "fn main() {\n    let s = \"hi\"; // note\n}\n";
        let spans = highlight(Language::Rust, text);
        for (token, capture) in [
            ("fn", "keyword"),
            ("let", "keyword"),
            ("\"hi\"", "string"),
            ("// note", "comment"),
            ("main", "function"),
        ] {
            assert!(
                has(text, &spans, token, capture),
                "{token}: {:?}",
                named(text, &spans)
            );
        }
    }

    #[test]
    fn cpp_gets_c_level_captures() {
        // `return`, `if`, strings and comments come only from the C query;
        // `namespace` / `class` only from the C++ one.
        let text = "namespace n {\nclass A {};\nint f(int x) {\n  if (x) return 1; // c\n  \
                    puts(\"s\");\n}\n}\n";
        let spans = highlight(Language::Cpp, text);
        for (token, capture) in [
            ("return", "keyword"),
            ("if", "keyword"),
            ("\"s\"", "string"),
            ("// c", "comment"),
            ("namespace", "keyword"),
            ("class", "keyword"),
        ] {
            assert!(
                has(text, &spans, token, capture),
                "{token}: {:?}",
                named(text, &spans)
            );
        }
    }

    #[test]
    fn cmake_highlights_each_category() {
        // Covers comments, strings, functions, control-flow keywords, variable
        // refs, and ALL_CAPS constants. Generator expressions `$<...>` are not
        // a grammar node in tree-sitter-cmake 0.7.5, so they stay uncolored.
        let text = "\
# line comment
#[[bracket comment]]
project(x \"quoted\" [=[bracket]=])
if(ON)
  set(V ${FOO} $ENV{PATH} $CACHE{BAR})
  target_link_libraries(t PUBLIC req)
elseif(OFF)
else()
endif()
foreach(i)
endforeach()
while(0)
endwhile()
function(f)
  return()
endfunction()
macro(m)
endmacro()
";
        let spans = highlight(Language::CMake, text);
        let got = named(text, &spans);
        for (token, capture) in [
            ("# line comment", "comment"),
            ("#[[bracket comment]]", "comment"),
            ("\"quoted\"", "string"),
            ("[=[bracket]=]", "string"),
            ("project", "function"),
            ("set", "function"),
            ("target_link_libraries", "function"),
            ("if", "keyword"),
            ("elseif", "keyword"),
            ("else", "keyword"),
            ("endif", "keyword"),
            ("foreach", "keyword"),
            ("endforeach", "keyword"),
            ("while", "keyword"),
            ("endwhile", "keyword"),
            ("function", "keyword"),
            ("endfunction", "keyword"),
            ("macro", "keyword"),
            ("endmacro", "keyword"),
            ("return", "keyword"),
            ("${FOO}", "variable"),
            ("$ENV{PATH}", "variable"),
            ("$CACHE{BAR}", "variable"),
            ("PUBLIC", "constant"),
        ] {
            assert!(
                has(text, &spans, token, capture),
                "{token}@{capture}: {got:?}"
            );
        }
    }

    /// Byte range of each line of `text`, without the `\n`.
    fn lines(text: &str) -> Vec<Range<usize>> {
        let mut start = 0;
        text.split('\n')
            .map(|l| {
                let r = start..start + l.len();
                start = r.end + 1;
                r
            })
            .collect()
    }

    #[test]
    fn block_comment_spans_every_line_it_covers() {
        let text = "let a = 1; /* one\ntwo\nthree */ let b;\n";
        let spans = highlight(Language::Rust, text);
        let comment = |r: Range<usize>| {
            spans_in(&spans, r)
                .find(|(_, c)| capture_name(*c) == "comment")
                .map(|(r, _)| r)
        };
        let ls = lines(text);
        // Ranges are relative to the line start.
        assert_eq!(
            comment(ls[0].clone()),
            Some(11..17),
            "{:?}",
            named(text, &spans)
        );
        assert_eq!(comment(ls[1].clone()), Some(0..3));
        assert_eq!(comment(ls[2].clone()), Some(0..8));
        // `let` after the comment on line 3 keeps its own capture.
        assert!(
            spans_in(&spans, ls[2].clone())
                .any(|(r, c)| r == (9..12) && capture_name(c) == "keyword")
        );
        assert_eq!(spans_in(&spans, ls[3].clone()).count(), 0);
    }

    #[test]
    fn spans_in_clips_and_sorts() {
        let s = |a, b, c| Span {
            range: a..b,
            capture: CaptureId(c),
        };
        let spans = [s(0, 4, 1), s(6, 12, 2), s(14, 15, 3), s(20, 30, 4)];
        let got = |r: Range<usize>| spans_in(&spans, r).collect::<Vec<_>>();
        assert_eq!(got(2..10), [(0..2, CaptureId(1)), (4..8, CaptureId(2))]);
        assert_eq!(got(12..14), []);
        assert_eq!(got(14..15), [(0..1, CaptureId(3))]);
        assert_eq!(got(25..27), [(0..2, CaptureId(4))]);
        assert_eq!(got(40..50), []);
        assert_eq!(got(3..3), []);
    }

    /// Sorted, non-overlapping, non-empty, inside `text`, on char boundaries.
    fn assert_well_formed(text: &str, spans: &[Span]) {
        for s in spans {
            assert!(
                s.range.start < s.range.end && s.range.end <= text.len(),
                "{s:?}"
            );
            assert!(text.is_char_boundary(s.range.start) && text.is_char_boundary(s.range.end));
        }
        for w in spans.windows(2) {
            assert!(w[0].range.end <= w[1].range.start, "{w:?}");
        }
    }

    #[test]
    fn innermost_capture_wins() {
        // The escape sits inside the string node; it splits the string span.
        let text = "let s = \"a\\nb\";";
        let spans = highlight(Language::Rust, text);
        assert_well_formed(text, &spans);
        let got = named(text, &spans);
        let i = got
            .iter()
            .position(|(t, _)| t == "\\n")
            .expect("escape span");
        assert_eq!(got[i].1, "escape");
        assert_eq!(got[i - 1], ("\"a".to_string(), "string"));
        assert_eq!(got[i + 1], ("b\"".to_string(), "string"));
    }

    #[test]
    fn empty_text_has_no_spans() {
        for lang in Language::ALL {
            assert_eq!(highlight(lang, ""), [], "{lang:?}");
        }
    }

    #[test]
    fn broken_input_still_highlights() {
        let cases = [
            (Language::Rust, "fn (( { let \"unterminated\n/* open", "fn"),
            (Language::Cpp, "int f( { return \"x\" ;;; }}} #if", "return"),
            (Language::CMake, "if(( \"a\" # c\n)))", "\"a\""),
        ];
        for (lang, text, token) in cases {
            let spans = highlight(lang, text);
            assert_well_formed(text, &spans);
            assert!(
                named(text, &spans).iter().any(|(t, _)| t == token),
                "{lang:?}: {spans:?}"
            );
        }
    }

    #[test]
    fn multibyte_text_keeps_char_boundaries() {
        let text = "// 注释\nlet s = \"中文\"; /* ü */\n";
        let spans = highlight(Language::Rust, text);
        assert_well_formed(text, &spans);
        assert!(has(text, &spans, "\"中文\"", "string"));
        assert_eq!(named(text, &spans)[0], ("// 注释".to_string(), "comment"));
    }

    #[test]
    fn detects_every_listed_file_kind() {
        let cases = [
            ("src/main.rs", Some(Language::Rust)),
            ("a.c", Some(Language::Cpp)),
            ("a.h", Some(Language::Cpp)),
            ("a.cpp", Some(Language::Cpp)),
            ("a.cc", Some(Language::Cpp)),
            ("a.cxx", Some(Language::Cpp)),
            ("a.hpp", Some(Language::Cpp)),
            ("a.hh", Some(Language::Cpp)),
            ("a.hxx", Some(Language::Cpp)),
            ("a.inl", Some(Language::Cpp)),
            ("a.ipp", Some(Language::Cpp)),
            ("CMakeLists.txt", Some(Language::CMake)),
            ("sub/dir/CMakeLists.txt", Some(Language::CMake)),
            ("cmake/Find.cmake", Some(Language::CMake)),
            ("config.cmake.in", Some(Language::CMake)),
            ("README.md", None),
            ("notes.txt", None),
            ("Makefile", None),
            ("a.in", None),
            // Listed extensions match exactly, lower case.
            ("A.RS", None),
            ("a.C", None),
            ("cmakelists.txt", None),
        ];
        for (path, want) in cases {
            assert_eq!(detect(Path::new(path), ""), want, "{path}");
        }
    }

    #[test]
    fn shebang_hook_parses_but_resolves_nothing_in_v1() {
        let cases = [
            ("#!/bin/bash", Some("bash")),
            ("#!/bin/sh -e", Some("sh")),
            ("#!/usr/bin/env python3", Some("python3")),
            ("#!/usr/bin/env -S node --flag", Some("node")),
            ("#!", None),
            ("fn main() {}", None),
        ];
        for (line, want) in cases {
            assert_eq!(shebang_interpreter(line), want, "{line}");
            assert_eq!(detect(Path::new("script"), line), None, "{line}");
        }
        // Extension wins over the first line.
        assert_eq!(detect(Path::new("x.rs"), "#!/bin/sh"), Some(Language::Rust));
    }

    /// Tiny guard so boundary cases stay readable.
    fn g(bytes: usize, lines: usize, line_bytes: usize) -> SizeGuard {
        SizeGuard {
            max_bytes: bytes,
            max_lines: lines,
            max_line_bytes: line_bytes,
        }
    }

    #[test]
    fn size_guard_boundaries() {
        // (label, text, guard, exceeds)
        let cases: &[(&str, &str, SizeGuard, bool)] = &[
            ("empty", "", g(0, 0, 0), false),
            ("bytes_eq", "abcd", g(4, 100, 100), false),
            ("bytes_over", "abcde", g(4, 100, 100), true),
            ("lines_eq", "a\nb\nc", g(100, 3, 100), false),
            ("lines_over", "a\nb\nc\nd", g(100, 3, 100), true),
            ("line_eq", "abcd", g(100, 100, 4), false),
            ("line_over", "abcde", g(100, 100, 4), true),
            ("longest_of_many", "a\nbbbbb\nc", g(100, 100, 4), true),
            ("crlf_line", "abcd\r\nef", g(100, 100, 4), false),
            ("crlf_line_over", "abcde\r\nef", g(100, 100, 4), true),
        ];
        for (label, text, guard, want) in cases {
            assert_eq!(
                exceeds_size_guard(text, *guard),
                *want,
                "{label}: {text:?}"
            );
        }
    }

    #[test]
    fn highlight_count_bumps_once_per_call() {
        let before = highlight_count();
        let _ = highlight(Language::Rust, "fn x() {}");
        let after = highlight_count();
        // Other tests may call `highlight` in parallel; require a bump of ≥1.
        assert!(after >= before + 1, "before={before} after={after}");
    }

    /// `cargo test --bin reviewfox warm_compile_bench --release -- --ignored --nocapture`
    #[test]
    #[ignore = "manual query-compile timing; not part of CI"]
    fn warm_compile_bench() {
        use std::time::Instant;
        let t = Instant::now();
        warm();
        eprintln!(
            "[bench] syntax::warm (query compile) {:.1}ms",
            t.elapsed().as_secs_f64() * 1e3
        );
    }

    /// `cargo test --bin reviewfox highlight_size_bench --release -- --ignored --nocapture`
    #[test]
    #[ignore = "manual size-guard timing; not part of CI"]
    fn highlight_size_bench() {
        use std::time::Instant;

        fn stats(text: &str) -> (usize, usize, usize) {
            let lines = text.lines().count();
            let max_line = text.lines().map(|l| l.len()).max().unwrap_or(0);
            (text.len(), lines, max_line)
        }

        fn run(label: &str, lang: Language, text: &str) {
            let (bytes, lines, max_line) = stats(text);
            // Warm registry once outside the timed call for fair per-file numbers.
            let _ = capture_names();
            let t = Instant::now();
            let spans = highlight(lang, text);
            let ms = t.elapsed().as_secs_f64() * 1e3;
            eprintln!(
                "[bench] {label}: {bytes} bytes, {lines} lines, max_line {max_line} → {:.1}ms, {} spans",
                ms,
                spans.len()
            );
        }

        let app_view = include_str!("../ui/app_view.rs");
        run("repo app_view.rs", Language::Rust, app_view);

        // Synthetic scales: repeated real-ish Rust / C++ so release timings
        // stay comparable without depending on cargo registry paths.
        let rust_unit = "fn foo(x: i32) -> i32 {\n    // c\n    let s = \"hi\";\n    x + 1\n}\n";
        let cpp_unit = "namespace n {\nint f(int x) {\n  // c\n  if (x) return 1;\n  puts(\"s\");\n}\n}\n";
        for (label, lang, unit, n) in [
            ("rust×2k lines", Language::Rust, rust_unit, 400usize),
            ("rust×10k lines", Language::Rust, rust_unit, 2_000),
            ("rust×20k lines", Language::Rust, rust_unit, 4_000),
            ("cpp×2k lines", Language::Cpp, cpp_unit, 300),
            ("cpp×10k lines", Language::Cpp, cpp_unit, 1_500),
            ("cpp×20k lines", Language::Cpp, cpp_unit, 3_000),
        ] {
            let text = unit.repeat(n);
            run(label, lang, &text);
        }

        // Long single line (minified-style).
        let long = format!("let x = {};\n", "a".repeat(16_384));
        run("rust long-line 16k", Language::Rust, &long);
        let long2 = format!("let x = {};\n", "a".repeat(65_536));
        run("rust long-line 64k", Language::Rust, &long2);

        // Large byte budgets at ~fixed line length.
        let chunk = "fn a(){ let b = 1; }\n";
        for (label, n) in [("rust ~256KB", 12_000usize), ("rust ~512KB", 24_000), ("rust ~1MB", 48_000)]
        {
            run(label, Language::Rust, &chunk.repeat(n));
        }
    }
}
