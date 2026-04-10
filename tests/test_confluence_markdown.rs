use unified_search_mcp::sources::confluence_markdown::to_markdown;

#[test]
fn converts_headings() {
    assert_eq!(to_markdown("<h1>Title</h1>"), "# Title\n");
    assert_eq!(to_markdown("<h2>Sub</h2>"), "## Sub\n");
    assert_eq!(to_markdown("<h3>Deep</h3>"), "### Deep\n");
}

#[test]
fn converts_paragraphs() {
    assert_eq!(to_markdown("<p>Hello world</p>"), "Hello world\n\n");
    assert_eq!(
        to_markdown("<p>First</p><p>Second</p>"),
        "First\n\nSecond\n\n"
    );
}

#[test]
fn converts_bold_italic() {
    assert_eq!(to_markdown("<strong>bold</strong>"), "**bold**");
    assert_eq!(to_markdown("<b>bold</b>"), "**bold**");
    assert_eq!(to_markdown("<em>italic</em>"), "*italic*");
    assert_eq!(to_markdown("<i>italic</i>"), "*italic*");
}

#[test]
fn converts_links() {
    assert_eq!(
        to_markdown(r#"<a href="https://example.com">click</a>"#),
        "[click](https://example.com)"
    );
}

#[test]
fn converts_unordered_list() {
    let html = "<ul><li>one</li><li>two</li><li>three</li></ul>";
    let expected = "- one\n- two\n- three\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_ordered_list() {
    let html = "<ol><li>first</li><li>second</li></ol>";
    let expected = "1. first\n2. second\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_nested_list() {
    let html = "<ul><li>top<ul><li>nested</li></ul></li></ul>";
    let expected = "- top\n  - nested\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_inline_code() {
    assert_eq!(to_markdown("<code>let x = 1;</code>"), "`let x = 1;`");
}

#[test]
fn converts_code_block() {
    let html = "<pre>fn main() {\n    println!(\"hello\");\n}</pre>";
    let expected = "```\nfn main() {\n    println!(\"hello\");\n}\n```\n\n";
    assert_eq!(to_markdown(html), expected);
}

#[test]
fn converts_confluence_code_macro() {
    let html = r#"<ac:structured-macro ac:name="code"><ac:parameter ac:name="language">rust</ac:parameter><ac:plain-text-body><![CDATA[fn main() {}]]></ac:plain-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(
        result.contains("```rust"),
        "Should have language hint, got: {}",
        result
    );
    assert!(result.contains("fn main() {}"), "Should have code content");
}

#[test]
fn converts_table() {
    let html =
        "<table><tr><th>Name</th><th>Age</th></tr><tr><td>Alice</td><td>30</td></tr></table>";
    let result = to_markdown(html);
    assert!(
        result.contains("| Name | Age |"),
        "Should have header row, got: {}",
        result
    );
    assert!(
        result.contains("|---|---|"),
        "Should have separator, got: {}",
        result
    );
    assert!(
        result.contains("| Alice | 30 |"),
        "Should have data row, got: {}",
        result
    );
}

#[test]
fn converts_image() {
    assert_eq!(
        to_markdown(r#"<img src="https://example.com/img.png" alt="photo">"#),
        "![photo](https://example.com/img.png)"
    );
}

#[test]
fn converts_info_macro() {
    let html = r#"<ac:structured-macro ac:name="info"><ac:rich-text-body><p>Important note</p></ac:rich-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(
        result.contains("> **Info:**"),
        "Should have info blockquote, got: {}",
        result
    );
    assert!(result.contains("Important note"), "Should have content");
}

#[test]
fn converts_warning_macro() {
    let html = r#"<ac:structured-macro ac:name="warning"><ac:rich-text-body><p>Be careful</p></ac:rich-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(
        result.contains("> **Warning:**"),
        "Should have warning blockquote, got: {}",
        result
    );
}

#[test]
fn converts_hr() {
    assert_eq!(to_markdown("<hr>"), "---\n\n");
    assert_eq!(to_markdown("<hr/>"), "---\n\n");
}

#[test]
fn converts_br() {
    assert_eq!(to_markdown("line1<br>line2"), "line1\nline2");
    assert_eq!(to_markdown("line1<br/>line2"), "line1\nline2");
}

#[test]
fn strips_unknown_tags() {
    assert_eq!(to_markdown("<div>content</div>"), "content");
    assert_eq!(to_markdown("<span class=\"x\">text</span>"), "text");
}

#[test]
fn handles_empty_input() {
    assert_eq!(to_markdown(""), "");
}

#[test]
fn converts_strikethrough() {
    assert_eq!(to_markdown("<del>removed</del>"), "~~removed~~");
    assert_eq!(to_markdown("<s>struck</s>"), "~~struck~~");
}

#[test]
fn converts_expand_macro() {
    let html = r#"<ac:structured-macro ac:name="expand"><ac:parameter ac:name="title">Details</ac:parameter><ac:rich-text-body><p>Hidden content</p></ac:rich-text-body></ac:structured-macro>"#;
    let result = to_markdown(html);
    assert!(
        result.contains("<details>"),
        "Should use details tag, got: {}",
        result
    );
    assert!(result.contains("Details"), "Should have summary title");
    assert!(result.contains("Hidden content"), "Should have body");
}

#[test]
fn plain_text_passthrough() {
    assert_eq!(to_markdown("just plain text"), "just plain text");
}
