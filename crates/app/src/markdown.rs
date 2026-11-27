//! Markdown for the Focus region: the subset LLM replies actually use. Headings, bullet and
//! numbered lists, quotes, rules, fenced code, and inline **bold**, *italic*, `code`, [links].
//! Hand-rolled to avoid a parser dependency; anything else renders as plain text.
// ponytail: no tables, nested emphasis inside code, or reference links; add when replies use them.

use gpui::{div, prelude::*, px, rgb, Div, FontStyle, FontWeight, HighlightStyle, Hsla, StyledText};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Span {
    Bold,
    Italic,
    Code,
    Link,
}

/// Strips inline markers; returns the visible text and byte ranges to style.
pub fn inline(src: &str) -> (String, Vec<(Range<usize>, Span)>) {
    let mut out = String::new();
    let mut spans = vec![];
    let b = src.as_bytes();
    let mut i = 0;
    let push_span = |out: &mut String, spans: &mut Vec<(Range<usize>, Span)>, inner: &str, kind: Span| {
        if kind == Span::Code {
            let start = out.len();
            out.push_str(inner);
            spans.push((start..out.len(), kind));
            return;
        }
        let (text, inner_spans) = inline(inner);
        let start = out.len();
        out.push_str(&text);
        spans.extend(inner_spans.into_iter().map(|(r, k)| (r.start + start..r.end + start, k)));
        spans.push((start..out.len(), kind));
    };
    while i < b.len() {
        let rest = &src[i..];
        let found = if let Some(r) = rest.strip_prefix('`') {
            r.find('`').map(|e| (1, e + 1, 1, Span::Code))
        } else if rest.starts_with("**") || rest.starts_with("__") {
            let m = &rest[..2];
            rest[2..].find(m).filter(|&e| e > 0).map(|e| (2, e + 2, 2, Span::Bold))
        } else if (rest.starts_with('*') || rest.starts_with('_')) && rest.len() > 1 && !rest[1..].starts_with(' ') {
            let m = &rest[..1];
            // `_` must sit on a word boundary, so snake_case identifiers stay intact.
            let boundary_ok = m != "_" || i == 0 || !b[i - 1].is_ascii_alphanumeric();
            rest[1..]
                .find(m)
                .filter(|&e| e > 0 && boundary_ok && (m != "_" || !rest.as_bytes().get(e + 2).is_some_and(|c| c.is_ascii_alphanumeric())))
                .map(|e| (1, e + 1, 1, Span::Italic))
        } else if rest.starts_with('[') {
            rest.find("](").and_then(|mid| rest[mid..].find(')').map(|end| (mid, end + mid))).map(|(mid, end)| (1, mid, end - mid + 1, Span::Link))
        } else {
            None
        };
        match found {
            Some((open, close_at, close_len, kind)) => {
                push_span(&mut out, &mut spans, &rest[open..close_at], kind);
                i += close_at + close_len;
            }
            None => {
                let ch = rest.chars().next().unwrap_or(' ');
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    (out, spans)
}

fn styled(src: &str, color: u32) -> Div {
    let (text, spans) = inline(src);
    let hl = spans.into_iter().map(|(r, k)| {
        let s = match k {
            Span::Bold => HighlightStyle { font_weight: Some(FontWeight::BOLD), ..Default::default() },
            Span::Italic => HighlightStyle { font_style: Some(FontStyle::Italic), ..Default::default() },
            Span::Code => HighlightStyle { color: Some(Hsla::from(rgb(0xe0af68))), background_color: Some(Hsla::from(rgb(0x171a20))), ..Default::default() },
            Span::Link => HighlightStyle { color: Some(Hsla::from(rgb(0x7aa2f7))), ..Default::default() },
        };
        (r, s)
    });
    div().min_w_0().text_color(rgb(color)).child(StyledText::new(text).with_highlights(hl.collect::<Vec<_>>()))
}

fn list_item(indent: usize, lead: String, body: &str, text: u32, dim: u32) -> Div {
    div().flex().gap_2().pl(px(indent as f32 * 6.0)).child(div().flex_none().text_color(rgb(dim)).child(lead)).child(styled(body, text).flex_1())
}

/// Ordered-list marker (`1.` / `1)`) and the text after it.
fn numbered(t: &str) -> Option<(&str, &str)> {
    let digits = t.find(|c: char| !c.is_ascii_digit())?;
    let rest = &t[digits..];
    (digits > 0 && (rest.starts_with(". ") || rest.starts_with(") "))).then(|| (&t[..digits], &rest[2..]))
}

/// Renders a reply. `code_block` draws fenced code so it matches the rest of the pane.
pub fn render(md: &str, text: u32, dim: u32, code_block: impl Fn(&str) -> Div) -> Div {
    let mut body = div().flex().flex_col().gap_1();
    let (mut in_code, mut code) = (false, String::new());
    for line in md.lines() {
        let t = line.trim_start();
        let indent = line.len() - t.len();
        if t.starts_with("```") {
            if in_code {
                body = body.child(code_block(code.trim_end_matches('\n')));
                code.clear();
            }
            in_code = !in_code;
        } else if in_code {
            code.push_str(line);
            code.push('\n');
        } else if t.is_empty() {
            body = body.child(div().h(px(4.0)));
        } else if let Some(h) = t.strip_prefix("# ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("### ")).or_else(|| t.strip_prefix("#### ")) {
            body = body.child(styled(h, text).font_weight(FontWeight::BOLD));
        } else if matches!(t, "---" | "***" | "___") {
            body = body.child(div().h(px(1.0)).my_1().bg(rgb(0x2f3440)));
        } else if let Some(q) = t.strip_prefix("> ").or_else(|| t.strip_prefix('>').filter(|q| q.is_empty())) {
            body = body.child(div().pl_2().border_l_2().border_color(rgb(0x2f3440)).child(styled(q, dim)));
        } else if let Some(item) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("+ ")) {
            body = body.child(list_item(indent, "•".into(), item, text, dim));
        } else if let Some((n, item)) = numbered(t) {
            body = body.child(list_item(indent, format!("{n}."), item, text, dim));
        } else {
            body = body.child(styled(line, text));
        }
    }
    if in_code && !code.is_empty() {
        body = body.child(code_block(code.trim_end_matches('\n'))); // unclosed fence (clamped reply)
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(s: &str) -> String {
        inline(s).0
    }

    #[test]
    fn inline_markers() {
        let (t, spans) = inline("**Task:** run `go test` now");
        assert_eq!(t, "Task: run go test now");
        assert_eq!(spans, vec![(0..5, Span::Bold), (10..17, Span::Code)]);
        assert_eq!(strip("use *select* with `default`"), "use select with default");
        assert_eq!(strip("see [the docs](https://go.dev)"), "see the docs");
        assert_eq!(strip("**bold with `code`**"), "bold with code");
    }

    #[test]
    fn plain_text_survives() {
        assert_eq!(strip("snake_case_name and a_b"), "snake_case_name and a_b");
        assert_eq!(strip("2 * 3 * 4"), "2 * 3 * 4");
        assert_eq!(strip("unclosed `tick and **bold"), "unclosed `tick and **bold");
        assert_eq!(strip("ünïcödé → ok"), "ünïcödé → ok");
        assert_eq!(numbered("12. step"), Some(("12", "step")));
        assert_eq!(numbered("1.5 is a float"), None);
    }
}
