//! Talking to MLX-Serve, the LLM server on this Mac: which models it holds in memory, and
//! unloading or loading them. This server never sends it a prompt.
//!
//! MLX-Serve exempts requests from this computer from its API key, so none is needed here.

use std::time::Duration;

use serde_json::{Value, json};

/// MLX-Serve's own address.
pub const DEFAULT_URL: &str = "http://127.0.0.1:11234";

/// The models MLX-Serve holds in memory.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Loaded {
    pub ids: Vec<String>,
    pub bytes: u64,
}

#[derive(Debug, PartialEq)]
pub enum LoadError {
    /// MLX-Serve refused because the model doesn't fit in free memory yet.
    NoRoom(String),
    Failed(String),
}

#[derive(Clone)]
pub struct Llm {
    pub base: String,
    http: reqwest::Client,
}

fn reason(status: reqwest::StatusCode, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body).ok().and_then(|v| {
        let error = v.get("error")?;
        error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| error.as_str())
            .map(str::to_string)
    });
    message.unwrap_or_else(|| format!("MLX-Serve returned {status}: {}", body.trim().chars().take(300).collect::<String>()))
}

/// The ids MLX-Serve lists in `GET /v1/models` as loaded (or loading), and their resident bytes.
pub fn loaded_from(models: &Value) -> Loaded {
    let mut loaded = Loaded::default();
    for model in models["data"].as_array().into_iter().flatten() {
        let holding = model["loaded"].as_bool() == Some(true) || model["state"].as_str() == Some("loading");
        let id = model["id"].as_str().or_else(|| model["alias"].as_str());
        if let (true, Some(id)) = (holding, id) {
            loaded.ids.push(id.to_string());
            loaded.bytes += model["bytes_resident"].as_u64().unwrap_or(0);
        }
    }
    loaded
}

impl Llm {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy() // MLX-Serve is local
            .connect_timeout(Duration::from_secs(3))
            .build()
            .expect("HTTP client");
        Llm {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    pub async fn answers(&self) -> bool {
        let request = self.http.get(format!("{}/health", self.base)).timeout(Duration::from_secs(2));
        matches!(request.send().await, Ok(r) if r.status().is_success())
    }

    pub async fn loaded(&self) -> Result<Loaded, String> {
        let response = self
            .http
            .get(format!("{}/v1/models", self.base))
            .timeout(Duration::from_secs(4))
            .send()
            .await
            .map_err(|_| format!("MLX-Serve isn't answering at {}.", self.base))?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(reason(status, &body));
        }
        let models = serde_json::from_str(&body).map_err(|_| "MLX-Serve sent an unreadable model list.".to_string())?;
        Ok(loaded_from(&models))
    }

    pub async fn unload(&self, id: &str) -> Result<(), String> {
        let response = self
            .http
            .post(format!("{}/v1/unload-model", self.base))
            .json(&json!({"model": id}))
            .timeout(Duration::from_secs(120))
            .send()
            .await
            .map_err(|_| format!("MLX-Serve stopped answering while unloading {id}."))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(reason(status, &response.text().await.unwrap_or_default()))
    }

    /// Load a model into memory. A large model from disk takes a while, so this waits up to 20 minutes.
    pub async fn load(&self, id: &str) -> Result<(), LoadError> {
        let response = self
            .http
            .post(format!("{}/v1/load-model", self.base))
            .json(&json!({"model": id}))
            .timeout(Duration::from_secs(20 * 60))
            .send()
            .await
            .map_err(|_| LoadError::Failed(format!("MLX-Serve stopped answering while loading {id}.")))?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        let message = reason(status, &body);
        let lower = format!("{body} {message}").to_lowercase();
        let no_room = status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            || ["out_of_memory", "refusing to load", "not enough memory"]
                .iter()
                .any(|s| lower.contains(s));
        Err(if no_room {
            LoadError::NoRoom(message)
        } else {
            LoadError::Failed(message)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loaded_models_count() {
        let models = json!({"object": "list", "data": [
            {"id": "ddalcu/Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit", "loaded": true, "state": "ready", "bytes_resident": 107_000_000_000u64},
            {"id": "Qwen3.8-27B-MLX-8bit", "loaded": false, "state": "unloaded", "bytes_resident": 0},
            {"alias": "gemma", "loaded": false, "state": "loading", "bytes_resident": 2_000_000_000u64},
        ]});
        assert_eq!(
            loaded_from(&models),
            Loaded {
                ids: vec!["ddalcu/Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit".into(), "gemma".into()],
                bytes: 109_000_000_000,
            }
        );
        assert_eq!(loaded_from(&json!({"data": []})), Loaded::default());
    }

    #[test]
    fn errors_read_like_mlx_serve_wrote_them() {
        let body = r#"{"error": {"message": "Refusing to load x: needs ~100 GB", "type": "out_of_memory"}}"#;
        assert_eq!(
            reason(reqwest::StatusCode::SERVICE_UNAVAILABLE, body),
            "Refusing to load x: needs ~100 GB"
        );
        assert_eq!(
            reason(reqwest::StatusCode::BAD_REQUEST, "nope"),
            "MLX-Serve returned 400 Bad Request: nope"
        );
    }
}
