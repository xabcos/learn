//! The pages: `/` maps the courses by group, `/{course}` lists a course's lessons by chapter, and
//! `/{course}/{number}` reads a lesson with its course's contents on its left and its own sections
//! on its right. Every page is the same for every reader and is cached as one.
use crate::{
    courses::{self, Course, Listed},
    error::{AppError, Result},
    markdown::{self, Section, Shown, escape},
    state::AppState,
};
use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use chrono::{DateTime, Utc};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fmt::Write};
use uuid::Uuid;

/// The site's name.
const SITE: &str = "XABC 学习";
/// Asset versions, raised when a file changes so caches take the new one.
const STYLES: &str = "/assets/styles.css?v=1";
const SCRIPT: &str = "/assets/learn.js?v=1";
const PUBLIC_CACHE: HeaderValue =
    HeaderValue::from_static("public, max-age=60, stale-while-revalidate=300");
/// Lessons new or revised within this many days lead a course's page, unless they are all it has.
const RECENT_DAYS: i64 = 7;
/// How many of them, at most.
const RECENT: usize = 5;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home))
        .route("/sitemap.xml", get(sitemap))
        .route("/robots.txt", get(robots))
        .route("/{course}", get(course))
        .route("/{course}/{number}", get(lesson))
}

/// A page ready to serve, and its validator.
struct Rendered {
    body: Bytes,
    etag: String,
}

impl Rendered {
    fn new(body: String) -> Self {
        let digest = Sha256::digest(body.as_bytes());
        let mut etag = String::with_capacity(34);
        etag.push('"');
        for byte in digest.iter().take(16) {
            let _ = write!(etag, "{byte:02x}");
        }
        etag.push('"');
        Self {
            body: Bytes::from(body),
            etag,
        }
    }
}

/// Serves a page, or 304 when the reader already holds this exact one. Proxies that recompress
/// the body (Cloudflare does) weaken the tag to W/"…", which still compares equal.
fn conditional(headers: &HeaderMap, body: String, content_type: &'static str) -> Response {
    let rendered = Rendered::new(body);
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|candidate| {
                let candidate = candidate.trim();
                candidate == "*"
                    || candidate.strip_prefix("W/").unwrap_or(candidate) == rendered.etag
            })
        });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        ([(header::CONTENT_TYPE, content_type)], rendered.body).into_response()
    };
    let headers = response.headers_mut();
    if let Ok(etag) = HeaderValue::from_str(&rendered.etag) {
        headers.insert(header::ETAG, etag);
    }
    headers.insert(header::CACHE_CONTROL, PUBLIC_CACHE);
    response
}

fn html(headers: &HeaderMap, page: String) -> Response {
    conditional(headers, page, "text/html; charset=utf-8")
}

fn absolute(state: &AppState, path: &str) -> String {
    format!(
        "{}{path}",
        state.config.public_base_url.as_str().trim_end_matches('/')
    )
}

fn day(state: &AppState, at: DateTime<Utc>, format: &str) -> String {
    at.with_timezone(&state.config.timezone)
        .format(format)
        .to_string()
}

fn time(state: &AppState, at: DateTime<Utc>, format: &str) -> String {
    format!(
        "<time datetime=\"{}\">{}</time>",
        at.to_rfc3339(),
        day(state, at, format)
    )
}

/// What a page tells about itself beyond its body.
struct Head<'a> {
    path: &'a str,
    title: &'a str,
    description: &'a str,
    /// The picture standing for the page where it is shared.
    image: Option<&'a str>,
    /// Structured data for search engines.
    schema: Option<serde_json::Value>,
}

/// A whole page: the bar on top, `content`, and the footer.
fn layout(state: &AppState, head: Head<'_>, class: &str, content: &str) -> String {
    let canonical = escape(&absolute(state, head.path));
    let title = escape(head.title);
    let description = escape(head.description);
    let card = match head.image {
        Some(image) => format!(
            "<meta property=\"og:image\" content=\"{}\"><meta name=\"twitter:card\" content=\"summary_large_image\">",
            escape(image)
        ),
        None => "<meta name=\"twitter:card\" content=\"summary\">".into(),
    };
    let schema = head
        .schema
        .map(|value| {
            format!(
                "<script type=\"application/ld+json\">{}</script>",
                value
                    .to_string()
                    .replace('&', "\\u0026")
                    .replace('<', "\\u003c")
                    .replace('>', "\\u003e")
            )
        })
        .unwrap_or_default();
    let og_type = if schema.is_empty() {
        "website"
    } else {
        "article"
    };
    let home = if head.path == "/" {
        " aria-current=\"page\""
    } else {
        ""
    };
    format!(
        r##"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title}</title><meta name="description" content="{description}"><meta name="theme-color" media="(prefers-color-scheme: light)" content="#fbfbf9"><meta name="theme-color" media="(prefers-color-scheme: dark)" content="#101216">
<link rel="canonical" href="{canonical}"><meta property="og:type" content="{og_type}"><meta property="og:title" content="{title}"><meta property="og:description" content="{description}"><meta property="og:url" content="{canonical}"><meta property="og:site_name" content="{SITE}">{card}
<link rel="icon" type="image/svg+xml" href="/assets/icon.svg"><link rel="stylesheet" href="{STYLES}"><script src="{SCRIPT}" defer></script>{schema}</head>
<body class="{class}"><a class="skip" href="#main">跳到正文</a>
<header class="top"><div class="top-inner"><a class="brand" href="/"><img src="/assets/icon.svg" alt="" width="29" height="22"><span>{SITE}</span></a><nav class="top-links" aria-label="站点"><a href="/"{home}>全部课程</a><a class="source" href="https://github.com/xabcos/learn" aria-label="源代码" title="源代码 · AGPL-3.0"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 19c-5 1.5-5-2.5-7-3m14 6v-3.87a3.37 3.37 0 0 0-.94-2.61c3.14-.35 6.44-1.54 6.44-7A5.44 5.44 0 0 0 20 4.77 5.07 5.07 0 0 0 19.91 1S18.73.65 16 2.48a13.38 13.38 0 0 0-7 0C6.27.65 5.09 1 5.09 1A5.07 5.07 0 0 0 5 4.77a5.44 5.44 0 0 0-1.5 3.78c0 5.42 3.3 6.61 6.44 7A3.37 3.37 0 0 0 9 18.13V22"/></svg></a></nav></div></header>
<main id="main" class="page">{content}</main>
<footer class="foot"><p>{SITE} · 课程由 XABC 持续编写与修订，内容以官方文档为准 · <a href="https://github.com/xabcos/learn">源代码（AGPL-3.0）</a></p></footer>
<button class="to-top" type="button" aria-label="回到顶部" title="回到顶部" hidden><svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m6 12 6-6 6 6M12 6v12"/></svg></button>
</body></html>"##
    )
}

/// A page saying what went wrong; it needs nothing from the database.
pub(crate) fn problem(status: StatusCode, message: &str) -> String {
    format!(
        r##"<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex">
<title>{} · {SITE}</title><link rel="icon" type="image/svg+xml" href="/assets/icon.svg"><link rel="stylesheet" href="{STYLES}"></head>
<body class="problem"><header class="top"><div class="top-inner"><a class="brand" href="/"><img src="/assets/icon.svg" alt="" width="29" height="22"><span>{SITE}</span></a></div></header>
<main id="main" class="page"><section class="hero"><p class="code">{}</p><h1>{}</h1><p class="lead"><a href="/">回到全部课程</a></p></section></main></body></html>"##,
        escape(message),
        status.as_u16(),
        escape(message)
    )
}

/// How many lessons and chapters a course has.
fn size(lessons: usize, chapters: usize) -> String {
    format!("{lessons} 课 · {chapters} 章")
}

/// `/`: the courses by group, with how many lessons each has and when one last changed.
async fn home(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let courses = courses::shown(&state).await?;
    let counts: HashMap<String, (i64, DateTime<Utc>)> =
        sqlx::query_as::<_, (String, i64, DateTime<Utc>)>(
            "SELECT course, count(*), max(updated_at) FROM lessons GROUP BY course",
        )
        .fetch_all(&state.pool)
        .await?
        .into_iter()
        .map(|(course, lessons, updated)| (course, (lessons, updated)))
        .collect();
    let shown = |course: &Course| counts.get(&course.id);
    let lessons: i64 = courses
        .iter()
        .filter_map(shown)
        .map(|(count, ..)| count)
        .sum();
    let updated = courses
        .iter()
        .filter_map(shown)
        .map(|(_, updated)| *updated)
        .max();
    let mut content = format!(
        "<section class=hero><h1>系统地学 AI 与硬件开发</h1><p class=lead>XABC 写的系列课程，从入门到实战，一课接一课往前走。</p><p class=stats>{} 门课 · {lessons} 课{}</p></section><div class=map>",
        courses.len(),
        updated
            .map(|at| format!(" · 最近更新 {}", time(&state, at, "%m-%d %H:%M")))
            .unwrap_or_default()
    );
    for (group, members) in courses::groups(&courses) {
        let _ = write!(
            content,
            "<section class=group><h2>{}</h2><ul class=courses>",
            escape(group)
        );
        for course in members {
            let meta = match shown(course) {
                Some((count, updated)) => {
                    format!("{count} 课 · 更新 {}", time(&state, *updated, "%m-%d"))
                }
                None => "第一课正在写".into(),
            };
            let _ = write!(
                content,
                "<li><a href=\"{}\"><span class=course-title>{}</span><span class=course-meta>{meta}</span></a><p>{}</p></li>",
                courses::path(&course.id, None),
                escape(&course.title),
                escape(&course.intro),
            );
        }
        content.push_str("</ul></section>");
    }
    content.push_str("</div>");
    let page = layout(
        &state,
        Head {
            path: "/",
            title: &format!("{SITE} · 系统地学 AI 与硬件开发"),
            description: "XABC 写的 AI 与 ESP32 系列课程：入门、提示词、AI 编程、Agent、RAG、模型 API、本地模型、创作与实战项目，持续更新，以官方文档为准。",
            image: None,
            schema: None,
        },
        "home",
        &content,
    );
    Ok(html(&headers, page))
}

/// Beside a course's page: every course by group, this one marked.
fn catalogue(courses: &[Course], current: &str) -> String {
    let mut html = "<nav class=side aria-label=全部课程>".to_owned();
    for (group, members) in courses::groups(courses) {
        let _ = write!(html, "<h2>{}</h2><ul>", escape(group));
        for course in members {
            let _ = write!(
                html,
                "<li><a href=\"{}\"{}>{}</a></li>",
                courses::path(&course.id, None),
                if course.id == current {
                    " aria-current=\"page\""
                } else {
                    ""
                },
                escape(&course.title)
            );
        }
        html.push_str("</ul>");
    }
    html.push_str("</nav>");
    html
}

/// A lesson's number and title, as lists show them.
fn named(lesson: &Listed) -> String {
    format!(
        "<span class=num>{}</span><span class=name>{}</span>",
        lesson.number,
        escape(&lesson.title)
    )
}

/// `/{course}`: the course's lessons by chapter, after the latest new or revised ones, with every
/// course on the left and the chapters on the right.
async fn course(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let courses = courses::shown(&state).await?;
    let course = courses
        .iter()
        .find(|course| course.id == id)
        .ok_or_else(|| AppError::NotFound("没有这门课".into()))?;
    let lessons = courses::lessons(&state, &course.id).await?;
    let numbers = courses::numbered(lessons.iter().map(|lesson| lesson.chapter.as_str()));
    let chapters = numbers.last().copied().unwrap_or_default();
    let mut main = format!(
        "<header class=head><p class=crumbs><a href=\"/\">全部课程</a> › {}</p><h1>{}</h1><p class=lead>{}</p>",
        escape(&course.grp),
        escape(&course.title),
        escape(&course.intro),
    );
    match lessons.iter().map(|lesson| lesson.updated_at).max() {
        Some(updated) => {
            let _ = write!(
                main,
                "<p class=stats>{} · 最近更新 {}</p></header>",
                size(lessons.len(), chapters),
                time(&state, updated, "%m-%d")
            );
        }
        None => main.push_str("<p class=stats>第一课正在写。</p></header>"),
    }
    let since = Utc::now() - chrono::Duration::days(RECENT_DAYS);
    let mut recent: Vec<&Listed> = lessons
        .iter()
        .filter(|lesson| lesson.updated_at > since)
        .collect();
    recent.sort_by_key(|lesson| std::cmp::Reverse(lesson.updated_at));
    recent.truncate(RECENT);
    if !recent.is_empty() && recent.len() < lessons.len() {
        main.push_str("<section class=recent><h2>最近更新</h2><ul class=lessons>");
        for lesson in recent {
            // A lesson revised after it went out says so; a new one says it is new.
            let revised = lesson.updated_at - lesson.published_at > chrono::Duration::hours(1);
            let _ = write!(
                main,
                "<li><a href=\"{}\">{}<span class=badge>{}</span></a>{}</li>",
                courses::path(&course.id, Some(lesson.number)),
                named(lesson),
                if revised { "更新" } else { "新" },
                time(&state, lesson.updated_at, "%m-%d")
            );
        }
        main.push_str("</ul></section>");
    }
    let mut aside = "<nav class=aside aria-label=章节><h2>章节</h2><ol>".to_owned();
    let mut chapter = 0;
    for (lesson, number) in lessons.iter().zip(&numbers) {
        if *number != chapter {
            if chapter > 0 {
                main.push_str("</ol></section>");
            }
            chapter = *number;
            let _ = write!(
                main,
                "<section class=chapter id=c{number}><h2><span class=num>第 {number} 章</span>{}</h2><ol class=lessons>",
                escape(&lesson.chapter)
            );
            let _ = write!(
                aside,
                "<li><a href=\"#c{number}\"><span class=num>{number}</span>{}</a></li>",
                escape(&lesson.chapter)
            );
        }
        let _ = write!(
            main,
            "<li><a href=\"{}\">{}</a>{}</li>",
            courses::path(&course.id, Some(lesson.number)),
            named(lesson),
            time(&state, lesson.updated_at, "%m-%d")
        );
    }
    if chapter > 0 {
        main.push_str("</ol></section>");
    }
    aside.push_str("</ol></nav>");
    let content = format!(
        "<div class=docs>{}<div class=main>{main}</div>{}</div>",
        catalogue(&courses, &course.id),
        if chapters > 1 { aside.as_str() } else { "" }
    );
    let page = layout(
        &state,
        Head {
            path: &courses::path(&course.id, None),
            title: &format!("{} · {SITE}", course.title),
            description: &course.intro,
            image: None,
            schema: None,
        },
        "course",
        &content,
    );
    Ok(html(&headers, page))
}

/// A lesson as its page reads it.
#[derive(sqlx::FromRow)]
struct Lesson {
    id: Uuid,
    chapter: String,
    title: String,
    body: String,
    excerpt: String,
    cover: Option<Uuid>,
    revision: i32,
    published_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

/// A lesson's picture as stored.
#[derive(sqlx::FromRow)]
struct Image {
    id: Uuid,
    url: String,
    width: i32,
    height: i32,
    expires_at: DateTime<Utc>,
}

/// `/{course}/{number}`: the lesson, with its course's contents on the left, its sections on the
/// right, and the lessons before and after it underneath.
async fn lesson(
    State(state): State<AppState>,
    Path((id, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let missing = || AppError::NotFound("没有这一课".into());
    let number: i32 = number.parse().map_err(|_| missing())?;
    let course = courses::find(&state, &id).await?.ok_or_else(missing)?;
    let lesson: Lesson = sqlx::query_as(
        "SELECT id, chapter, title, body, excerpt, cover, revision, published_at, updated_at
         FROM lessons WHERE course = $1 AND number = $2",
    )
    .bind(&course.id)
    .bind(number)
    .fetch_optional(&state.pool)
    .await?
    .ok_or_else(missing)?;
    let images: HashMap<Uuid, Image> = sqlx::query_as::<_, Image>(
        "SELECT id, url, width, height, expires_at FROM images WHERE lesson = $1",
    )
    .bind(lesson.id)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|image| (image.id, image))
    .collect();
    let now = Utc::now();
    let picture = |id: Uuid| {
        let image = images.get(&id).filter(|image| image.expires_at > now)?;
        Some(Shown {
            src: image.url.clone(),
            width: image.width,
            height: image.height,
        })
    };
    let (body, sections) = markdown::render(&lesson.body, picture);
    let cover = lesson.cover.and_then(picture);
    let lessons = courses::lessons(&state, &course.id).await?;
    let numbers = courses::numbered(lessons.iter().map(|listed| listed.chapter.as_str()));
    let at = lessons.iter().position(|listed| listed.number == number);
    let chapter = at.map_or(1, |at| numbers[at]);
    let contents = contents(&course, &lessons, &numbers, number);
    let path = courses::path(&course.id, Some(number));
    let mut main = format!(
        "<article class=\"main lesson\"><p class=crumbs><a href=\"/\">全部课程</a> › <a href=\"{}\">{}</a> › 第 {chapter} 章 {}</p><details class=toc><summary>课程目录</summary>{contents}</details>",
        courses::path(&course.id, None),
        escape(&course.title),
        escape(&lesson.chapter),
    );
    if let Some(cover) = &cover {
        let _ = write!(
            main,
            "<img class=cover src=\"{}\" width={} height={} alt=\"\" decoding=async referrerpolicy=no-referrer>",
            escape(&cover.src),
            cover.width,
            cover.height
        );
    }
    let _ = write!(
        main,
        "<h1>{}</h1><p class=meta>第 {number} 课 · 最后更新 {} · 第 {} 版</p><div class=prose>{body}</div>{}<p class=back><a href=\"{}\">← {} 目录</a></p></article>",
        headline(&lesson.title),
        time(&state, lesson.updated_at, "%Y-%m-%d"),
        lesson.revision,
        steps(&course, &lessons, number),
        courses::path(&course.id, None),
        escape(&course.title),
    );
    let content = format!(
        "<div class=docs><nav class=side aria-label=课程目录><a class=back href=\"/\">← 全部课程</a><a class=side-course href=\"{}\">{}</a><p class=side-meta>{}</p>{contents}</nav>{main}{}</div>",
        courses::path(&course.id, None),
        escape(&course.title),
        size(lessons.len(), numbers.last().copied().unwrap_or_default()),
        outline(&sections),
    );
    let canonical = absolute(&state, &path);
    let mut schema = json!({
        "@context": "https://schema.org", "@type": "Article", "headline": lesson.title,
        "description": lesson.excerpt, "url": canonical, "mainEntityOfPage": canonical,
        "datePublished": lesson.published_at.to_rfc3339(),
        "dateModified": lesson.updated_at.to_rfc3339(),
        "author": {"@type": "Organization", "name": "XABC"},
        "publisher": {"@type": "Organization", "name": SITE, "url": absolute(&state, "/")},
        "isPartOf": {"@type": "Course", "name": course.title, "description": course.intro,
                     "url": absolute(&state, &courses::path(&course.id, None))},
    });
    if let Some(cover) = &cover {
        schema["image"] = json!(cover.src);
    }
    let page = layout(
        &state,
        Head {
            path: &path,
            title: &format!("{} · {} · {SITE}", lesson.title, course.title),
            description: &lesson.excerpt,
            image: cover.as_ref().map(|cover| cover.src.as_str()),
            schema: Some(schema),
        },
        "lesson",
        &content,
    );
    Ok(html(&headers, page))
}

/// A course's chapters, each folding its lessons away but the current one's, which stands open
/// with `current` marked.
fn contents(course: &Course, lessons: &[Listed], numbers: &[usize], current: i32) -> String {
    let open = lessons
        .iter()
        .position(|lesson| lesson.number == current)
        .map(|at| numbers[at]);
    let mut html = String::new();
    let mut chapter = 0;
    for (lesson, number) in lessons.iter().zip(numbers) {
        if *number != chapter {
            if chapter > 0 {
                html.push_str("</ol></details>");
            }
            chapter = *number;
            let _ = write!(
                html,
                "<details{}><summary><span class=num>{number}</span>{}</summary><ol>",
                if Some(chapter) == open { " open" } else { "" },
                escape(&lesson.chapter)
            );
        }
        let _ = write!(
            html,
            "<li><a href=\"{}\"{}>{}</a></li>",
            courses::path(&course.id, Some(lesson.number)),
            if lesson.number == current {
                " aria-current=\"page\""
            } else {
                ""
            },
            named(lesson)
        );
    }
    if chapter > 0 {
        html.push_str("</ol></details>");
    }
    html
}

/// Under the lesson: the lessons before and after it.
fn steps(course: &Course, lessons: &[Listed], current: i32) -> String {
    let step = |lesson: Option<&Listed>, rel: &str, label: &str| {
        lesson
            .map(|lesson| {
                format!(
                    "<a rel={rel} href=\"{}\"><span class=step-label>{label}</span><span class=step-title>第 {} 课 {}</span></a>",
                    courses::path(&course.id, Some(lesson.number)),
                    lesson.number,
                    escape(&lesson.title)
                )
            })
            .unwrap_or_else(|| "<span></span>".into())
    };
    let previous = lessons.iter().rev().find(|other| other.number < current);
    let next = lessons.iter().find(|other| other.number > current);
    if previous.is_none() && next.is_none() {
        return String::new();
    }
    format!(
        "<nav class=steps aria-label=上一课与下一课>{}{}</nav>",
        step(previous, "prev", "← 上一课"),
        step(next, "next", "下一课 →")
    )
}

/// On the lesson's right: its sections, to jump to.
fn outline(sections: &[Section]) -> String {
    if sections.len() < 2 {
        return String::new();
    }
    let mut html = "<nav class=aside aria-label=本课><h2>本课</h2><ol>".to_owned();
    for section in sections {
        let _ = write!(
            html,
            "<li class=l{}><a href=\"#{}\">{}</a></li>",
            section.level,
            section.anchor,
            escape(&section.title)
        );
    }
    html.push_str("</ol></nav>");
    html
}

/// A headline, escaped. A Chinese headline is cut after its punctuation into phrases that wrap as
/// wholes (`.phrase`), so a line ends at "：" or "，" whenever the next phrase fits instead of
/// splitting a phrase; one without such punctuation stays as it is.
fn headline(title: &str) -> String {
    let pause = |c: char| matches!(c, '，' | '：' | '；' | '、' | '！' | '？' | '。');
    let mut phrases = Vec::new();
    let mut start = 0;
    let mut chars = title.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if pause(c) && !chars.peek().is_some_and(|&(_, next)| pause(next)) {
            phrases.push(&title[start..at + c.len_utf8()]);
            start = at + c.len_utf8();
        }
    }
    phrases.push(&title[start..]);
    phrases.retain(|phrase| !phrase.trim().is_empty());
    if phrases.len() < 2 {
        return escape(title);
    }
    phrases
        .iter()
        .map(|phrase| format!("<span class=phrase>{}</span>", escape(phrase)))
        .collect()
}

/// Every page a search engine should know, with when each lesson last changed.
async fn sitemap(State(state): State<AppState>, headers: HeaderMap) -> Result<Response> {
    let lessons: Vec<(String, i32, DateTime<Utc>)> = sqlx::query_as(
        "SELECT l.course, l.number, l.updated_at FROM lessons l
         JOIN courses c ON c.id = l.course AND c.visible
         ORDER BY c.place, l.number",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\"><url><loc>{}</loc></url>",
        escape(&absolute(&state, "/"))
    );
    for course in courses::shown(&state).await? {
        let _ = write!(
            xml,
            "<url><loc>{}</loc></url>",
            escape(&absolute(&state, &courses::path(&course.id, None)))
        );
    }
    for (course, number, updated) in lessons {
        let _ = write!(
            xml,
            "<url><loc>{}</loc><lastmod>{}</lastmod></url>",
            escape(&absolute(&state, &courses::path(&course, Some(number)))),
            updated.format("%Y-%m-%d")
        );
    }
    xml.push_str("</urlset>");
    Ok(conditional(&headers, xml, "application/xml; charset=utf-8"))
}

async fn robots(State(state): State<AppState>, headers: HeaderMap) -> Response {
    conditional(
        &headers,
        format!(
            "User-agent: *\nAllow: /\nSitemap: {}\n",
            absolute(&state, "/sitemap.xml")
        ),
        "text/plain; charset=utf-8",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn listed(number: i32, chapter: &str) -> Listed {
        let at = Utc.with_ymd_and_hms(2026, 10, 20, 8, 0, 0).unwrap();
        Listed {
            chapter: chapter.into(),
            number,
            title: format!("课{number}<b>"),
            published_at: at,
            updated_at: at,
        }
    }

    fn course() -> Course {
        Course {
            id: "esp32".into(),
            grp: "硬件".into(),
            title: "ESP32".into(),
            intro: String::new(),
            brief: String::new(),
            direction: String::new(),
            adjusted_at: None,
        }
    }

    #[test]
    fn the_contents_open_the_current_chapter_and_mark_the_lesson() {
        let lessons = [listed(1, "甲"), listed(2, "甲"), listed(4, "乙")];
        let numbers = courses::numbered(lessons.iter().map(|lesson| lesson.chapter.as_str()));
        let html = contents(&course(), &lessons, &numbers, 4);
        assert_eq!(html.matches("<details").count(), 2);
        assert_eq!(html.matches(" open>").count(), 1);
        assert!(html.contains(
            "<details open><summary><span class=num>2</span>乙</summary><ol><li><a href=\"/esp32/4\" aria-current=\"page\"><span class=num>4</span><span class=name>课4&lt;b&gt;</span></a></li></ol></details>"
        ));
        // A lesson not among them opens no chapter.
        assert!(!contents(&course(), &lessons, &numbers, 3).contains(" open>"));
    }

    #[test]
    fn steps_lead_to_the_lessons_around_and_skip_the_gaps() {
        let lessons = [listed(1, "甲"), listed(2, "甲"), listed(4, "乙")];
        let html = steps(&course(), &lessons, 2);
        assert!(html.contains("<a rel=prev href=\"/esp32/1\">"));
        assert!(html.contains("<a rel=next href=\"/esp32/4\">"));
        assert!(
            steps(&course(), &lessons, 1)
                .starts_with("<nav class=steps aria-label=上一课与下一课><span></span>")
        );
        assert_eq!(steps(&course(), &lessons[..1], 1), "");
    }

    #[test]
    fn headlines_wrap_after_their_punctuation_and_stay_escaped() {
        assert_eq!(
            headline("先装好：再<跑>，最后看"),
            "<span class=phrase>先装好：</span><span class=phrase>再&lt;跑&gt;，</span><span class=phrase>最后看</span>"
        );
        assert_eq!(headline("没有停顿 & 标点"), "没有停顿 &amp; 标点");
    }

    #[test]
    fn the_outline_needs_two_sections_and_indents_the_third_level() {
        let section = |anchor: &str, title: &str, level| Section {
            anchor: anchor.into(),
            title: title.into(),
            level,
        };
        assert_eq!(outline(&[section("s1", "一", 2)]), "");
        let html = outline(&[section("s1", "安装", 2), section("s2", "<坑>", 3)]);
        assert!(html.contains("<li class=l2><a href=\"#s1\">安装</a></li><li class=l3><a href=\"#s2\">&lt;坑&gt;</a></li>"));
    }

    #[test]
    fn a_page_is_served_whole_or_not_at_all_when_the_reader_holds_it() {
        let page = || "<p>页</p>".to_owned();
        let first = conditional(&HeaderMap::new(), page(), "text/html; charset=utf-8");
        assert_eq!(first.status(), StatusCode::OK);
        let etag = first.headers()[header::ETAG].clone();
        assert_eq!(
            first.headers()[header::CACHE_CONTROL],
            "public, max-age=60, stale-while-revalidate=300"
        );
        let mut held = HeaderMap::new();
        held.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&format!("W/{}", etag.to_str().unwrap())).unwrap(),
        );
        assert_eq!(
            conditional(&held, page(), "text/html; charset=utf-8").status(),
            StatusCode::NOT_MODIFIED
        );
    }
}
