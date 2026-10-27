//! What the site's keeper runs on the server, in place of an admin page:
//! `learn open <idea>`, `learn turn <course> <change>`, `learn hide|show <course>`,
//! `learn write` and `learn courses`.
use crate::{
    courses,
    error::{AppError, Result},
    state::AppState,
    writer::{self, design},
};
use chrono::{DateTime, Utc};
use std::fmt::Write;

pub const USAGE: &str = "用法：
  learn                       运行网站，并按时写课
  learn open <一句话>          照这句话设计一门新课
  learn turn <课程> <一句话>    照这句话调整一门课的方向
  learn hide <课程>            隐藏一门课：不再列出，也不再写
  learn show <课程>            恢复一门隐藏的课
  learn write                 马上写一课：先修订到期的，否则写下一课
  learn courses               列出全部课程
  learn configcheck           只检查配置";

/// Runs one command and says what it did.
pub async fn run(state: &AppState, command: &str, words: &[String]) -> Result<String> {
    let sentence = || {
        let text = words.join(" ").trim().to_owned();
        if (4..=200).contains(&text.chars().count()) {
            Ok(text)
        } else {
            Err(AppError::Conflict("用 4–200 个字说".into()))
        }
    };
    let course = || {
        words
            .first()
            .cloned()
            .ok_or_else(|| AppError::Conflict("要写明是哪门课".into()))
    };
    match command {
        "open" => {
            let id = design::open(state, &sentence()?, "request").await?;
            Ok(format!("开了新课 /{id}，XABC 接下来会先写它的第一课"))
        }
        "turn" => {
            let id = course()?;
            let change = words[1..].join(" ").trim().to_owned();
            if !(4..=200).contains(&change.chars().count()) {
                return Err(AppError::Conflict("用 4–200 个字说怎么调整".into()));
            }
            let rewrite = design::turn(state, &id, &change).await?;
            Ok(if rewrite {
                format!("调整了 /{id}；已写的课会逐课按新说明修订")
            } else {
                format!("调整了 /{id}；以后的课按新说明写")
            })
        }
        "hide" | "show" => {
            let id = course()?;
            if !courses::set_visible(state, &id, command == "show").await? {
                return Err(AppError::NotFound("没有这门课".into()));
            }
            Ok(format!(
                "/{id} {}",
                if command == "show" {
                    "已恢复"
                } else {
                    "已隐藏"
                }
            ))
        }
        "write" => Ok(writer::step(state, Utc::now()).await?.to_string()),
        "courses" => listed(state).await,
        _ => Err(AppError::Conflict(
            format!("不认识的命令：{command}\n{USAGE}").into(),
        )),
    }
}

/// A course as the list shows it.
#[derive(sqlx::FromRow)]
struct Row {
    id: String,
    grp: String,
    title: String,
    visible: bool,
    source: String,
    lessons: i64,
    updated: Option<DateTime<Utc>>,
}

/// Every course, hidden ones too: its address, lessons, last change, and how it came.
async fn listed(state: &AppState) -> Result<String> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT c.id, c.grp, c.title, c.visible, c.source, count(l.id) AS lessons,
                max(l.updated_at) AS updated
         FROM courses c LEFT JOIN lessons l ON l.course = c.id
         GROUP BY c.id ORDER BY c.place",
    )
    .fetch_all(&state.pool)
    .await?;
    let mut text = String::new();
    for row in rows {
        let _ = writeln!(
            text,
            "{:<20} {} · {} · {} 课{}{} · {}",
            row.id,
            row.grp,
            row.title,
            row.lessons,
            row.updated
                .map(|at| format!(
                    " · 更新 {}",
                    at.with_timezone(&state.config.timezone)
                        .format("%m-%d %H:%M")
                ))
                .unwrap_or_default(),
            if row.visible { "" } else { " · 已隐藏" },
            row.source,
        );
    }
    Ok(text.trim_end().to_owned())
}
