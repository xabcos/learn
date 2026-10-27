use crate::error::{AppError, Result};
use std::{env, net::SocketAddr, path::PathBuf, time::Duration};
use url::Url;

/// How the site runs, from the environment. Serving pages needs only the database; writing needs
/// the AI gateway and tmpfile as well.
#[derive(Clone, Debug)]
pub struct Config {
    pub(crate) bind_addr: SocketAddr,
    pub(crate) database_url: String,
    pub(crate) database_max_connections: u32,
    pub(crate) migrations_dir: PathBuf,
    pub(crate) pages_dir: PathBuf,
    pub(crate) public_base_url: Url,
    pub(crate) timezone: chrono_tz::Tz,
    /// Whether XABC writes on its schedule; the commands write whenever they are run.
    pub(crate) writer: bool,
    pub(crate) ai_base_url: Url,
    pub(crate) ai_api_key: String,
    /// The model that plans, searches and writes.
    pub(crate) ai_model: String,
    /// The model that draws the pictures.
    pub(crate) ai_image_model: String,
    /// The longest one AI request may take.
    pub(crate) ai_timeout: Duration,
    pub(crate) tmpfile_base_url: Url,
    pub(crate) tmpfile_api_key: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let public_base_url = url("PUBLIC_BASE_URL", "https://learn.xabcnews.com")?;
        let timezone = env_value("LEARN_TIMEZONE", "Asia/Shanghai");
        Ok(Self {
            bind_addr: parse_env("BIND_ADDR", SocketAddr::from(([0, 0, 0, 0], 8788)))?,
            database_url: required_env("DATABASE_URL")?,
            database_max_connections: parse_env("DATABASE_MAX_CONNECTIONS", 10)?,
            migrations_dir: env_value("MIGRATIONS_DIR", "migrations").into(),
            pages_dir: env_value("PAGES_DIR", "pages").into(),
            public_base_url,
            timezone: timezone
                .parse()
                .map_err(|_| AppError::Config(format!("未知的时区 {timezone}").into()))?,
            writer: bool_env("LEARN_WRITER", true)?,
            ai_base_url: url("AI_BASE_URL", "https://api.zairouter.com")?,
            ai_api_key: env_value("AI_API_KEY", ""),
            ai_model: env_value("AI_MODEL", "gpt-6-astra"),
            ai_image_model: env_value("AI_IMAGE_MODEL", "gpt-image-2.5-flare"),
            ai_timeout: Duration::from_secs(parse_env("AI_TIMEOUT_SECS", 1800)?),
            tmpfile_base_url: url("TMPFILE_BASE_URL", "https://dl.xabcnews.com")?,
            tmpfile_api_key: env_value("TMPFILE_API_KEY", ""),
        })
    }
}

fn required_env(name: &str) -> Result<String> {
    optional_env(name).ok_or_else(|| AppError::Config(format!("缺少环境变量 {name}").into()))
}

fn optional_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_value(name: &str, default: &str) -> String {
    optional_env(name).unwrap_or_else(|| default.to_owned())
}

fn parse_env<T: std::str::FromStr>(name: &str, default: T) -> Result<T> {
    match optional_env(name) {
        Some(value) => value
            .parse()
            .map_err(|_| AppError::Config(format!("环境变量 {name} 的值无效").into())),
        None => Ok(default),
    }
}

fn bool_env(name: &str, default: bool) -> Result<bool> {
    match optional_env(name).as_deref() {
        None => Ok(default),
        Some("1" | "true" | "yes" | "on") => Ok(true),
        Some("0" | "false" | "no" | "off") => Ok(false),
        Some(_) => Err(AppError::Config(
            format!("环境变量 {name} 只能是 true 或 false").into(),
        )),
    }
}

/// An http(s) address with a host.
fn url(name: &str, default: &str) -> Result<Url> {
    let value = env_value(name, default);
    Url::parse(&value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        .ok_or_else(|| AppError::Config(format!("环境变量 {name} 不是有效的地址").into()))
}
