//! A draft's body as readers get it: search marks gone, each picture it places drawn and
//! captioned, and links leading only to pages the models found.
use crate::{markdown, media::Picture};
use pulldown_cmark::{Event, Tag, TagEnd};
use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};
use url::Url;

/// How the body places a picture it asks for, `![](img:1)` on a line of its own, and one a lesson
/// being revised already shows, `![](keep:1)`.
const PLACEHOLDER: &str = "img:";
const KEPT: &str = "keep:";
/// Pictures a lesson may show, its cover included.
pub(crate) const LESSON_PICTURES: usize = 32;
/// Pictures the body may place: as many as a lesson may have, less the cover on top.
pub(crate) const PICTURES: usize = LESSON_PICTURES - 1;
/// Rounds of link checks before a body whose rewriting keeps forming new links is given up.
const LINK_ROUNDS: usize = 8;

/// A picture the draft asks for, drawn to its prompt, and the line under it.
#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct Wanted {
    pub(crate) prompt: String,
    pub(crate) caption: String,
}

/// How an address is told apart: public HTTPS only, without `www.`, the fragment, tracking
/// parameters or the trailing slash.
pub(crate) fn identity(value: &str) -> Option<String> {
    let mut url = Url::parse(value.trim()).ok().filter(public)?;
    url.set_fragment(None);
    let kept: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| !tracking(key))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    url.set_query(None);
    if !kept.is_empty() {
        url.query_pairs_mut().extend_pairs(kept);
    }
    if let Some(host) = url
        .host_str()
        .and_then(|host| host.strip_prefix("www."))
        .map(str::to_owned)
    {
        let _ = url.set_host(Some(&host));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Some(url.to_string())
}

/// Public HTTPS on the default port, to a named host.
fn public(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(url.host(), Some(url::Host::Domain(host)) if host.contains('.') && !host.ends_with(".local") && !host.ends_with(".localhost"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port_or_known_default() == Some(443)
}

fn tracking(key: &str) -> bool {
    key.starts_with("utm_")
        || key.starts_with("share_")
        || matches!(
            key,
            "fbclid" | "gclid" | "igshid" | "ref" | "ref_src" | "spm"
        )
}

/// The pages a body links to, told apart as `identity` does.
pub(crate) fn linked(body: &str) -> Vec<String> {
    markdown::parser(body)
        .filter_map(|event| match event {
            Event::Start(Tag::Link { dest_url, .. }) => identity(&dest_url),
            _ => None,
        })
        .collect()
}

/// A picture the body places, alone in its paragraph: a new one by its number among the pictures
/// asked for, or a figure the lesson showed before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Slot {
    New(usize),
    Kept(usize),
}

/// Every picture in the body, with its slot when it is a placeholder standing alone in its
/// paragraph: only there can a caption line follow it.
pub(crate) fn images(body: &str) -> Vec<(Range<usize>, Option<Slot>)> {
    let mut images = Vec::new();
    let mut paragraph: Option<Range<usize>> = None;
    let mut inside = 0..0;
    for (event, range) in markdown::parser(body).into_offset_iter() {
        // The words a picture describes itself with belong to it.
        if inside.start <= range.start && range.end <= inside.end {
            continue;
        }
        match event {
            Event::Start(Tag::Paragraph) => paragraph = Some(range),
            Event::End(TagEnd::Paragraph) => paragraph = None,
            Event::Start(Tag::Image { dest_url, .. }) => {
                let alone = paragraph.as_ref().is_some_and(|paragraph| {
                    body.get(paragraph.clone()).map(str::trim)
                        == body.get(range.clone()).map(str::trim)
                });
                images.push((range.clone(), slot(&dest_url).filter(|_| alone)));
                inside = range;
            }
            _ => {}
        }
    }
    images
}

/// The new pictures the body places, by number, each once and in order, `room` at most.
pub(crate) fn placed(body: &str, count: usize, room: usize) -> Vec<usize> {
    let mut placed = Vec::new();
    for (_, slot) in images(body) {
        if let Some(Slot::New(number)) = slot
            && (1..=count).contains(&number)
            && !placed.contains(&number)
        {
            placed.push(number);
        }
    }
    placed.truncate(room);
    placed
}

fn slot(destination: &str) -> Option<Slot> {
    match destination.strip_prefix(PLACEHOLDER) {
        Some(number) => number.parse().ok().map(Slot::New),
        None => destination.strip_prefix(KEPT)?.parse().ok().map(Slot::Kept),
    }
}

/// The body as readers get it: each placed picture with a caption line under it and no other
/// picture, and no link to an address the models never found. Reducing a link to its words can
/// let the brackets around it form another link, so links are checked until nothing changes;
/// each round that changes something removes a pair of brackets.
pub(crate) fn compose(
    body: &str,
    wanted: &[Wanted],
    pictures: &HashMap<usize, Picture>,
    kept: &[String],
    known: &HashSet<String>,
) -> Option<String> {
    let mut body = pictured(body, wanted, pictures, kept);
    for _ in 0..LINK_ROUNDS {
        let checked = unlinked(&body, known);
        if checked == body {
            return Some(body);
        }
        body = checked;
    }
    None
}

/// Applies edits that don't overlap, in order. An edit overlapping one before it, or cutting a
/// character, is skipped: a rewrite must never take the server down.
fn apply(body: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.sort_by_key(|(range, _)| range.start);
    let mut rewritten = String::with_capacity(body.len());
    let mut done = 0;
    for (range, replacement) in edits {
        if range.start < done
            || range.start > range.end
            || !body.is_char_boundary(range.start)
            || !body.is_char_boundary(range.end)
        {
            continue;
        }
        rewritten.push_str(&body[done..range.start]);
        rewritten.push_str(&replacement);
        done = range.end;
    }
    rewritten.push_str(&body[done..]);
    rewritten
}

/// The body without the marks search answers leave after a sentence, a link named after its
/// site in parentheses such as `([example.com](https://example.com/a))`: the lesson links its
/// words instead.
pub(crate) fn uncited(body: &str) -> String {
    let mut marks = Vec::new();
    let mut link: Option<(Range<usize>, String, String)> = None;
    for (event, range) in markdown::parser(body).into_offset_iter() {
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                let site = Url::parse(&dest_url)
                    .ok()
                    .and_then(|url| Some(url.host_str()?.trim_start_matches("www.").to_owned()))
                    .unwrap_or_default();
                link = Some((range, site, String::new()));
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, words)) = &mut link {
                    words.push_str(&text);
                }
            }
            Event::End(TagEnd::Link) => {
                let Some((range, site, words)) = link.take() else {
                    continue;
                };
                let before = body
                    .get(..range.start)
                    .and_then(|text| text.chars().next_back());
                let after = body.get(range.end..).and_then(|text| text.chars().next());
                if let (Some(open @ ('(' | '（')), Some(close @ (')' | '）'))) = (before, after)
                    && !site.is_empty()
                    && words.trim().trim_start_matches("www.") == site
                {
                    marks.push((
                        range.start - open.len_utf8()..range.end + close.len_utf8(),
                        String::new(),
                    ));
                }
            }
            _ => {}
        }
    }
    apply(body, marks)
}

/// Each placed picture with a caption line under it, and each kept figure as it was, the first
/// time it is placed; every other picture goes, including a placed one that didn't come.
fn pictured(
    body: &str,
    wanted: &[Wanted],
    pictures: &HashMap<usize, Picture>,
    kept: &[String],
) -> String {
    let mut shown = HashSet::new();
    let edits = images(body)
        .into_iter()
        .map(|(range, slot)| {
            let figure = slot
                .filter(|slot| shown.insert(*slot))
                .and_then(|slot| match slot {
                    Slot::New(number) => {
                        let caption = &wanted.get(number.checked_sub(1)?)?.caption;
                        Some(figure(pictures.get(&number)?, caption))
                    }
                    Slot::Kept(number) => kept.get(number.checked_sub(1)?).cloned(),
                })
                .unwrap_or_default();
            (range, figure)
        })
        .collect();
    apply(body, edits)
}

/// The body with a link to an address the models never found, or any link inside another link,
/// reduced to its words.
fn unlinked(body: &str, known: &HashSet<String>) -> String {
    let mut edits = Vec::new();
    let mut kept = false;
    // A link being reduced: where it is, how deep inside it the parser is, and its words.
    let mut dropping: Option<(Range<usize>, usize, String)> = None;
    for (event, range) in markdown::parser(body).into_offset_iter() {
        if let Some((link, depth, words)) = &mut dropping {
            match event {
                Event::Start(Tag::Link { .. }) => *depth += 1,
                Event::End(TagEnd::Link) => *depth -= 1,
                Event::Text(text) | Event::Code(text) => words.push_str(&text),
                Event::SoftBreak | Event::HardBreak => words.push(' '),
                _ => {}
            }
            if *depth == 0 {
                edits.push((link.clone(), plain(words)));
                dropping = None;
            }
            continue;
        }
        match event {
            Event::Start(Tag::Link { dest_url, .. }) => {
                if !kept && identity(&dest_url).is_some_and(|address| known.contains(&address)) {
                    kept = true;
                } else {
                    dropping = Some((range, 1, String::new()));
                }
            }
            Event::End(TagEnd::Link) => kept = false,
            _ => {}
        }
    }
    apply(body, edits)
}

/// A picture and its caption line: what it shows, then that it was drawn.
fn figure(picture: &Picture, caption: &str) -> String {
    let caption = plain(caption);
    let line = if caption.is_empty() {
        "AI 生成".to_owned()
    } else {
        format!("{caption}（AI 生成）")
    };
    format!("![{caption}](image:{})\n*{line}*", picture.id)
}

/// Text set inside Markdown as plain words: one line, with Markdown's marks escaped.
pub(crate) fn plain(text: &str) -> String {
    let mut plain = String::new();
    for c in text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
    {
        if matches!(
            c,
            '\\' | '*' | '_' | '[' | ']' | '`' | '<' | '>' | '!' | '#' | '~' | '|'
        ) {
            plain.push('\\');
        }
        plain.push(c);
    }
    plain
}

/// The body of a lesson being revised, with each figure it shows (a picture leading a paragraph
/// of its own, the caption line under it) standing as `![](keep:N)`; and the figures as they were.
pub(crate) fn figures(body: &str) -> (String, Vec<String>) {
    let mut paragraph = None;
    let mut found = Vec::new();
    for (event, range) in markdown::parser(body).into_offset_iter() {
        match event {
            Event::Start(Tag::Paragraph) => paragraph = Some(range),
            Event::Start(Tag::Image { dest_url, .. }) => {
                if let Some(paragraph) = paragraph.take()
                    && paragraph.start == range.start
                    && markdown::image_id(&dest_url).is_some()
                {
                    // The blank line after it stays, so the placeholder keeps its paragraph.
                    let end = paragraph.start + body[paragraph].trim_end().len();
                    found.push(range.start..end);
                }
            }
            _ => {}
        }
    }
    let kept = found
        .iter()
        .map(|range| body[range.clone()].to_owned())
        .collect();
    let edits = found
        .into_iter()
        .enumerate()
        .map(|(index, range)| (range, format!("![]({KEPT}{})", index + 1)))
        .collect();
    (apply(body, edits), kept)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    fn picture() -> Picture {
        Picture {
            id: Uuid::from_u128(7),
            url: "https://media.example/files/a.jpg".into(),
            width: 1536,
            height: 1024,
            expires_at: Utc::now(),
        }
    }

    fn wanted(caption: &str) -> Wanted {
        Wanted {
            prompt: String::new(),
            caption: caption.into(),
        }
    }

    fn known(addresses: &[&str]) -> HashSet<String> {
        addresses
            .iter()
            .filter_map(|address| identity(address))
            .collect()
    }

    /// Where a body's links lead once the page renders it.
    fn hrefs(body: &str) -> Vec<String> {
        let (html, _) = markdown::render(body, |_| {
            Some(markdown::Shown {
                src: "https://media.example/a.jpg".into(),
                width: 1,
                height: 1,
            })
        });
        html.split("href=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn a_revision_keeps_the_figures_it_moves_and_drops_the_rest() {
        let figure = |id: u128, caption: &str| {
            format!(
                "![{caption}](image:{})\n*{caption}（AI 生成）*",
                Uuid::from_u128(id)
            )
        };
        let body = format!(
            "开头，\n\n{}\n\n中间 ![行内](image:{}) 的图。\n\n{}\n\n结尾。",
            figure(1, "甲"),
            Uuid::from_u128(2),
            figure(3, "乙")
        );
        let (shown, kept) = figures(&body);
        assert_eq!(
            shown,
            format!(
                "开头，\n\n![](keep:1)\n\n中间 ![行内](image:{}) 的图。\n\n![](keep:2)\n\n结尾。",
                Uuid::from_u128(2)
            )
        );
        assert_eq!(kept, [figure(1, "甲"), figure(3, "乙")]);
        // The writer moved the second up and dropped the first; a figure it names twice shows
        // once, and the picture inside a paragraph goes like any picture not placed.
        let revised = "![](keep:2)\n\n新的开头。\n\n![](keep:2)\n\n![](keep:9)\n\n中间 ![行内](image:x) 的图。";
        assert_eq!(
            pictured(revised, &[], &HashMap::new(), &kept),
            format!("{}\n\n新的开头。\n\n\n\n\n\n中间  的图。", figure(3, "乙"))
        );
    }

    #[test]
    fn the_pages_a_lesson_linked_to_are_told_apart_like_any_address() {
        let body = "看 [工具](https://www.known.example/tool/#readme)、<https://x.com/maker/status/1> \
            和 [明文](http://plain.example/a)。\n\n![](image:abc)";
        assert_eq!(
            linked(body),
            [
                identity("https://known.example/tool").unwrap(),
                identity("https://x.com/maker/status/1").unwrap()
            ]
        );
        assert_eq!(
            identity("https://www.example.com/page/?utm_source=x#top"),
            identity("https://example.com/page")
        );
        assert_eq!(identity("https://localhost/page"), None);
        assert_eq!(identity("https://example.com:8443/page"), None);
        assert_eq!(identity("not a url"), None);
    }

    #[test]
    fn only_placeholders_alone_in_their_paragraph_are_placed_each_once_in_order() {
        let body = "![](img:2)\n\n![x](img:1)\n\n![again](img:2)\n\n文字 ![](img:3) 中间\n\n\
            | a |\n|---|\n| ![](img:4) |\n\n[![](img:5)](https://known.example/a)\n\n\
            ## ![](img:6)\n\n![](img:9)\n\n![](image:abc)\n\n![](img:x)";
        assert_eq!(placed(body, 6, PICTURES), [2, 1]);
        let many: String = (1..=40).map(|n| format!("![](img:{n})\n\n")).collect();
        assert_eq!(
            placed(&many, 40, PICTURES),
            (1..=PICTURES).collect::<Vec<_>>()
        );
    }

    #[test]
    fn placed_pictures_get_caption_lines_and_links_lead_only_where_models_found() {
        let known = known(&["https://known.example/tool", "https://x.com/maker/status/1"]);
        let body = "开头看 [工具](https://www.known.example/tool/#readme) 和 [别处](https://elsewhere.example/b)，\
            还有 [明文](http://known.example/tool)、<https://elsewhere.example/c> 与 [**帖子**](https://x.com/maker/status/1)。\n\n\
            ![](img:1)\n\n![](img:2)\n\n![别人的图](https://elsewhere.example/p.png)\n\n![](img:1)\n\n结尾。";
        let pictures = HashMap::from([(1, picture())]);
        let composed = compose(
            body,
            &[wanted("一只*猫*"), wanted("不会来")],
            &pictures,
            &[],
            &known,
        )
        .unwrap();
        let id = Uuid::from_u128(7);
        assert_eq!(
            composed,
            format!(
                "开头看 [工具](https://www.known.example/tool/#readme) 和 别处，\
            还有 明文、https://elsewhere.example/c 与 [**帖子**](https://x.com/maker/status/1)。\n\n\
            ![一只\\*猫\\*](image:{id})\n*一只\\*猫\\*（AI 生成）*\n\n\n\n\n\n\n\n结尾。"
            )
        );
        assert_eq!(
            hrefs(&composed),
            [
                "https://www.known.example/tool/#readme",
                "https://x.com/maker/status/1"
            ]
        );
    }

    #[test]
    fn no_rewriting_lets_through_a_link_no_model_found() {
        let known = known(&["https://known.example/a"]);
        for body in [
            "[<https://a.example> <https://b.example>](https://c.example) 后文",
            "[`[click](https://evil.example)`](https://unknown.example)",
            "[click]![](img:9)(https://evil.example)",
            "[好的](https://known.example/a) 和 [<https://evil.example>](https://known.example/a)",
            "[多行\n标签](https://evil.example)",
            "[ref][r]\n\n[r]: https://evil.example",
            "<https://evil.example> 与 [x](javascript:alert(1))",
            "[click][(https://evil.example)](https://unknown.example)",
            "[click][](https://unknown.example)(https://evil.example)",
            "[[click][](https://unknown.example)(https://evil.example)](https://unknown.example)",
        ] {
            let composed = compose(body, &[], &HashMap::new(), &[], &known).unwrap();
            for href in hrefs(&composed) {
                assert_eq!(
                    href, "https://known.example/a",
                    "{body:?} became {composed:?}"
                );
            }
        }
    }

    #[test]
    fn code_tables_and_links_around_pictures_keep_their_shape() {
        let code = "```py\nx = '''a\n\n\n\nb'''\n```";
        let body = format!(
            "    缩进的代码\n\n{code}\n\n| a | b |\n|---|---|\n| ![](img:1) | c |\n\n\
             [![](img:1)](https://known.example/a)\n\n![](img:1)"
        );
        let pictures = HashMap::from([(1, picture())]);
        let composed = compose(
            &body,
            &[wanted("猫")],
            &pictures,
            &[],
            &known(&["https://known.example/a"]),
        )
        .unwrap();
        assert_eq!(
            composed,
            format!(
                "    缩进的代码\n\n{code}\n\n| a | b |\n|---|---|\n|  | c |\n\n\
                 [](https://known.example/a)\n\n![猫](image:{})\n*猫（AI 生成）*",
                Uuid::from_u128(7)
            )
        );
    }

    #[test]
    fn overlapping_or_broken_edits_are_skipped_instead_of_failing() {
        let edit = |range: Range<usize>, text: &str| (range, text.to_owned());
        assert_eq!(
            apply(
                "abcdef",
                vec![edit(5..6, "Z"), edit(1..3, "X"), edit(2..4, "Y")]
            ),
            "aXdeZ"
        );
        assert_eq!(apply("中文", vec![edit(1..2, "X")]), "中文");
        assert_eq!(
            apply(
                "abc",
                vec![edit(Range { start: 2, end: 1 }, "X"), edit(2..9, "Y")]
            ),
            "abc"
        );
    }

    #[test]
    fn a_drawn_picture_says_so_under_its_caption() {
        assert_eq!(
            figure(&picture(), " 架构  示意 "),
            format!(
                "![架构 示意](image:{})\n*架构 示意（AI 生成）*",
                Uuid::from_u128(7)
            )
        );
        assert_eq!(
            figure(&picture(), ""),
            format!("![](image:{})\n*AI 生成*", Uuid::from_u128(7))
        );
        assert_eq!(
            figure(&picture(), "[链接](x) #1 <b>"),
            format!(
                "![\\[链接\\](x) \\#1 \\<b\\>](image:{})\n*\\[链接\\](x) \\#1 \\<b\\>（AI 生成）*",
                Uuid::from_u128(7)
            )
        );
    }

    #[test]
    fn source_marks_after_sentences_go_and_named_links_stay() {
        let body = "它不需要越狱。([linpados.com](https://linpados.com/))\n\n\
            见 [GitHub 仓库](https://github.com/fspecii/LinPad)（[github.com](https://www.github.com/fspecii/LinPad)）\
            和 ([官网](https://linpados.com/))，以及 [linpados.com](https://linpados.com/) 本身。";
        assert_eq!(
            uncited(body),
            "它不需要越狱。\n\n见 [GitHub 仓库](https://github.com/fspecii/LinPad)\
            和 ([官网](https://linpados.com/))，以及 [linpados.com](https://linpados.com/) 本身。"
        );
        assert_eq!(
            compose(&uncited(body), &[], &HashMap::new(), &[], &HashSet::new()).unwrap(),
            "它不需要越狱。\n\n见 GitHub 仓库和 (官网)，以及 linpados.com 本身。"
        );
    }

    #[test]
    fn a_link_reduced_to_its_words_keeps_them_apart() {
        let composed = compose(
            "去 [New\nYork](https://unknown.example) 看看",
            &[],
            &HashMap::new(),
            &[],
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(composed, "去 New York 看看");
    }
}
