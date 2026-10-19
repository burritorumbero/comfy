//! Talking to ComfyUI: queueing, the queue and history, images, and its websocket.

use std::fmt;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

/// ComfyUI portable and manual installs listen on 8188, ComfyUI Desktop on 8000.
pub const CANDIDATES: &[&str] = &["http://127.0.0.1:8188", "http://127.0.0.1:8000"];

#[derive(Debug)]
pub enum ComfyError {
    /// ComfyUI did not answer at all.
    Unavailable(String),
    /// ComfyUI answered, but refused or failed the request.
    Rejected(String),
}

impl fmt::Display for ComfyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ComfyError::Unavailable(m) | ComfyError::Rejected(m) => f.write_str(m),
        }
    }
}

/// One saved image, as ComfyUI's history names it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    pub filename: String,
    #[serde(default)]
    pub subfolder: String,
    #[serde(rename = "type", default = "output")]
    pub kind: String,
}

fn output() -> String {
    "output".into()
}

impl ImageRef {
    /// How a Load Image node names it: `subfolder/filename`, with ` [output]` for a result.
    pub fn load_image_name(&self) -> String {
        let path = if self.subfolder.is_empty() {
            self.filename.clone()
        } else {
            format!("{}/{}", self.subfolder, self.filename)
        };
        if self.kind == "input" {
            path
        } else {
            format!("{path} [{}]", self.kind)
        }
    }
}

/// The ids ComfyUI is running and, in queue order, the ones waiting.
pub struct QueueSnapshot {
    pub running: Vec<String>,
    pub pending: Vec<String>,
}

/// Turn ComfyUI's validation error body into one readable message.
pub fn format_error(body: &Value) -> String {
    let mut parts = Vec::new();
    match body.get("error") {
        Some(Value::Object(e)) => parts.push(
            e.get("message")
                .or_else(|| e.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("ComfyUI rejected the workflow")
                .to_string(),
        ),
        Some(Value::String(e)) if !e.is_empty() => parts.push(e.clone()),
        _ => {}
    }
    if let Some(Value::Object(nodes)) = body.get("node_errors") {
        for (id, info) in nodes {
            let class = info.get("class_type").and_then(Value::as_str).unwrap_or("node");
            for err in info.get("errors").and_then(Value::as_array).into_iter().flatten() {
                let message = err
                    .get("message")
                    .or_else(|| err.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("error");
                match err.get("details").and_then(Value::as_str).filter(|d| !d.is_empty()) {
                    Some(details) => parts.push(format!("{class} {id}: {message} ({details})")),
                    None => parts.push(format!("{class} {id}: {message}")),
                }
            }
        }
    }
    if parts.is_empty() {
        "ComfyUI rejected the workflow".into()
    } else {
        parts.join("; ")
    }
}

#[derive(Clone)]
pub struct Comfy {
    pub base: String,
    http: reqwest::Client,
}

impl Comfy {
    pub fn new(base: &str) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy() // ComfyUI is local; never send its traffic through a system proxy
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(30))
            .build()
            .expect("HTTP client");
        Comfy {
            base: base.trim_end_matches('/').to_string(),
            http,
        }
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<reqwest::Response, ComfyError> {
        let response = request
            .send()
            .await
            .map_err(|e| ComfyError::Unavailable(format!("ComfyUI is not reachable at {} ({}).", self.base, root_cause(&e))))?;
        if response.status().is_success() {
            return Ok(response);
        }
        let status = response.status().as_u16();
        let raw = response.text().await.unwrap_or_default();
        let detail = match serde_json::from_str::<Value>(&raw) {
            Ok(body) => format_error(&body),
            Err(_) => raw.trim().chars().take(500).collect(),
        };
        Err(ComfyError::Rejected(format!("ComfyUI returned {status}: {detail}")))
    }

    async fn get_json(&self, path: &str) -> Result<Value, ComfyError> {
        let response = self.send(self.http.get(format!("{}{path}", self.base))).await?;
        response
            .json()
            .await
            .map_err(|e| ComfyError::Rejected(format!("ComfyUI sent something unexpected for {path}: {e}")))
    }

    async fn post_json(&self, path: &str, body: &Value) -> Result<Value, ComfyError> {
        let response = self.send(self.http.post(format!("{}{path}", self.base)).json(body)).await?;
        Ok(response.json().await.unwrap_or(Value::Null))
    }

    pub async fn reachable(&self) -> bool {
        let request = self.http.get(format!("{}/system_stats", self.base)).timeout(Duration::from_secs(2));
        matches!(request.send().await, Ok(r) if r.status().is_success())
    }

    /// Queue a workflow; returns ComfyUI's prompt id.
    pub async fn queue_prompt(&self, graph: &Value, client_id: &str) -> Result<String, ComfyError> {
        let reply = self.post_json("/prompt", &json!({"prompt": graph, "client_id": client_id})).await?;
        match reply.get("prompt_id").and_then(Value::as_str) {
            Some(id) => Ok(id.to_string()),
            None => Err(ComfyError::Rejected(format!(
                "ComfyUI did not queue the prompt: {}",
                format_error(&reply)
            ))),
        }
    }

    pub async fn queue(&self) -> Result<QueueSnapshot, ComfyError> {
        let queue = self.get_json("/queue").await?;
        let ids = |key: &str| -> Vec<(f64, String)> {
            queue
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| Some((item.get(0)?.as_f64()?, item.get(1)?.as_str()?.to_string())))
                .collect()
        };
        let running = ids("queue_running").into_iter().map(|(_, id)| id).collect();
        // The pending list is a heap, not in order; its first field is the queue number.
        let mut pending = ids("queue_pending");
        pending.sort_by(|a, b| a.0.total_cmp(&b.0));
        Ok(QueueSnapshot {
            running,
            pending: pending.into_iter().map(|(_, id)| id).collect(),
        })
    }

    /// The history entry of a finished run, or None while it has not finished.
    pub async fn history(&self, prompt_id: &str) -> Result<Option<Value>, ComfyError> {
        let mut history = self.get_json(&format!("/history/{prompt_id}")).await?;
        Ok(history.get_mut(prompt_id).map(Value::take))
    }

    pub async fn image(&self, image: &ImageRef, preview: bool) -> Result<reqwest::Response, ComfyError> {
        let mut query = vec![
            ("filename", image.filename.as_str()),
            ("subfolder", image.subfolder.as_str()),
            ("type", image.kind.as_str()),
        ];
        if preview {
            query.push(("preview", "webp;85"));
        }
        self.send(
            self.http
                .get(format!("{}/view", self.base))
                .query(&query)
                .timeout(Duration::from_secs(120)),
        )
        .await
    }

    /// Put a picture in ComfyUI's input folder, under `subfolder`, for a Load Image node.
    /// Returns where ComfyUI saved it, which may differ from `name` if that was taken.
    pub async fn upload_image(&self, bytes: Vec<u8>, subfolder: &str, name: &str, mime: &str) -> Result<ImageRef, ComfyError> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(name.to_string())
            .mime_str(mime)
            .map_err(|e| ComfyError::Rejected(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .part("image", part)
            .text("type", "input")
            .text("subfolder", subfolder.to_string())
            .text("overwrite", "false");
        let response = self
            .send(
                self.http
                    .post(format!("{}/upload/image", self.base))
                    .multipart(form)
                    .timeout(Duration::from_secs(120)),
            )
            .await?;
        let saved: Value = response
            .json()
            .await
            .map_err(|e| ComfyError::Rejected(format!("ComfyUI sent something unexpected after an upload: {e}")))?;
        let name = saved.get("name").and_then(Value::as_str).unwrap_or_default();
        if name.is_empty() {
            return Err(ComfyError::Rejected("ComfyUI didn't say where it saved the picture.".into()));
        }
        Ok(ImageRef {
            filename: name.to_string(),
            subfolder: saved.get("subfolder").and_then(Value::as_str).unwrap_or(subfolder).to_string(),
            kind: "input".into(),
        })
    }

    pub async fn delete_from_queue(&self, prompt_id: &str) -> Result<(), ComfyError> {
        self.post_json("/queue", &json!({"delete": [prompt_id]})).await.map(drop)
    }

    /// Unload every model and drop the node cache, like ComfyUI's "Unload Models and Execution Cache".
    /// ComfyUI does it between jobs, so the memory is released a moment after this returns.
    pub async fn free(&self) -> Result<(), ComfyError> {
        self.post_json("/free", &json!({"unload_models": true, "free_memory": true}))
            .await
            .map(drop)
    }

    /// Older ComfyUI ignores prompt_id and stops whatever runs, so only call this for a job that is running.
    pub async fn interrupt(&self, prompt_id: &str) -> Result<(), ComfyError> {
        self.post_json("/interrupt", &json!({"prompt_id": prompt_id})).await.map(drop)
    }

    /// Follow ComfyUI's websocket, calling `on_message` for every JSON message, until it closes.
    /// ComfyUI sends progress only to the client id that queued the prompt.
    pub async fn follow(&self, client_id: &str, mut on_message: impl FnMut(Value)) -> Result<(), String> {
        let url = format!("{}/ws?clientId={client_id}", self.base.replacen("http", "ws", 1));
        let (mut socket, _) = tokio_tungstenite::connect_async(url.as_str()).await.map_err(|e| e.to_string())?;
        on_message(json!({"type": "feed_connected"}));
        while let Some(message) = socket.next().await {
            match message.map_err(|e| e.to_string())? {
                Message::Text(text) => {
                    if let Ok(value) = serde_json::from_str::<Value>(&text) {
                        on_message(value);
                    }
                }
                Message::Close(_) => break,
                // Binary messages are live previews; pings are answered by the library.
                _ => {}
            }
        }
        Ok(())
    }
}

fn root_cause(error: &(dyn std::error::Error + 'static)) -> String {
    let mut cause = error;
    while let Some(source) = cause.source() {
        cause = source;
    }
    cause.to_string()
}

/// The first of `candidates` that answers.
pub async fn detect(candidates: &[&str]) -> Option<String> {
    for url in candidates {
        if Comfy::new(url).reachable().await {
            return Some(url.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_errors_name_the_node() {
        let body = json!({
            "error": {"type": "prompt_outputs_failed_validation", "message": "Prompt outputs failed validation"},
            "node_errors": {"238:226": {"class_type": "UNETLoader", "errors": [
                {"message": "Value not in list", "details": "unet_name: 'x.safetensors' not in []"}
            ]}}
        });
        assert_eq!(
            format_error(&body),
            "Prompt outputs failed validation; UNETLoader 238:226: Value not in list (unet_name: 'x.safetensors' not in [])"
        );
    }
}
