// SPDX-FileCopyrightText: 2026 Axle Duggan (axlecoffee) <contact@axle.coffee>
// SPDX-License-Identifier: AGPL-3.0-only
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd, html};
use std::sync::LazyLock;
use two_face::re_exports::syntect::{html::{ClassStyle, ClassedHTMLGenerator}, parsing::SyntaxSet, util::LinesWithEndings};

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

pub fn code(text: &str, language: &str) -> String {
    // Bound synchronous parsing on the UI thread; large or long-line pastes stay plain.
    if text.len() > 32_000 || text.lines().any(|line| line.len() > 1_000) || matches!(language, "txt" | "text" | "plain") {
        return ammonia::clean_text(text);
    }
    let language = match language { "rust" => "rs", "typescript" => "ts", "kotlin" => "kt", "javascript" => "js", "python" => "py", "bash" => "sh", other => other };
    let syntax = SYNTAXES.find_syntax_by_extension(language).or_else(|| SYNTAXES.find_syntax_by_token(language)).unwrap_or_else(|| SYNTAXES.find_syntax_plain_text());
    let mut html = ClassedHTMLGenerator::new_with_class_style(syntax, &SYNTAXES, ClassStyle::SpacedPrefixed { prefix: "syn-" });
    for line in LinesWithEndings::from(text) {
        if html.parse_html_for_line_which_includes_newline(line).is_err() { return ammonia::clean_text(text); }
    }
    html.finalize()
}

pub fn markdown(text: &str) -> String {
    if text.len() > 100_000 { return format!("<pre>{}</pre>", ammonia::clean_text(text)); }
    let mut events = Vec::new();
    let mut block: Option<(String, String)> = None;
    for event in Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        match event {
            Event::Start(Tag::CodeBlock(kind)) => {
                let language = match kind { CodeBlockKind::Fenced(name) => name.split_whitespace().next().unwrap_or("txt").to_owned(), _ => "txt".into() };
                block = Some((language, String::new()));
            }
            Event::Text(ref text) if block.is_some() => block.as_mut().unwrap().1.push_str(text),
            Event::End(TagEnd::CodeBlock) => {
                let (language, text) = block.take().unwrap();
                events.push(Event::Html(format!("<pre><code>{}</code></pre>", code(&text, &language)).into()));
            }
            // Raw user HTML is text, so only our generated spans can carry classes.
            Event::Html(text) | Event::InlineHtml(text) => events.push(Event::Text(text)),
            event => events.push(event),
        }
    }
    let mut output = String::new();
    html::push_html(&mut output, events.into_iter());
    ammonia::Builder::default().add_tag_attributes("span", &["class"]).clean(&output).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsafe_markup_is_not_executable() {
        let output = markdown("<script>alert(1)</script>\n\n[x](javascript:alert%281%29)\n\n<img src=x onerror=alert(1)>\n\n```html\n<script>alert(1)</script>\n```");
        assert!(!output.contains("<script>"));
        assert!(!output.contains("href=\"javascript:"));
        assert!(!output.contains("<img"));
    }

    #[test]
    fn source_preserves_newlines_and_escapes_html() {
        assert_eq!(code("<x>\r\n\n", "txt"), "&lt;x&gt;&#13;&#10;&#10;");
        for (ext, text) in [("rs", "fn main() {}\n"), ("ts", "const a: number = 1;\n"), ("kt", "val a = 1\n")] {
            let result = code(text, ext);
            assert!(result.contains("syn-"), "{ext}: {result}");
            assert!(!result.contains("style="));
        }
        assert!(code("<x>", "unknown").contains("&lt;x&gt;"));
        assert!(markdown("| a | b |\n|---|---|\n| 1 | 2 |").contains("<table>"));
    }

    #[test]
    fn large_documents_skip_highlighting() {
        let text = "let x = 1;\n".repeat(40_000);
        assert!(code(&text, "rs") == ammonia::clean_text(&text));
    }
}