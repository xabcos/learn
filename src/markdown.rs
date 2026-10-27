//! Lessons are Markdown. Readers get HTML that can't run anything: raw HTML is dropped, links
//! keep only http(s) addresses without credentials, and a picture shows only as one of the
//! lesson's own (`image:<id>`).
use pulldown_cmark::{
    CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, TextMergeStream, html,
};
use uuid::Uuid;

/// The Markdown lessons are written in.
pub(crate) const OPTIONS: Options = Options::ENABLE_STRIKETHROUGH.union(Options::ENABLE_TABLES);
/// How a body names one of its lesson's pictures: `![…](image:<id>)`.
const IMAGE: &str = "image:";

pub(crate) fn parser(body: &str) -> Parser<'_> {
    Parser::new_ext(body, OPTIONS)
}

/// The picture a destination names, if it is one of the lesson's.
pub(crate) fn image_id(destination: &str) -> Option<Uuid> {
    destination
        .strip_prefix(IMAGE)
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// The pictures a body shows, each once, in order.
pub(crate) fn pictures(body: &str) -> Vec<Uuid> {
    let mut shown = Vec::new();
    for event in parser(body) {
        if let Event::Start(Tag::Image { dest_url, .. }) = event
            && let Some(id) = image_id(&dest_url)
            && !shown.contains(&id)
        {
            shown.push(id);
        }
    }
    shown
}

/// Where a picture loads from, and its size.
pub(crate) struct Shown {
    pub(crate) src: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

/// A section the lesson's second- and third-level headings open, as the page lists them.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Section {
    pub(crate) anchor: String,
    pub(crate) title: String,
    pub(crate) level: u8,
}

/// The body as HTML, and its sections: each second- and third-level heading gets an anchor
/// (`s1`, `s2`, …) the page links to.
pub(crate) fn render(
    body: &str,
    picture: impl Fn(Uuid) -> Option<Shown>,
) -> (String, Vec<Section>) {
    let mut sections = Vec::new();
    let mut heading: Option<usize> = None;
    let mut image: Option<Option<Shown>> = None;
    let mut alt = String::new();
    let events = bolded(parser(body)).into_iter().filter_map(|event| match event {
        Event::Html(_) | Event::InlineHtml(_) => None,
        Event::Start(Tag::Image { dest_url, .. }) => {
            alt.clear();
            image = Some(image_id(&dest_url).and_then(&picture));
            None
        }
        Event::End(TagEnd::Image) => Some(match image.take().flatten() {
            Some(Shown { src, width, height }) => Event::Html(
                format!(
                    "<img src=\"{}\" alt=\"{}\" width={width} height={height} loading=lazy decoding=async referrerpolicy=no-referrer>",
                    escape(&src),
                    escape(&alt)
                )
                .into(),
            ),
            None => Event::Text(std::mem::take(&mut alt).into()),
        }),
        Event::Text(text) | Event::Code(text) if image.is_some() => {
            alt.push_str(&text);
            None
        }
        _ if image.is_some() => None,
        Event::Start(Tag::Heading {
            level: level @ (HeadingLevel::H2 | HeadingLevel::H3),
            classes,
            attrs,
            ..
        }) => {
            sections.push(Section {
                anchor: format!("s{}", sections.len() + 1),
                title: String::new(),
                level: if level == HeadingLevel::H2 { 2 } else { 3 },
            });
            heading = Some(sections.len() - 1);
            Some(Event::Start(Tag::Heading {
                level,
                id: Some(CowStr::from(sections[sections.len() - 1].anchor.clone())),
                classes,
                attrs,
            }))
        }
        Event::End(TagEnd::Heading(level)) => {
            heading = None;
            Some(Event::End(TagEnd::Heading(level)))
        }
        Event::Text(ref text) | Event::Code(ref text) if heading.is_some() => {
            if let Some(index) = heading {
                sections[index].title.push_str(text);
            }
            Some(event)
        }
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => {
            let safe = url::Url::parse(&dest_url).is_ok_and(|url| {
                matches!(url.scheme(), "https" | "http")
                    && url.username().is_empty()
                    && url.password().is_none()
            });
            Some(Event::Start(Tag::Link {
                link_type,
                dest_url: if safe { dest_url } else { "".into() },
                title,
                id,
            }))
        }
        other => Some(other),
    });
    let mut html = String::new();
    html::push_html(&mut html, events);
    (html, sections)
}

/// CommonMark leaves `**` as text when punctuation is just inside it and a letter just outside,
/// as in `一起玩。**把` or `看**“引号”**里`: it expects a space there, and Chinese writes none.
/// Within a run of plain text such stars are paired in order and become bold where punctuation
/// at the edge is what kept them apart; stars around letters were escaped and stay.
pub(crate) fn bolded<'a>(input: impl Iterator<Item = Event<'a>>) -> Vec<Event<'a>> {
    let mut events = Vec::new();
    let mut code = false;
    for event in TextMergeStream::new(input) {
        match &event {
            Event::Start(Tag::CodeBlock(_)) => code = true,
            Event::End(TagEnd::CodeBlock) => code = false,
            Event::Text(text) if !code && text.contains("**") => {
                let pieces: Vec<&str> = text.split("**").collect();
                events.push(Event::Text(pieces[0].to_string().into()));
                for pair in pieces[1..].chunks(2) {
                    match pair {
                        [inside, outside] if refused(inside) => events.extend([
                            Event::Start(Tag::Strong),
                            Event::Text(inside.to_string().into()),
                            Event::End(TagEnd::Strong),
                            Event::Text(outside.to_string().into()),
                        ]),
                        _ => events.push(Event::Text(format!("**{}", pair.join("**")).into())),
                    }
                }
                continue;
            }
            _ => {}
        }
        events.push(event);
    }
    events
}

/// Whether CommonMark refused bold around `inside` only for punctuation at its edge: the text
/// touches its stars, is no part of a longer run of stars, and starts or ends with punctuation.
fn refused(inside: &str) -> bool {
    let (Some(first), Some(last)) = (inside.chars().next(), inside.chars().next_back()) else {
        return false;
    };
    [first, last]
        .iter()
        .all(|edge| !edge.is_whitespace() && *edge != '*')
        && [first, last].iter().any(|edge| !edge.is_alphanumeric())
}

/// The body's words as one line of plain text: no markup and no picture descriptions.
pub(crate) fn words(body: &str) -> String {
    let mut text = String::new();
    let mut picture = false;
    for event in bolded(parser(body)) {
        match event {
            Event::Start(Tag::Image { .. }) => picture = true,
            Event::End(TagEnd::Image) => picture = false,
            Event::Text(part) | Event::Code(part) if !picture => text.push_str(&part),
            Event::SoftBreak
            | Event::HardBreak
            | Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::BlockQuote(_)
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::TableCell,
            ) => text.push(' '),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The start of the body's words, for lists and the page's description.
pub(crate) fn excerpt(body: &str) -> String {
    const CHARS: usize = 120;
    let line = words(body);
    if line.chars().count() <= CHARS {
        return line;
    }
    let cut: String = line.chars().take(CHARS).collect();
    format!("{}…", cut.trim_end())
}

/// The body's headings, which tell a revision's planner what the lesson covers.
pub(crate) fn headings(body: &str) -> Vec<String> {
    let mut headings = Vec::new();
    let mut heading: Option<String> = None;
    for event in parser(body) {
        match event {
            Event::Start(Tag::Heading { .. }) => heading = Some(String::new()),
            Event::End(TagEnd::Heading(_)) => headings.extend(heading.take()),
            Event::Text(text) | Event::Code(text) => {
                if let Some(heading) = &mut heading {
                    heading.push_str(&text);
                }
            }
            _ => {}
        }
    }
    headings
}

pub(crate) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(id: Uuid) -> Option<Shown> {
        (id == Uuid::from_u128(1)).then(|| Shown {
            src: "https://dl.example.com/a.jpg".into(),
            width: 1536,
            height: 1024,
        })
    }

    #[test]
    fn markdown_runs_nothing_and_shows_only_the_lessons_pictures() {
        let (html, _) = render(
            "<script>alert(1)</script>\n\n[x](javascript:alert(1)) [y](https://u:p@example.com) [z](https://example.com/z)\n\n![架构图](image:00000000-0000-0000-0000-000000000001)\n\n![别处的图](https://tracker.example.com/a.png) ![没有的图](image:00000000-0000-0000-0000-000000000002)",
            shown,
        );
        assert!(!html.contains("<script"));
        assert!(html.contains("<a href=\"\">x</a>"));
        assert!(html.contains("<a href=\"\">y</a>"));
        assert!(html.contains("<a href=\"https://example.com/z\">z</a>"));
        assert!(html.contains(
            "<img src=\"https://dl.example.com/a.jpg\" alt=\"架构图\" width=1536 height=1024"
        ));
        assert!(!html.contains("tracker.example.com"));
        assert!(html.contains("别处的图") && html.contains("没有的图"));
    }

    #[test]
    fn second_and_third_level_headings_open_the_sections() {
        let (html, sections) = render(
            "开头\n\n## 安装 `idf.py`\n\n正文\n\n### 常见的坑\n\n#### 更细\n\n## 参考资料",
            shown,
        );
        assert!(html.contains("<h2 id=\"s1\">安装 <code>idf.py</code></h2>"));
        assert!(html.contains("<h3 id=\"s2\">常见的坑</h3>"));
        assert!(html.contains("<h4>更细</h4>"));
        let listed: Vec<(&str, &str, u8)> = sections
            .iter()
            .map(|section| {
                (
                    section.anchor.as_str(),
                    section.title.as_str(),
                    section.level,
                )
            })
            .collect();
        assert_eq!(
            listed,
            [
                ("s1", "安装 idf.py", 2),
                ("s2", "常见的坑", 3),
                ("s3", "参考资料", 2)
            ]
        );
    }

    #[test]
    fn chinese_bold_next_to_punctuation_still_bolds() {
        let (html, _) = render("一起玩。**把这个记住**。看**“引号”**里", shown);
        assert!(html.contains("<strong>把这个记住</strong>"));
        assert!(html.contains("<strong>“引号”</strong>"));
        let (code, _) = render("```\na **“b”** c\n```", shown);
        assert!(code.contains("a **“b”** c"));
    }

    #[test]
    fn words_and_the_excerpt_leave_markup_and_pictures_out() {
        assert_eq!(
            words("## 标题\n\n第一段 **粗体**。\n\n![图注](image:x)\n\n- 列表"),
            "标题 第一段 粗体。 列表"
        );
        let long = "字".repeat(200);
        assert_eq!(excerpt(&long).chars().count(), 121);
        assert!(excerpt(&long).ends_with('…'));
        assert_eq!(
            pictures(
                "![](image:00000000-0000-0000-0000-000000000001) ![](image:00000000-0000-0000-0000-000000000001) ![](https://x.example/a.png)"
            ),
            [Uuid::from_u128(1)]
        );
        assert_eq!(
            headings("开头。\n\n## 安装 `idf.py`\n\n### 常见的坑"),
            ["安装 idf.py", "常见的坑"]
        );
    }
}
