//! A document may nest as deeply as it likes without spending a client's stack.
//!
//! Both frontends parse markdown on their own thread — a stream's every delta on
//! the terminal side, a document's every edit on the pixel side — so a parse that
//! recurses per nesting level is a stack overflow waiting for a pathological
//! message. Past [`misa_markdown::MAX_NESTING`] the nesting stops and the content
//! is kept as text: nothing is dropped, and nothing recurses further.

use misa_markdown::MAX_NESTING;

#[test]
fn the_nesting_bound_is_half_the_views() {
    assert!(MAX_NESTING * 2 <= misa_proto::view::MAX_DEPTH);
}

fn deepest(roots: &[misa_proto::view::Node]) -> usize {
    let mut max = 0;
    let mut pending: Vec<(usize, &[misa_proto::view::Node])> = vec![(1, roots)];
    while let Some((depth, nodes)) = pending.pop() {
        max = max.max(depth);
        for node in nodes {
            pending.push((depth + 1, &node.children));
        }
    }
    max
}

fn text(roots: &[misa_proto::view::Node]) -> String {
    let mut out = String::new();
    let mut pending: Vec<&[misa_proto::view::Node]> = vec![roots];
    while let Some(nodes) = pending.pop() {
        for node in nodes {
            out.push_str(&format!("{node:?}"));
            pending.push(&node.children);
        }
    }
    out
}

#[test]
fn deeply_nested_markdown_stays_flat_instead_of_overflowing_the_stack() {
    let cases = [
        ("quotes", "> ".repeat(50_000) + "deep"),
        ("lists", "  ".repeat(20_000) + &"- deep"),
        ("emphasis", "**".repeat(20_000) + "deep"),
    ];
    for (name, body) in cases {
        let parsed = std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn({
                let body = body.clone();
                move || misa_markdown::blocks("markdown", &body)
            })
            .unwrap()
            .join()
            .unwrap_or_else(|_| panic!("{name} overflowed the parse"));
        assert!(!parsed.is_empty(), "{name} kept no text");
        for node in &parsed {
            misa_proto::view::validate(node)
                .unwrap_or_else(|fault| panic!("{name} produced an invalid view node: {fault}"));
        }
        assert!(
            deepest(&parsed) <= misa_proto::view::MAX_DEPTH,
            "{name} nested past the view's depth bound: depth {}",
            deepest(&parsed)
        );
        assert!(text(&parsed).contains("deep"), "{name} dropped its content");
    }
}
