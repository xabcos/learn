//! Courses and their lessons as the pages and XABC read them.
use crate::{error::Result, state::AppState};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

/// A course: its address, its group and name, a line on what it teaches, and the brief XABC
/// writes it to; how it was last turned, and when.
#[derive(Clone, Debug, FromRow)]
pub(crate) struct Course {
    pub(crate) id: String,
    pub(crate) grp: String,
    pub(crate) title: String,
    pub(crate) intro: String,
    pub(crate) brief: String,
    pub(crate) direction: String,
    pub(crate) adjusted_at: Option<DateTime<Utc>>,
}

/// A query for courses, ending as given.
macro_rules! course {
    ($rest:literal) => {
        concat!(
            "SELECT id, grp, title, intro, brief, direction, adjusted_at FROM courses",
            $rest
        )
    };
}

/// The courses shown: group by group in the order the groups came, each group's courses in the
/// order they came.
pub(crate) async fn shown(state: &AppState) -> Result<Vec<Course>> {
    Ok(sqlx::query_as(course!(
        " WHERE visible ORDER BY min(place) OVER (PARTITION BY grp), place"
    ))
    .fetch_all(&state.pool)
    .await?)
}

/// A course shown.
pub(crate) async fn find(state: &AppState, id: &str) -> Result<Option<Course>> {
    Ok(sqlx::query_as(course!(" WHERE id = $1 AND visible"))
        .bind(id)
        .fetch_optional(&state.pool)
        .await?)
}

/// A lesson as contents list it.
#[derive(Clone, Debug, FromRow)]
pub(crate) struct Listed {
    pub(crate) chapter: String,
    pub(crate) number: i32,
    pub(crate) title: String,
    pub(crate) published_at: DateTime<Utc>,
    pub(crate) updated_at: DateTime<Utc>,
}

/// A course's lessons in order.
pub(crate) async fn lessons(state: &AppState, course: &str) -> Result<Vec<Listed>> {
    Ok(sqlx::query_as(
        "SELECT chapter, number, title, published_at, updated_at
         FROM lessons WHERE course = $1 ORDER BY number",
    )
    .bind(course)
    .fetch_all(&state.pool)
    .await?)
}

/// Each lesson's chapter number: a chapter starts where a lesson names another than the one
/// before it.
pub(crate) fn numbered<'a>(chapters: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    let mut last = None;
    let mut number = 0;
    chapters
        .into_iter()
        .map(|chapter| {
            if last != Some(chapter) {
                number += 1;
                last = Some(chapter);
            }
            number
        })
        .collect()
}

/// The courses by group, in the order `shown` gives them.
pub(crate) fn groups(courses: &[Course]) -> Vec<(&str, Vec<&Course>)> {
    let mut groups: Vec<(&str, Vec<&Course>)> = Vec::new();
    for course in courses {
        match groups.last_mut() {
            Some((group, members)) if *group == course.grp => members.push(course),
            _ => groups.push((&course.grp, vec![course])),
        }
    }
    groups
}

/// The address of a course, or of one of its lessons.
pub(crate) fn path(course: &str, number: Option<i32>) -> String {
    match number {
        Some(number) => format!("/{course}/{number}"),
        None => format!("/{course}"),
    }
}

/// Shows or hides a course: a hidden one is neither listed nor extended, and its lessons stay.
pub(crate) async fn set_visible(state: &AppState, id: &str, visible: bool) -> Result<bool> {
    Ok(sqlx::query("UPDATE courses SET visible = $2 WHERE id = $1")
        .bind(id)
        .bind(visible)
        .execute(&state.pool)
        .await?
        .rows_affected()
        > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapters_are_numbered_where_their_names_change() {
        assert_eq!(
            numbered(["选型", "选型", "工程化", "工程化", "工程化", "外设"]),
            [1, 1, 2, 2, 2, 3]
        );
        // A name coming back later starts a chapter again, so the numbers keep the order.
        assert_eq!(numbered(["甲", "乙", "甲"]), [1, 2, 3]);
        assert!(numbered([]).is_empty());
    }

    #[test]
    fn courses_group_as_they_come() {
        let course = |id: &str, grp: &str| Course {
            id: id.into(),
            grp: grp.into(),
            title: id.into(),
            intro: String::new(),
            brief: String::new(),
            direction: String::new(),
            adjusted_at: None,
        };
        let courses = [course("a", "甲"), course("b", "甲"), course("c", "乙")];
        let grouped: Vec<(&str, Vec<&str>)> = groups(&courses)
            .into_iter()
            .map(|(group, members)| (group, members.iter().map(|c| c.id.as_str()).collect()))
            .collect();
        assert_eq!(grouped, [("甲", vec!["a", "b"]), ("乙", vec!["c"])]);
        assert_eq!(path("esp32", Some(12)), "/esp32/12");
        assert_eq!(path("esp32", None), "/esp32");
    }
}
