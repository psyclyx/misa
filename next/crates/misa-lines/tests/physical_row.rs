use misa_lines::Line;
use misa_style::Style;
use misa_terminal_ui::output::Output;

#[test]
fn semantic_node_identity_does_not_repaint_the_same_physical_row() {
    let mut output = Output::default();
    let mut bytes = Vec::new();
    let mut row = Line {
        spans: vec![(Style::PLAIN, "hello".into())],
        node: Some("old".into()),
        ..Line::default()
    };
    output.paint(&mut bytes, &[row.clone()], 20, &[]).unwrap();
    bytes.clear();
    row.node = Some("new".into());
    output.paint(&mut bytes, &[row], 20, &[]).unwrap();
    assert!(bytes.is_empty());
}
