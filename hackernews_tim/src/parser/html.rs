use crate::prelude::*;
use crate::utils;
use once_cell::sync::Lazy;
use regex::Regex;

/// A regex to parse a HN text (in HTML).
/// It consists of multiple regexes representing different components.
static HN_TEXT_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(&format!(
        "(({})|({})|({})|({})|({})|({}))",
        // a regex matching a HTML paragraph
        r"<p>(?s)(?P<paragraph>(|[^>].*?))</p>",
        // a regex matching a paragraph quote (in markdown format)
        r"<p>(?s)(?P<quote>>[> ]*)(?P<text>.*?)</p>",
        // a regex matching an HTML italic string
        r"<i>(?s)(?P<italic>.*?)</i>",
        // a regex matching a HTML code block (multiline)
        r"<pre><code>(?s)(?P<multiline_code>.*?)[\n]*</code></pre>",
        // a regex matching a single line code block (markdown format)
        r"`(?P<code>[^`]+?)`",
        // a regex matching a HTML link
        r#"<a\s+?href="(?P<link>.*?)"(?s).+?</a>"#,
    ))
    .unwrap()
});

/// Parsed result of a HTML text
#[derive(Debug, Default)]
pub struct HTMLTextParsedResult {
    /// parsed HTML content
    pub content: StyledString,
    /// a list of links inside the HTML document
    pub links: Vec<String>,
}

/// Parsed result of a HTML table
#[derive(Debug, Default)]
pub struct HTMLTableParsedResult {
    /// a list of links inside the HTML document
    pub links: Vec<String>,
    /// parsed table headers
    pub headers: Vec<StyledString>,
    /// parsed table rows
    pub rows: Vec<Vec<StyledString>>,
}

impl HTMLTextParsedResult {
    /// merge two HTML parsed results
    pub fn merge(&mut self, mut other: HTMLTextParsedResult) {
        self.content.append(other.content);
        self.links.append(&mut other.links);
    }
}

/// parse a Hacker News HTML text
pub fn parse_hn_html_text(text: String, style: Style, base_link_id: usize) -> HTMLTextParsedResult {
    debug!("parse hn html text: {}", text);

    // pre-processed the HTML text
    let text = {
        // The item's text returned from HN APIs may have `<p>` tags representing
        // paragraph breaks. Convert `<p>` tags to <p></p> tag pairs to make the text
        // easier to parse.
        if text.is_empty() {
            text
        } else {
            format!("<p>{}</p>", text.replace("<p>", "</p>\n<p>"))
        }
    };

    parse(text, style, base_link_id)
}

/// a helper function of [parse_hn_html_text] for recursively parsing HTML elements inside the text
fn parse(text: String, style: Style, base_link_id: usize) -> HTMLTextParsedResult {
    let mut result = HTMLTextParsedResult::default();
    // an index such that `text[curr_pos..]` represents the slice of the
    // text that hasn't been parsed.
    let mut curr_pos = 0;

    for caps in HN_TEXT_RE.captures_iter(&text) {
        // the part that doesn't match any patterns is rendered in the default style
        let whole_match = caps.get(0).unwrap();
        if curr_pos < whole_match.start() {
            result
                .content
                .append_styled(&text[curr_pos..whole_match.start()], style);
        }
        curr_pos = whole_match.end();

        let component_style = &config::get_config_theme().component_style;

        if let (Some(m_quote), Some(m_text)) = (caps.name("quote"), caps.name("text")) {
            // quoted paragraph
            // render quote character `>` using the `|` indentation character
            result.content.append_styled(
                "▎"
                    .to_string()
                    .repeat(m_quote.as_str().matches('>').count()),
                style,
            );
            result.merge(parse(
                m_text.as_str().to_string(),
                component_style.quote.into(),
                base_link_id + result.links.len(),
            ));

            result.content.append_plain("\n");
        } else if let Some(m) = caps.name("paragraph") {
            // normal paragraph
            result.merge(parse(
                m.as_str().to_string(),
                style,
                base_link_id + result.links.len(),
            ));

            result.content.append_plain("\n");
        } else if let Some(m) = caps.name("link") {
            // HTML link
            result.links.push(m.as_str().to_string());

            result.content.append_styled(
                utils::shorten_url(m.as_str()),
                style.combine(component_style.link),
            );
            result.content.append_styled(" ", style);
            result.content.append_styled(
                format!("[{}]", result.links.len() + base_link_id),
                style.combine(component_style.link_id),
            );
        } else if let Some(m) = caps.name("multiline_code") {
            // HTML code block
            result.content.append_styled(
                m.as_str(),
                style.combine(component_style.multiline_code_block),
            );
            result.content.append_plain("\n");
        } else if let Some(m) = caps.name("code") {
            // markdown single line code block
            result
                .content
                .append_styled(m.as_str(), style.combine(component_style.single_code_block));
        } else if let Some(m) = caps.name("italic") {
            // HTML italic
            result
                .content
                .append_styled(m.as_str(), style.combine(component_style.italic));
        }
    }

    if curr_pos < text.len() {
        result
            .content
            .append_styled(&text[curr_pos..text.len()], style);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    fn render(text: &str, base_link_id: usize) -> HTMLTextParsedResult {
        // The parser reads the global theme; install defaults so the call
        // doesn't panic when this test module runs in isolation.
        config::init_test_config();
        parse_hn_html_text(text.to_string(), Style::default(), base_link_id)
    }

    fn rendered_source(text: &str) -> String {
        render(text, 0).content.source().to_string()
    }

    #[test]
    fn plain_text_paragraph_has_no_links() {
        let result = render("hello world", 0);
        // Trailing newline is added by the paragraph branch wrapping the
        // input in <p>...</p>.
        assert_eq!(result.content.source(), "hello world\n");
        assert!(result.links.is_empty());
    }

    #[test]
    fn single_link_collected_with_marker() {
        let result = render(r#"<p>see <a href="https://example.com">site</a></p>"#, 0);
        assert_eq!(result.links, vec!["https://example.com".to_string()]);
        let src = result.content.source();
        // First link is numbered [1] when base_link_id == 0.
        assert!(src.contains("[1]"), "expected [1] marker; got {src:?}");
        assert!(src.contains("example.com"), "got {src:?}");
    }

    #[test]
    fn base_link_id_offsets_marker() {
        let result = render(r#"<p><a href="https://a.example">a</a></p>"#, 4);
        // base_link_id = 4 → first link marker is [5].
        let src = result.content.source();
        assert!(src.contains("[5]"), "expected [5] marker; got {src:?}");
        assert_eq!(result.links, vec!["https://a.example".to_string()]);
    }

    #[test]
    fn multiple_links_numbered_sequentially() {
        let result = render(
            r#"<p><a href="https://one.example">one</a> and <a href="https://two.example">two</a> and <a href="https://three.example">three</a></p>"#,
            0,
        );
        assert_eq!(
            result.links,
            vec![
                "https://one.example".to_string(),
                "https://two.example".to_string(),
                "https://three.example".to_string(),
            ]
        );
        let src = result.content.source();
        assert!(src.contains("[1]"));
        assert!(src.contains("[2]"));
        assert!(src.contains("[3]"));
    }

    #[test]
    fn pre_code_block_preserved_verbatim_with_whitespace() {
        // The code-block branch passes the inner text through as-is, including
        // multi-line indentation. Trailing newlines are stripped by the regex.
        let src = rendered_source("<pre><code>fn main() {\n    println!(\"hi\");\n}</code></pre>");
        assert!(
            src.contains("fn main() {\n    println!(\"hi\");\n}"),
            "got {src:?}"
        );
    }

    #[test]
    fn italic_tag_content_is_preserved() {
        let src = rendered_source("<i>emphasis</i>");
        assert!(src.contains("emphasis"), "got {src:?}");
    }

    #[test]
    fn inline_code_backticks_are_consumed() {
        // Markdown-style inline code uses backticks; the backticks themselves
        // are not part of the rendered output, only the inner text is.
        let src = rendered_source("see `foo` here");
        assert!(src.contains("foo"), "got {src:?}");
        assert!(!src.contains("`foo`"), "backticks should be stripped: {src:?}");
    }

    #[test]
    fn link_inside_italic_is_not_recursed_into() {
        // Italic content is appended verbatim — the parser doesn't recurse
        // through `<i>...</i>`, so a link nested inside an italic tag is
        // *not* lifted into the links list. This pins the current behaviour;
        // a future change to recurse through styled tags would update this
        // expectation.
        let result = render(
            r#"<p><i>see <a href="https://nested.example">it</a></i></p>"#,
            0,
        );
        assert!(result.links.is_empty(), "got links {:?}", result.links);
    }

    #[test]
    fn link_inside_paragraph_is_recursed_into() {
        // Paragraphs *are* recursed into, so a link inside a `<p>` is
        // extracted. Pair with the italic test above to triangulate the
        // recurse-vs-passthrough boundary.
        let result = render(
            r#"<p>see <a href="https://outer.example">it</a></p>"#,
            0,
        );
        assert_eq!(result.links, vec!["https://outer.example".to_string()]);
    }

    #[test]
    fn empty_input_returns_empty_styled_string_and_no_links() {
        let result = render("", 0);
        assert_eq!(result.content.source(), "");
        assert!(result.links.is_empty());
    }

    #[test]
    fn whitespace_only_input_does_not_emit_links() {
        let result = render("   ", 0);
        assert!(result.links.is_empty());
    }

    #[test]
    fn merge_appends_content_and_links() {
        let mut a = render(r#"<p><a href="https://a.example">a</a></p>"#, 0);
        let b = render(r#"<p><a href="https://b.example">b</a></p>"#, 0);
        a.merge(b);
        assert_eq!(
            a.links,
            vec![
                "https://a.example".to_string(),
                "https://b.example".to_string(),
            ]
        );
    }
}
