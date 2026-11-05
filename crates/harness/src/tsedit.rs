//! Small, careful text helpers for finding tests, assertions and functions
//! in TypeScript/JavaScript files. Used only to *inject* known defects, so
//! they favour "find nothing" over guessing: a mutation that can't find a
//! clean target is simply not applied.

use regex::Regex;

pub use checks::paths::{is_source_file, is_test_file};

/// Index of the bracket matching the one at `open`, skipping strings,
/// template literals and comments. `None` if unbalanced.
pub fn matching_bracket(src: &str, open: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let (open_ch, close_ch) = match bytes.get(open)? {
        b'(' => (b'(', b')'),
        b'{' => (b'{', b'}'),
        b'[' => (b'[', b']'),
        _ => return None,
    };
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b'\'' | b'"' => i = skip_string(bytes, i)?,
            b'`' => i = skip_template(src, i)?,
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = bytes[i..].iter().position(|b| *b == b'\n').map_or(bytes.len(), |p| i + p);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = src[i + 2..].find("*/").map(|p| i + 2 + p + 1)?;
            }
            _ if c == open_ch => depth += 1,
            _ if c == close_ch => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// From an opening quote, return the index of the closing quote.
fn skip_string(bytes: &[u8], start: usize) -> Option<usize> {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'\n' => return None,
            c if c == quote => return Some(i),
            _ => i += 1,
        }
    }
    None
}

/// From an opening backtick, return the index of the closing backtick.
fn skip_template(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'`' => return Some(i),
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                i = matching_bracket(src, i + 1)? + 1;
            }
            _ => i += 1,
        }
    }
    None
}

fn line_start(src: &str, at: usize) -> usize {
    src[..at].rfind('\n').map_or(0, |p| p + 1)
}

/// Extend `end` (exclusive) over an optional `;` and the rest of the line.
fn extend_to_line_end(src: &str, mut end: usize) -> usize {
    let bytes = src.as_bytes();
    if bytes.get(end) == Some(&b';') {
        end += 1;
    }
    while matches!(bytes.get(end), Some(b' ' | b'\t' | b'\r')) {
        end += 1;
    }
    if bytes.get(end) == Some(&b'\n') {
        end += 1;
    }
    end
}

/// A test case: `it(...)` or `test(...)` (not `describe`, not already
/// skipped or focused).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    pub name: String,
    /// Start of the line holding `it(` / `test(`.
    pub start: usize,
    /// End of the call, including `;` and the newline (exclusive).
    pub end: usize,
    /// Index just after the `it` / `test` keyword (where `.skip` goes).
    pub keyword_end: usize,
}

pub fn find_tests(src: &str) -> Vec<TestCase> {
    let re = Regex::new(r"(?m)^[ \t]*(it|test)[ \t]*\(").expect("regex");
    let mut out = Vec::new();
    for m in re.find_iter(src) {
        let open = m.end() - 1;
        let Some(close) = matching_bracket(src, open) else {
            continue;
        };
        let keyword_end = src[m.start()..open].trim_end().len() + m.start();
        let name = first_string_literal(&src[open + 1..close]).unwrap_or_default();
        out.push(TestCase {
            name,
            start: line_start(src, m.start()),
            end: extend_to_line_end(src, close + 1),
            keyword_end,
        });
    }
    out
}

fn first_string_literal(s: &str) -> Option<String> {
    let re = Regex::new(r#"^\s*(?:'([^'\n]*)'|"([^"\n]*)"|`([^`\n]*)`)"#).expect("regex");
    let caps = re.captures(s)?;
    caps.get(1)
        .or(caps.get(2))
        .or(caps.get(3))
        .map(|m| m.as_str().to_string())
}

/// A full `expect(...)...` chain, e.g. `expect(a).toBe(3)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assertion {
    pub start: usize,
    /// Exclusive end of the chain (before any `;`).
    pub end: usize,
}

pub fn find_assertions(src: &str) -> Vec<Assertion> {
    let re = Regex::new(r"\bexpect[ \t]*\(").expect("regex");
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    for m in re.find_iter(src) {
        let Some(mut end) = matching_bracket(src, m.end() - 1).map(|c| c + 1) else {
            continue;
        };
        // Follow `.name` and `.name(...)` links of the chain.
        loop {
            let mut j = end;
            while matches!(bytes.get(j), Some(b' ' | b'\t' | b'\n' | b'\r')) {
                j += 1;
            }
            if bytes.get(j) != Some(&b'.') {
                break;
            }
            j += 1;
            let ident_len = src[j..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '$'))
                .unwrap_or(src.len() - j);
            if ident_len == 0 {
                break;
            }
            j += ident_len;
            end = j;
            if bytes.get(j) == Some(&b'(') {
                match matching_bracket(src, j) {
                    Some(c) => end = c + 1,
                    None => break,
                }
            }
        }
        out.push(Assertion { start: m.start(), end });
    }
    out
}

/// A `function name(...) { ... }` declaration (optionally exported/async).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionDecl {
    pub name: String,
    pub body_open: usize,
    pub body_close: usize,
}

pub fn find_functions(src: &str) -> Vec<FunctionDecl> {
    let re = Regex::new(
        r"(?m)^[ \t]*(?:export[ \t]+)?(?:default[ \t]+)?(?:async[ \t]+)?function[ \t]*\*?[ \t]*([A-Za-z_$][\w$]*)[ \t]*(?:<[^>\n]*>)?[ \t]*\(",
    )
    .expect("regex");
    let mut out = Vec::new();
    for caps in re.captures_iter(src) {
        let whole = caps.get(0).expect("match");
        let Some(params_close) = matching_bracket(src, whole.end() - 1) else {
            continue;
        };
        // The body starts at the first `{` after the parameters (skipping a
        // simple return type annotation).
        let Some(rel) = src[params_close + 1..].find('{') else {
            continue;
        };
        let body_open = params_close + 1 + rel;
        // Anything but a simple return type between `)` and `{` (e.g. an
        // overload ending in `;`) means this isn't a function body.
        if src[params_close + 1..body_open].contains(['\n', ';', '=']) {
            continue;
        }
        let Some(body_close) = matching_bracket(src, body_open) else {
            continue;
        };
        out.push(FunctionDecl {
            name: caps[1].to_string(),
            body_open,
            body_close,
        });
    }
    out
}

/// Indentation of the line containing `at`.
pub fn indent_at(src: &str, at: usize) -> String {
    let start = line_start(src, at);
    src[start..].chars().take_while(|c| *c == ' ' || *c == '\t').collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SRC: &str = r#"import { describe, it, expect } from 'vitest';
import { add } from './add';

describe('add', () => {
  it('adds two numbers', () => {
    expect(add(1, 2)).toBe(3);
  });

  it("handles ) in strings", () => {
    const s = `tricky ${"}"} text`; // a ) comment
    expect(add(-1, 1))
      .toEqual(0);
  });

  test('third', async () => {
    await expect(Promise.resolve(add(2, 2))).resolves.toBe(4);
  });
});
"#;

    #[test]
    fn classifies_files() {
        assert!(is_test_file("src/a.test.ts"));
        assert!(is_test_file("src/__tests__/a.ts"));
        assert!(is_test_file("b.spec.jsx"));
        assert!(!is_test_file("src/a.ts"));
        assert!(is_source_file("src/a.ts"));
        assert!(is_source_file("src/App.tsx"));
        assert!(!is_source_file("src/a.test.ts"));
        assert!(!is_source_file("src/types.d.ts"));
        assert!(!is_source_file("package.json"));
    }

    #[test]
    fn brackets_skip_strings_templates_and_comments() {
        let s = r#"f(")", `${")"}`, /* ) */ 1) // )"#;
        assert_eq!(matching_bracket(s, 1), Some(s.find(" 1)").unwrap() + 2));
        assert_eq!(matching_bracket("(()", 0), None);
    }

    #[test]
    fn finds_test_cases_with_names_and_extents() {
        let tests = find_tests(TEST_SRC);
        let names: Vec<&str> = tests.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["adds two numbers", "handles ) in strings", "third"]);
        let first = &TEST_SRC[tests[0].start..tests[0].end];
        assert!(first.starts_with("  it('adds two numbers'"));
        assert!(first.ends_with("});\n"));
        assert_eq!(&TEST_SRC[tests[2].keyword_end - 4..tests[2].keyword_end], "test");
        // Removing a case leaves balanced brackets.
        let removed = format!("{}{}", &TEST_SRC[..tests[1].start], &TEST_SRC[tests[1].end..]);
        assert_eq!(find_tests(&removed).len(), 2);
        assert!(matching_bracket(&removed, removed.find("describe(").unwrap() + 8).is_some());
    }

    #[test]
    fn finds_whole_assertion_chains() {
        let a = find_assertions(TEST_SRC);
        let chains: Vec<&str> = a.iter().map(|x| &TEST_SRC[x.start..x.end]).collect();
        assert_eq!(
            chains,
            [
                "expect(add(1, 2)).toBe(3)",
                "expect(add(-1, 1))\n      .toEqual(0)",
                "expect(Promise.resolve(add(2, 2))).resolves.toBe(4)",
            ]
        );
    }

    #[test]
    fn finds_function_bodies() {
        let src = "export function settle(a: number[]): number {\n  return a.reduce((x, y) => x + y, 0);\n}\n\nasync function load<T>(p: string): Promise<T> {\n  return JSON.parse(p);\n}\nconst arrow = () => 1;\n";
        let fns = find_functions(src);
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["settle", "load"]);
        // An overload signature has no body.
        let overload = "function f(a: string): void;\nconst x = { a: 1 };\n";
        assert!(find_functions(overload).is_empty());
        assert_eq!(&src[fns[0].body_open..=fns[0].body_close], "{\n  return a.reduce((x, y) => x + y, 0);\n}");
    }

    #[test]
    fn indentation() {
        assert_eq!(indent_at("a\n    b", 7), "    ");
    }
}
