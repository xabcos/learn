//! The whole site against a disposable database, with the AI gateway and tmpfile played by a
//! local server: a course opened with a sentence, XABC's lessons written and revised with drawn
//! pictures, the pages, a turn and hiding a course.
use crate::{commands, config::Config, state::AppState, web, writer};
use axum::{Json, Router, body::Bytes, extract::State, http::StatusCode, routing::post};
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// What the mock gateway was asked, by schema name, in order.
type Asked = Arc<Mutex<Vec<(String, Value)>>>;

#[derive(Clone)]
struct Mock {
    asked: Asked,
    base: String,
    /// The next upload fails.
    refuse: Arc<AtomicBool>,
    /// While the next new lesson is written, another one goes into its course.
    interject: Arc<AtomicBool>,
    pool: sqlx::PgPool,
}

fn reply(text: Value, cited: &[&str]) -> Json<Value> {
    let annotations: Vec<Value> = cited
        .iter()
        .map(|url| json!({ "type": "url_citation", "url": url }))
        .collect();
    Json(json!({
        "status": "completed",
        "output": [{ "type": "message", "content": [{
            "type": "output_text", "text": text.to_string(), "annotations": annotations,
        }] }],
        "usage": { "input_tokens": 10, "output_tokens": 5 },
    }))
}

/// A lesson body long enough to publish, with its pictures and links.
fn lesson_body(extra: &str) -> String {
    format!(
        "学完这一课，你能做出一个能跑的东西。\n\n## 概念\n\n{}\n\n参考 [官方指南](https://docs.example.com/guide) 和 [编造的页面](https://made-up.example/x)。\n\n{extra}\n\n## 动手\n\n```sh\necho hi\n```",
        "这是讲解的正文，用来凑够一课的长度。".repeat(30)
    )
}

async fn responses(State(mock): State<Mock>, Json(body): Json<Value>) -> Json<Value> {
    let name = body["text"]["format"]["name"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    mock.asked
        .lock()
        .unwrap()
        .push((name.clone(), body.clone()));
    let input = body["input"][0]["content"].as_str().unwrap_or_default();
    if name == "lesson"
        && !input.contains("这一课的现有全文")
        && mock.interject.swap(false, Ordering::SeqCst)
    {
        sqlx::query(
            // Its time is its transaction's start, before this run began: only its number tells.
            "INSERT INTO lessons (id, course, number, chapter, title, body, excerpt, review_at,
                                  published_at, updated_at)
             SELECT gen_random_uuid(), 'rust-web', max(number) + 1, '起步', '别处写成的一课', '正文',
                    '正文', now() + interval '30 days', now() - interval '1 hour',
                    now() - interval '1 hour'
             FROM lessons WHERE course = 'rust-web'",
        )
        .execute(&mock.pool)
        .await
        .unwrap();
    }
    match name.as_str() {
        "course" => reply(
            json!({ "id": "rust-web", "group": "实战项目", "title": "Rust 网站实战",
                    "intro": "用 axum 从零做一个网站", "brief": "读者会一点 Rust。章节：路由、模板、数据库。" }),
            &[],
        ),
        "discovery" => reply(json!({ "idea": "", "sources": [] }), &[]),
        "plan" => reply(
            json!({ "chapter": "起步", "title": if input.contains("要修订的是") { "修订后的第一课" } else { "第一个路由" },
                    "goal": "跑起一个页面", "outline": ["安装", "路由"], "review_days": 7 }),
            &[],
        ),
        "finds" => reply(
            json!({ "finds": [{ "title": "官方指南", "why": "权威", "points": "要点",
                                "sources": ["https://docs.example.com/guide"] }] }),
            &["https://docs.example.com/guide"],
        ),
        "lesson" if input.contains("这一课的现有全文") => reply(
            json!({ "title": "修订后的第一课",
                    "body": lesson_body("![](keep:1)\n\n新的一段。\n\n![](img:1)"),
                    "pictures": [{ "prompt": "新的示意图", "caption": "新图" }], "cover": "" }),
            &[],
        ),
        "lesson" => reply(
            json!({ "title": "第一个路由", "body": lesson_body("![](img:1)\n\n![](img:2)"),
                    "pictures": [{ "prompt": "路由示意", "caption": "请求怎么走" },
                                 { "prompt": "", "caption": "没有提示词的图" }],
                    "cover": "封面插图" }),
            &[],
        ),
        "turn" => reply(
            json!({ "title": "Rust 网站实战", "intro": "用 axum 和 sqlx 做网站",
                    "brief": "改用 sqlx。", "rewrite": true }),
            &[],
        ),
        other => panic!("unexpected request {other}"),
    }
}

async fn images(State(mock): State<Mock>) -> Json<Value> {
    mock.asked
        .lock()
        .unwrap()
        .push(("image".into(), Value::Null));
    let picture = image::RgbImage::from_pixel(1536, 1024, image::Rgb([40, 90, 160]));
    let mut png = Vec::new();
    picture
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    use base64::Engine;
    Json(json!({ "data": [{ "b64_json": base64::engine::general_purpose::STANDARD.encode(png) }] }))
}

async fn upload(State(mock): State<Mock>, body: Bytes) -> Result<Json<Value>, StatusCode> {
    assert!(body.len() > 1000, "the upload carries the picture");
    if mock.refuse.swap(false, Ordering::SeqCst) {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let count = {
        let mut asked = mock.asked.lock().unwrap();
        asked.push(("upload".into(), Value::Null));
        asked.iter().filter(|(name, _)| name == "upload").count()
    };
    Ok(Json(json!({
        "direct_link": format!("{}/files/{count}.jpg", mock.base),
        "expires_at": (Utc::now() + chrono::Duration::days(365)).to_rfc3339(),
    })))
}

fn times(asked: &Asked, name: &str) -> usize {
    asked
        .lock()
        .unwrap()
        .iter()
        .filter(|(asked, _)| asked == name)
        .count()
}

#[tokio::test]
#[ignore = "requires LEARN_TEST_DATABASE_URL, a disposable database"]
async fn postgres_courses_are_opened_written_revised_turned_and_hidden() {
    let database_url = std::env::var("LEARN_TEST_DATABASE_URL").expect("disposable database URL");
    let pool = sqlx::PgPool::connect(&database_url).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(name.starts_with("learn_test"), "{name}");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let asked: Asked = Arc::default();
    let refuse = Arc::new(AtomicBool::new(false));
    let interject = Arc::new(AtomicBool::new(false));
    let mock = Router::new()
        .route("/v1/responses", post(responses))
        .route("/v1/images/generations", post(images))
        .route("/upload", post(upload))
        .with_state(Mock {
            asked: asked.clone(),
            base: base.clone(),
            refuse: refuse.clone(),
            interject: interject.clone(),
            pool,
        });
    let gateway = tokio::spawn(async move { axum::serve(listener, mock).await.unwrap() });
    let config = Config {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        database_url,
        database_max_connections: 4,
        public_base_url: "https://learn.example.com".parse().unwrap(),
        timezone: chrono_tz::Asia::Shanghai,
        writer: false,
        ai_base_url: base.parse().unwrap(),
        ai_api_key: "test".into(),
        ai_model: "test-model".into(),
        ai_image_model: "test-image".into(),
        ai_timeout: std::time::Duration::from_secs(30),
        tmpfile_base_url: base.parse().unwrap(),
        tmpfile_api_key: "test".into(),
    };
    let state = AppState::new(config).await.unwrap();

    // A course opened with a sentence stands at once, and XABC writes it next.
    let said = commands::run(&state, "open", &["一门讲".into(), "Rust 网站的课".into()])
        .await
        .unwrap();
    assert!(said.contains("/rust-web"), "{said}");
    let source: String = sqlx::query_scalar("SELECT source FROM courses WHERE id = 'rust-web'")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(source, "request");

    let done = writer::step(&state, Utc::now()).await.unwrap().to_string();
    assert!(done.contains("新课 /rust-web/1"), "{done}");
    // The week's look for a course ran once, before the lesson.
    assert_eq!(times(&asked, "discovery"), 1);
    let (body, cover, review): (String, Option<uuid::Uuid>, chrono::DateTime<Utc>) =
        sqlx::query_as(
            "SELECT body, cover, review_at FROM lessons WHERE course = 'rust-web' AND number = 1",
        )
        .fetch_one(&state.pool)
        .await
        .unwrap();
    // The placed picture was drawn with its caption, the one without a prompt left out; the link
    // the search found stays and the made-up one is reduced to its words.
    assert_eq!(body.matches("](image:").count(), 1, "{body}");
    assert!(body.contains("*请求怎么走（AI 生成）*"));
    assert!(!body.contains("img:"));
    assert!(body.contains("[官方指南](https://docs.example.com/guide)"));
    assert!(body.contains("编造的页面") && !body.contains("made-up.example"));
    assert!(cover.is_some());
    assert_eq!(times(&asked, "image"), 2);
    assert_eq!(times(&asked, "upload"), 2);
    // The planner's 7 days are held to the 30-day floor.
    assert!(review > Utc::now() + chrono::Duration::days(29));
    let pictures: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(pictures, 2);

    // The next lesson follows in the same course. A picture tmpfile refuses is left out; the
    // lesson goes out with the other.
    refuse.store(true, Ordering::SeqCst);
    let done = writer::step(&state, Utc::now()).await.unwrap().to_string();
    assert!(done.contains("新课 /rust-web/2"), "{done}");
    let pictures: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(pictures, 3);
    // Another lesson within the window is refused when it comes to be published.
    let refused = writer::step(&state, Utc::now() - chrono::Duration::hours(1)).await;
    assert!(
        matches!(&refused, Err(crate::error::AppError::Conflict(message)) if message == "刚写过一课")
    );
    let lessons: i64 = sqlx::query_scalar("SELECT count(*) FROM lessons")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(lessons, 2);
    // A lesson another process wrote meanwhile takes this one's place, so this one, planned
    // without it, is not published.
    interject.store(true, Ordering::SeqCst);
    let raced = writer::step(&state, Utc::now()).await;
    assert!(matches!(
        &raced,
        Err(crate::error::AppError::Conflict(message)) if message == "这门课刚有了新的一课"
    ));
    let numbers: Vec<i32> = sqlx::query_scalar("SELECT number FROM lessons ORDER BY number")
        .fetch_all(&state.pool)
        .await
        .unwrap();
    assert_eq!(numbers, [1, 2, 3]);
    sqlx::query("DELETE FROM lessons WHERE number = 3")
        .execute(&state.pool)
        .await
        .unwrap();

    // The pages.
    let site = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", site.local_addr().unwrap());
    let app = web::router(state.clone());
    let server = tokio::spawn(async move { axum::serve(site, app).await.unwrap() });
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let get = |path: &str| http.get(format!("{address}{path}")).send();
    let home = get("/").await.unwrap();
    assert_eq!(home.status(), StatusCode::OK);
    assert!(
        home.headers()["cache-control"]
            .to_str()
            .unwrap()
            .starts_with("public")
    );
    let home = home.text().await.unwrap();
    assert!(home.contains("Rust 网站实战") && home.contains("2 课"));
    let course = get("/rust-web").await.unwrap().text().await.unwrap();
    assert!(course.contains("第 1 章") && course.contains("第一个路由"));
    let lesson = get("/rust-web/1").await.unwrap().text().await.unwrap();
    assert!(lesson.contains(&format!("{base}/files/")));
    assert!(lesson.contains("<h2 id=\"s1\">概念</h2>"));
    assert!(lesson.contains("aria-current=\"page\""));
    assert!(lesson.contains("<a rel=next href=\"/rust-web/2\">"));
    assert!(lesson.contains("\"@type\":\"Article\""));
    for missing in [
        "/rust-web/3",
        "/rust-web/x",
        "/nope",
        "/nope/1",
        "/assets/nope.css",
    ] {
        assert_eq!(
            get(missing).await.unwrap().status(),
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }
    let styles = get("/assets/styles.css").await.unwrap();
    assert_eq!(styles.status(), StatusCode::OK);
    assert!(
        styles.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("default-src 'self'")
    );
    let sitemap = get("/sitemap.xml").await.unwrap().text().await.unwrap();
    assert!(sitemap.contains("https://learn.example.com/rust-web/1</loc>"));
    assert!(
        get("/robots.txt")
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
            .contains("Sitemap: https://learn.example.com/sitemap.xml")
    );

    // A lesson that comes due is revised: it keeps its figure, gains a new one, keeps its cover,
    // and the discovery doesn't run again within the week.
    sqlx::query("UPDATE lessons SET review_at = now() - interval '1 minute'")
        .execute(&state.pool)
        .await
        .unwrap();
    let done = writer::step(&state, Utc::now()).await.unwrap().to_string();
    assert!(done.starts_with("修订 /rust-web/1"), "{done}");
    assert_eq!(times(&asked, "discovery"), 1);
    let (revised, revision, kept_cover): (String, i32, Option<uuid::Uuid>) =
        sqlx::query_as("SELECT body, revision, cover FROM lessons WHERE number = 1")
            .fetch_one(&state.pool)
            .await
            .unwrap();
    assert_eq!(revision, 2);
    assert_eq!(kept_cover, cover);
    assert_eq!(revised.matches("](image:").count(), 2, "{revised}");
    assert!(revised.contains("*请求怎么走（AI 生成）*") && revised.contains("*新图（AI 生成）*"));
    let pictures: i64 = sqlx::query_scalar("SELECT count(*) FROM images")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(pictures, 4);

    // Turning the course with a sentence brings every lesson due again.
    let said = commands::run(
        &state,
        "turn",
        &["rust-web".into(), "数据库改用".into(), "sqlx".into()],
    )
    .await
    .unwrap();
    assert!(said.contains("逐课"), "{said}");
    let due: i64 = sqlx::query_scalar("SELECT count(*) FROM lessons WHERE review_at <= now()")
        .fetch_one(&state.pool)
        .await
        .unwrap();
    assert_eq!(due, 2);

    // A hidden course is neither shown nor written.
    commands::run(&state, "hide", &["rust-web".into()])
        .await
        .unwrap();
    assert_eq!(
        get("/rust-web").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get("/rust-web/1").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert!(
        !get("/")
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
            .contains("Rust 网站实战")
    );
    assert!(writer::step(&state, Utc::now()).await.is_err());
    let listed = commands::run(&state, "courses", &[]).await.unwrap();
    assert!(
        listed.contains("rust-web") && listed.contains("已隐藏"),
        "{listed}"
    );
    assert!(
        commands::run(&state, "show", &["nope".into()])
            .await
            .is_err()
    );

    server.abort();
    gateway.abort();
}
