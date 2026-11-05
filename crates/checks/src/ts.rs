//! Code facts from a TypeScript/JavaScript file, using the tree-sitter
//! parser (TSX grammar, which also reads plain JS/JSX).
//!
//! The mutation generator finds code with simple text helpers; the checker
//! uses a real parser. Two independent methods make it less likely that a
//! bug in one hides behind the same bug in the other.

use std::collections::BTreeSet;
use std::sync::OnceLock;

use regex::Regex;
use tree_sitter::{Node, Parser};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestCase {
    /// Full name including enclosing `describe` blocks: `a > b > it name`.
    pub name: String,
    /// `.skip`, `x…`, `.todo`, or inside a skipped `describe`.
    pub skipped: bool,
    /// `.only` / `f…` (which silently skips every other test).
    pub focused: bool,
    /// Number of `expect(…)` calls inside the test.
    pub expects: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Facts {
    pub tests: Vec<TestCase>,
    /// `expect(<literal>)`: compares a constant, so it always passes or
    /// always fails regardless of the code.
    pub vacuous_expects: usize,
    /// Names of functions whose body is a stub.
    pub stubs: BTreeSet<String>,
    /// `catch` blocks (and `.catch(() => {})` handlers) that do nothing.
    pub swallowed_errors: usize,
    /// Names declared at the top level (functions, classes, variables,
    /// types), in order.
    pub top_level: Vec<String>,
    /// Byte range of the top-level statement declaring each name in
    /// `top_level` (same order; a statement declaring two names appears
    /// twice).
    pub top_level_spans: Vec<(usize, usize)>,
    /// The parser found syntax errors (facts may be incomplete).
    pub has_errors: bool,
}

impl Facts {
    pub fn skipped(&self) -> usize {
        self.tests.iter().filter(|t| t.skipped).count()
    }

    pub fn focused(&self) -> usize {
        self.tests.iter().filter(|t| t.focused).count()
    }
}

/// Parse `src` and extract its facts. Never panics; unparseable input gives
/// empty facts with `has_errors` set.
pub fn facts(src: &str) -> Facts {
    let mut parser = Parser::new();
    if parser.set_language(&tree_sitter_typescript::LANGUAGE_TSX.into()).is_err() {
        return Facts { has_errors: true, ..Facts::default() };
    }
    let Some(tree) = parser.parse(src, None) else {
        return Facts { has_errors: true, ..Facts::default() };
    };
    let root = tree.root_node();
    let mut f = Facts { has_errors: root.has_error(), ..Facts::default() };
    let mut ctx = Walk { src, facts: &mut f };
    ctx.walk(root, &mut Vec::new(), false, false);
    for (name, span) in top_level_names(root, src) {
        f.top_level.push(name);
        f.top_level_spans.push(span);
    }
    f
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

fn named_children(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CallKind {
    Test,
    Suite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Modifier {
    None,
    Skip,
    Only,
}

/// Is this call a test (`it`, `test`) or suite (`describe`), and with which
/// modifier?
fn classify(call: Node, src: &str) -> Option<(CallKind, Modifier)> {
    let func = call.child_by_field_name("function")?;
    match func.kind() {
        "identifier" => match text(func, src) {
            "it" | "test" => Some((CallKind::Test, Modifier::None)),
            "describe" | "suite" => Some((CallKind::Suite, Modifier::None)),
            "xit" | "xtest" => Some((CallKind::Test, Modifier::Skip)),
            "xdescribe" => Some((CallKind::Suite, Modifier::Skip)),
            "fit" => Some((CallKind::Test, Modifier::Only)),
            "fdescribe" => Some((CallKind::Suite, Modifier::Only)),
            _ => None,
        },
        "member_expression" => {
            let object = func.child_by_field_name("object")?;
            let property = text(func.child_by_field_name("property")?, src);
            let kind = match text(object, src) {
                "it" | "test" => CallKind::Test,
                "describe" | "suite" => CallKind::Suite,
                _ => return None,
            };
            let modifier = match property {
                "skip" | "todo" => Modifier::Skip,
                "only" => Modifier::Only,
                "concurrent" | "sequential" => Modifier::None,
                _ => return None,
            };
            Some((kind, modifier))
        }
        // it.each(table)("name", fn)
        "call_expression" => {
            let inner = func.child_by_field_name("function")?;
            if inner.kind() != "member_expression" {
                return None;
            }
            let object = text(inner.child_by_field_name("object")?, src);
            let property = text(inner.child_by_field_name("property")?, src);
            match (object, property) {
                ("it" | "test", "each") => Some((CallKind::Test, Modifier::None)),
                ("describe", "each") => Some((CallKind::Suite, Modifier::None)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The name argument of a test/suite call.
fn call_name(call: Node, src: &str) -> String {
    let Some(args) = call.child_by_field_name("arguments") else {
        return String::new();
    };
    let Some(first) = named_children(args).into_iter().next() else {
        return String::new();
    };
    let t = text(first, src);
    match first.kind() {
        "string" | "template_string" => t.trim_matches(['"', '\'', '`']).to_string(),
        _ => t.to_string(),
    }
}

fn is_expect_call(node: Node, src: &str) -> bool {
    if node.kind() != "call_expression" {
        return false;
    }
    let Some(func) = node.child_by_field_name("function") else {
        return false;
    };
    match func.kind() {
        "identifier" => text(func, src) == "expect",
        // expect.soft(…)
        "member_expression" => func
            .child_by_field_name("object")
            .is_some_and(|o| o.kind() == "identifier" && text(o, src) == "expect"),
        _ => false,
    }
}

fn is_literal(node: Node, src: &str) -> bool {
    match node.kind() {
        "true" | "false" | "null" | "number" | "string" => true,
        "undefined" => true,
        "identifier" => text(node, src) == "undefined",
        "template_string" => !named_children(node).iter().any(|c| c.kind() == "template_substitution"),
        "unary_expression" => named_children(node).first().is_some_and(|c| c.kind() == "number"),
        _ => false,
    }
}

fn is_vacuous_expect(node: Node, src: &str) -> bool {
    is_expect_call(node, src)
        && node
            .child_by_field_name("arguments")
            .and_then(|a| named_children(a).into_iter().next())
            .is_some_and(|first| is_literal(first, src))
}

fn count_expects(node: Node, src: &str) -> usize {
    let mut n = usize::from(is_expect_call(node, src));
    for c in named_children(node) {
        n += count_expects(c, src);
    }
    n
}

fn todo_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(todo|fixme|xxx|placeholder|implement)").expect("regex"))
}

fn not_implemented_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)not\s+(yet\s+)?implemented|unimplemented|\btodo\b|implement\s+me").expect("regex")
    })
}

/// Literal-ish return values used as placeholders.
fn is_placeholder_value(node: Node, src: &str) -> bool {
    is_literal(node, src)
        || (matches!(node.kind(), "array" | "object") && named_children(node).is_empty())
}

/// A function body that is a stub: only a "not implemented" throw, only a
/// TODO comment, or a TODO comment plus a placeholder return.
fn is_stub_body(block: Node, src: &str) -> bool {
    if block.kind() != "statement_block" {
        return false;
    }
    let children = named_children(block);
    let comments: Vec<&str> = children.iter().filter(|c| c.kind() == "comment").map(|c| text(*c, src)).collect();
    let stmts: Vec<Node> = children.into_iter().filter(|c| c.kind() != "comment").collect();
    let todo_comment = comments.iter().any(|c| todo_re().is_match(c));
    match stmts.as_slice() {
        [] => todo_comment,
        [only] if only.kind() == "throw_statement" => not_implemented_re().is_match(text(*only, src)),
        [.., last] if stmts.len() <= 2 && todo_comment && last.kind() == "return_statement" => {
            match named_children(*last).first() {
                None => true,
                Some(value) => is_placeholder_value(*value, src),
            }
        }
        _ => false,
    }
}

/// A block that does nothing with the error: no statements, and no comment
/// explaining why (a bare `// ignore` doesn't count as a reason). A short
/// explanation, e.g. "storage may be unavailable in private mode", marks
/// a deliberate choice and isn't reported.
fn swallows(block: Node, src: &str) -> bool {
    if block.kind() != "statement_block" {
        return false;
    }
    let children = named_children(block);
    if children.iter().any(|c| c.kind() != "comment") {
        return false;
    }
    let words: usize = children
        .iter()
        .map(|c| {
            text(*c, src)
                .trim_start_matches("//")
                .trim_start_matches("/*")
                .trim_end_matches("*/")
                .split_whitespace()
                .count()
        })
        .sum();
    words < 4
}

fn is_function(node: Node) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "generator_function_declaration"
            | "function_expression"
            | "function"
            | "arrow_function"
            | "method_definition"
    )
}

/// Name for a function node: its own name, or the variable / property it
/// is assigned to.
fn function_name(node: Node, src: &str) -> Option<String> {
    if let Some(n) = node.child_by_field_name("name") {
        return Some(text(n, src).to_string());
    }
    let parent = node.parent()?;
    let n = match parent.kind() {
        "variable_declarator" => parent.child_by_field_name("name")?,
        "pair" => parent.child_by_field_name("key")?,
        "public_field_definition" => parent.child_by_field_name("name")?,
        _ => return None,
    };
    Some(text(n, src).trim_matches(['"', '\'']).to_string())
}

struct Walk<'a> {
    src: &'a str,
    facts: &'a mut Facts,
}

impl Walk<'_> {
    fn walk(&mut self, node: Node, suites: &mut Vec<String>, skipped: bool, focused: bool) {
        let src = self.src;
        if node.kind() == "call_expression" {
            if is_vacuous_expect(node, src) {
                self.facts.vacuous_expects += 1;
            }
            if let Some((kind, modifier)) = classify(node, src) {
                let name = call_name(node, src);
                let skip = skipped || modifier == Modifier::Skip;
                let only = focused || modifier == Modifier::Only;
                match kind {
                    CallKind::Test => {
                        let mut full = suites.clone();
                        full.push(name);
                        self.facts.tests.push(TestCase {
                            name: full.join(" > "),
                            skipped: skip,
                            focused: only,
                            expects: count_expects(node, src),
                        });
                        // Still walk inside for stubs, catches, vacuous expects.
                        for c in named_children(node) {
                            self.walk(c, suites, skip, only);
                        }
                        return;
                    }
                    CallKind::Suite => {
                        suites.push(name);
                        for c in named_children(node) {
                            self.walk(c, suites, skip, only);
                        }
                        suites.pop();
                        return;
                    }
                }
            }
            // promise.catch(() => {})
            if let Some(func) = node.child_by_field_name("function")
                && func.kind() == "member_expression"
                && func.child_by_field_name("property").is_some_and(|p| text(p, src) == "catch")
                && let Some(handler) = node
                    .child_by_field_name("arguments")
                    .and_then(|a| named_children(a).into_iter().next())
                && is_function(handler)
                && handler.child_by_field_name("body").is_some_and(|b| swallows(b, src))
            {
                self.facts.swallowed_errors += 1;
            }
        }
        if node.kind() == "catch_clause" && node.child_by_field_name("body").is_some_and(|b| swallows(b, src)) {
            self.facts.swallowed_errors += 1;
        }
        if is_function(node)
            && let Some(body) = node.child_by_field_name("body")
            && is_stub_body(body, src)
            && let Some(name) = function_name(node, src)
        {
            self.facts.stubs.insert(name);
        }
        for c in named_children(node) {
            self.walk(c, suites, skipped, focused);
        }
    }
}

fn declaration_names(node: Node, src: &str, out: &mut Vec<String>) {
    match node.kind() {
        "export_statement" => {
            if let Some(d) = node.child_by_field_name("declaration") {
                declaration_names(d, src, out);
            }
        }
        "function_declaration" | "generator_function_declaration" | "class_declaration"
        | "abstract_class_declaration" | "interface_declaration" | "type_alias_declaration"
        | "enum_declaration" => {
            if let Some(n) = node.child_by_field_name("name") {
                out.push(text(n, src).to_string());
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            for d in named_children(node).into_iter().filter(|d| d.kind() == "variable_declarator") {
                if let Some(n) = d.child_by_field_name("name").filter(|n| n.kind() == "identifier") {
                    out.push(text(n, src).to_string());
                }
            }
        }
        _ => {}
    }
}

fn top_level_names(root: Node, src: &str) -> Vec<(String, (usize, usize))> {
    let mut out = Vec::new();
    for c in named_children(root) {
        let mut names = Vec::new();
        declaration_names(c, src, &mut names);
        out.extend(names.into_iter().map(|n| (n, (c.start_byte(), c.end_byte()))));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TESTS: &str = r#"import { describe, it, test, expect } from "vitest";
import { add } from "./add";

describe("add", () => {
  it("adds", () => {
    expect(add(1, 2)).toBe(3);
    expect(add(2, 2)).toBe(4);
  });

  it.skip("skipped one", () => {
    expect(add(0, 0)).toBe(0);
  });

  describe.skip("old", () => {
    test("inside skipped suite", () => {});
  });

  it.todo("later");

  it("vacuous", () => {
    expect(true).toBe(true);
    expect(`fixed`).toBe("fixed");
    expect(-1).toBeLessThan(0);
    expect(`${add(1, 1)}`).toBe("2");
  });

  it.each([[1, 2]])("each %i", (a, b) => {
    expect(add(a, b)).toBe(3);
  });
});

xit("old style", () => {});
test.only("focused", () => {});
"#;

    #[test]
    fn finds_tests_with_names_skips_and_focus() {
        let f = facts(TESTS);
        assert!(!f.has_errors);
        let names: Vec<(&str, bool, bool, usize)> =
            f.tests.iter().map(|t| (t.name.as_str(), t.skipped, t.focused, t.expects)).collect();
        assert_eq!(
            names,
            [
                ("add > adds", false, false, 2),
                ("add > skipped one", true, false, 1),
                ("add > old > inside skipped suite", true, false, 0),
                ("add > later", true, false, 0),
                ("add > vacuous", false, false, 4),
                ("add > each %i", false, false, 1),
                ("old style", true, false, 0),
                ("focused", false, true, 0),
            ]
        );
        assert_eq!(f.skipped(), 4);
        assert_eq!(f.focused(), 1);
        // true, `fixed`, -1 are constants; `${…}` uses code.
        assert_eq!(f.vacuous_expects, 3);
    }

    #[test]
    fn finds_stubs() {
        let src = r#"
export function a(): number {
  throw new Error("not implemented");
}
function b() {
  // TODO: implement
}
const c = (x: number) => {
  // TODO wire up the real API
  return [];
};
const obj = {
  d() { throw new Error("Not yet implemented"); },
  e: () => { /* placeholder */ },
};
class K {
  f(): void { throw new Error("TODO"); }
}
function realThrow(x: number) {
  throw new Error("x must be positive");
}
function noop() {}
function real(x: number) {
  // TODO: tidy this later
  const y = x * 2;
  return y + 1;
}
"#;
        let f = facts(src);
        let stubs: Vec<&str> = f.stubs.iter().map(String::as_str).collect();
        assert_eq!(stubs, ["a", "b", "c", "d", "e", "f"]);
    }

    #[test]
    fn finds_swallowed_errors() {
        let src = r#"
try { risky(); } catch (e) {}
try { risky(); } catch { /* ignore */ }
try { risky(); } catch (e) { console.error(e); }
fetchData().catch(() => {});
fetchData().catch((e) => { report(e); });
fetchData().catch(function () { /* nothing */ });
try { save(); } catch {
  // Storage may be unavailable (e.g. private mode); the app still works.
}
"#;
        // Two empty catch blocks and two empty .catch() handlers. The last
        // catch explains why it's empty, so it's a deliberate choice
        // (seen in a real recording).
        assert_eq!(facts(src).swallowed_errors, 4);
    }

    #[test]
    fn top_level_declarations() {
        let src = r#"
import x from "y";
export function a() {}
function b() {}
export const c = 1, d = 2;
let { e } = obj;
class F {}
export interface G {}
type H = string;
export default function I() {}
if (x) { const notTopLevel = 1; }
"#;
        assert_eq!(facts(src).top_level, ["a", "b", "c", "d", "F", "G", "H", "I"]);
    }

    #[test]
    fn handles_jsx_and_broken_code() {
        let jsx = "export function App() {\n  return <div onClick={() => {}}>hi</div>;\n}\n";
        let f = facts(jsx);
        assert!(!f.has_errors);
        assert!(f.stubs.is_empty(), "empty JSX handler isn't a named stub");
        assert!(facts("function (((").has_errors);
    }
}
