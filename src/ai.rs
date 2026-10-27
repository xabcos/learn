//! The AI gateway: the Responses API, which plans, searches and writes, and the image model.
use crate::{
    error::{AppError, Result},
    state::AppState,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const MAX_ERROR: usize = 300;
/// The image model's answer may weigh this much: a drawn 1536×1024 PNG is about 3 MB, and base64
/// adds a third.
const DRAWN_BYTES: usize = 16 * 1024 * 1024;

/// Sends one request to the Responses API. A busy gateway (429, 5xx) or a dropped connection is
/// tried twice more; any other refusal returns at once. Callers bound the whole with their own
/// deadline.
pub(crate) async fn request(state: &AppState, body: Value) -> Result<Value> {
    let url = endpoint(state, "/v1/responses");
    let mut delays = [10, 30].into_iter();
    loop {
        let failure = match state
            .http
            .post(&url)
            .bearer_auth(&state.config.ai_api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return Ok(response.json().await?),
            Ok(response)
                if response.status().as_u16() == 429 || response.status().is_server_error() =>
            {
                response.status().to_string()
            }
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                let message: String = text.trim().chars().take(MAX_ERROR).collect();
                return Err(AppError::Internal(format!("{status}: {message}").into()));
            }
            Err(error) => error.to_string(),
        };
        let Some(seconds) = delays.next() else {
            return Err(AppError::Internal(failure.into()));
        };
        tracing::debug!(%failure, seconds, model = body["model"].as_str(), "retrying AI request");
        tokio::time::sleep(Duration::from_secs(seconds)).await;
    }
}

fn endpoint(state: &AppState, path: &str) -> String {
    format!(
        "{}{path}",
        state.config.ai_base_url.as_str().trim_end_matches('/')
    )
}

/// A strict JSON Schema object: every property is required and no other is allowed.
pub(crate) fn object(properties: Value) -> Value {
    let required: Vec<_> = properties
        .as_object()
        .expect("schema properties")
        .keys()
        .cloned()
        .collect();
    json!({"type": "object", "additionalProperties": false, "required": required, "properties": properties})
}

/// The JSON object the reply ends with.
pub(crate) fn parse_payload(reply: &Value) -> Result<Value> {
    if let Some(error) = reply.get("error").filter(|error| !error.is_null()) {
        let message: String = error.to_string().chars().take(MAX_ERROR).collect();
        return Err(AppError::Internal(format!("AI 接口错误: {message}").into()));
    }
    if reply["status"] != "completed" {
        return Err(AppError::Internal(
            format!("AI 响应未完成: {}", reply["status"]).into(),
        ));
    }
    let text = reply["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|output| output["type"] == "message")
        .filter_map(|output| output["content"].as_array())
        .flatten()
        // Search replies can hold a draft before the final message.
        .rev()
        .find(|content| content["type"] == "output_text")
        .and_then(|content| content["text"].as_str())
        .ok_or_else(|| AppError::Internal("AI 响应没有文本输出".into()))?;
    // Some providers wrap the JSON object in prose, even in strict mode.
    let json = match (text.find('{'), text.rfind('}')) {
        (Some(start), Some(end)) if start < end => &text[start..=end],
        _ => text,
    };
    serde_json::from_str(json).map_err(|_| AppError::Internal("AI 响应不是有效的 JSON".into()))
}

/// Asks with a strict schema and reads the answer as `T`.
pub(crate) async fn ask<T: for<'de> Deserialize<'de>>(
    state: &AppState,
    body: Value,
    what: &str,
) -> Result<(T, Value)> {
    let reply = request(state, body).await?;
    let answer = serde_json::from_value(parse_payload(&reply)?)
        .map_err(|_| AppError::Internal(format!("{what}不符合约定的结构").into()))?;
    Ok((answer, reply))
}

/// The pages a reply cites from its searches.
pub(crate) fn cited(reply: &Value) -> Vec<String> {
    reply["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|output| output["content"].as_array())
        .flatten()
        .filter_map(|content| content["annotations"].as_array())
        .flatten()
        .filter(|annotation| annotation["type"] == "url_citation")
        .filter_map(|annotation| annotation["url"].as_str().map(str::to_owned))
        .collect()
}

/// What a reply cost, for the log.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct Usage {
    #[serde(default)]
    pub(crate) input_tokens: i64,
    #[serde(default)]
    pub(crate) output_tokens: i64,
    #[serde(default)]
    pub(crate) server_side_tool_usage_details: Tools,
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Tools {
    #[serde(default)]
    pub(crate) web_search_calls: i64,
}

impl Usage {
    /// A missing or malformed block counts as nothing; it only feeds the log.
    pub(crate) fn of(reply: &Value) -> Self {
        Self::deserialize(&reply["usage"]).unwrap_or_default()
    }
}

impl std::ops::AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.server_side_tool_usage_details.web_search_calls +=
            other.server_side_tool_usage_details.web_search_calls;
    }
}

/// A picture drawn to `prompt`, as the PNG the image model answers with.
pub(crate) async fn draw(state: &AppState, prompt: &str) -> Result<Vec<u8>> {
    let mut response = state
        .http
        .post(endpoint(state, "/v1/images/generations"))
        .bearer_auth(&state.config.ai_api_key)
        .json(&json!({
            "model": state.config.ai_image_model,
            "prompt": format!("{}\n画面中不要出现任何文字、标志或水印。", prompt.trim()),
            "n": 1,
            "size": "1536x1024",
            "quality": "medium",
        }))
        .send()
        .await?
        .error_for_status()?;
    let mut reply = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if reply.len() + chunk.len() > DRAWN_BYTES {
            return Err(AppError::Internal("图像模型的回答过大".into()));
        }
        reply.extend_from_slice(&chunk);
    }
    let reply: Value = serde_json::from_slice(&reply)?;
    let encoded = reply["data"][0]["b64_json"]
        .as_str()
        .ok_or_else(|| AppError::Internal("图像模型没有返回图片".into()))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| AppError::Internal("图像模型返回的图片编码无效".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_final_answer_after_search_drafts() {
        let message =
            |text: &str| json!({"type":"message","content":[{"type":"output_text","text":text}]});
        let mut reply = json!({"status":"completed","output":[
            message(r#"{"title":"draft"}"#),
            {"type":"web_search_call"},
            message(r#"Here it is: {"title":"final"}"#)
        ]});
        assert_eq!(parse_payload(&reply).unwrap()["title"], "final");
        reply["output"][2] = message("An unfinished answer");
        assert!(parse_payload(&reply).is_err());
        reply["status"] = json!("incomplete");
        assert!(parse_payload(&reply).is_err());
    }

    #[test]
    fn cited_pages_come_from_the_url_citations() {
        let reply = json!({"output":[{"type":"message","content":[{"type":"output_text","text":"x",
            "annotations":[{"type":"url_citation","url":"https://docs.example.com/a"},
                           {"type":"file_citation","url":"https://other.example.com"}]}]}]});
        assert_eq!(cited(&reply), ["https://docs.example.com/a"]);
    }
}
