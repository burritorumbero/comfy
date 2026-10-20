//! Every job this server queued, kept on disk, with live state from ComfyUI.
//!
//! The page and the app both list jobs from here, so every device sees the same history,
//! and finished jobs keep their images after ComfyUI's own history is wiped by a restart.
//! A tracker task follows ComfyUI (its websocket for instant updates, its queue and history
//! to confirm), so answering "how is my job doing" never waits on ComfyUI.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::comfy::{Comfy, ComfyError, ImageRef};
use crate::store;
use crate::workflow::{Graph, Kind};

/// How many jobs the history keeps; the oldest finished ones go first.
pub const HISTORY_LIMIT: usize = 2000;
/// Tracker checks before a job ComfyUI no longer knows counts as lost.
const MISSES_BEFORE_LOST: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Queued,
    Running,
    Done,
    Error,
    Cancelled,
    /// ComfyUI forgot the job before it ran, usually because it restarted.
    Lost,
}

impl State {
    pub fn active(self) -> bool {
        matches!(self, State::Queued | State::Running)
    }
}

/// Records saved by the first version held only finished jobs.
fn done() -> State {
    State::Done
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub prompt: String,
    /// Jobs saved before editing existed made new pictures.
    #[serde(default)]
    pub kind: Kind,
    /// The picture an edit started from, in ComfyUI's input folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<ImageRef>,
    /// The workflow's options as this job ran them.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, bool>,
    /// Unix milliseconds; 0 for jobs saved before history was kept.
    #[serde(default)]
    pub created: u64,
    #[serde(default = "done")]
    pub state: State,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Progress {
    pub value: u64,
    pub max: u64,
}

/// State that only matters while a job is active, never saved.
#[derive(Default)]
struct Live {
    ahead: Option<usize>,
    progress: Option<Progress>,
    misses: u8,
}

/// A job as the API returns it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: String,
    pub prompt: String,
    pub kind: Kind,
    /// Where the picture an edit started from is served.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, bool>,
    pub created: u64,
    pub state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Jobs ComfyUI runs before this one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImageView>,
}

#[derive(Debug, Serialize)]
pub struct ImageView {
    pub filename: String,
    pub url: String,
    pub preview: String,
    pub download: String,
}

fn view(record: &Record, live: Option<&Live>) -> JobView {
    let base = format!("/api/jobs/{}/images/", record.id);
    let active = record.state.active();
    JobView {
        id: record.id.clone(),
        prompt: record.prompt.clone(),
        kind: record.kind,
        source: record.source.as_ref().map(|_| format!("/api/jobs/{}/source", record.id)),
        options: record.options.clone(),
        created: record.created,
        state: record.state,
        seed: record.seed,
        seconds: record.seconds,
        error: record.error.clone(),
        ahead: live.filter(|_| record.state == State::Queued).and_then(|l| l.ahead),
        progress: live.filter(|_| active).and_then(|l| l.progress),
        images: record
            .images
            .iter()
            .enumerate()
            .map(|(n, image)| ImageView {
                filename: image.filename.clone(),
                url: format!("{base}{n}"),
                preview: format!("{base}{n}?preview=1"),
                download: format!("{base}{n}?download=1"),
            })
            .collect(),
    }
}

/// How a finished run ended, read from its ComfyUI history entry.
fn outcome(entry: &Value) -> (State, Option<String>, Vec<ImageRef>, Option<f64>) {
    let status = entry.get("status");
    let messages: Vec<(&str, &Value)> = status
        .and_then(|s| s.get("messages"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| Some((m.get(0)?.as_str()?, m.get(1)?)))
        .collect();

    let stamp = |events: &[&str]| {
        messages
            .iter()
            .rev()
            .find(|(e, _)| events.contains(e))
            .and_then(|(_, d)| d.get("timestamp")?.as_u64())
    };
    let seconds = match (
        stamp(&["execution_start"]),
        stamp(&["execution_success", "execution_error", "execution_interrupted"]),
    ) {
        (Some(start), Some(end)) if end >= start => Some(((end - start) as f64 / 100.0).round() / 10.0),
        _ => None,
    };

    for (event, data) in &messages {
        match *event {
            "execution_interrupted" => return (State::Cancelled, None, vec![], seconds),
            "execution_error" => {
                let node = data.get("node_type").and_then(Value::as_str).unwrap_or("a node");
                let message = data
                    .get("exception_message")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|m| !m.is_empty())
                    .or_else(|| data.get("exception_type").and_then(Value::as_str))
                    .unwrap_or("unknown error");
                return (State::Error, Some(format!("{node} failed: {message}")), vec![], seconds);
            }
            _ => {}
        }
    }
    if status.and_then(|s| s.get("status_str")).and_then(Value::as_str) == Some("error") {
        return (
            State::Error,
            Some("ComfyUI reported an error while running the workflow.".into()),
            vec![],
            seconds,
        );
    }

    let mut images: Vec<ImageRef> = entry
        .get("outputs")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|outputs| outputs.values())
        .filter_map(|output| output.get("images")?.as_array())
        .flatten()
        .filter_map(|image| serde_json::from_value::<ImageRef>(image.clone()).ok())
        .filter(|image| !image.filename.is_empty())
        .collect();
    // Save Image wins over Preview Image when the workflow has both.
    if images.iter().any(|i| i.kind == "output") {
        images.retain(|i| i.kind == "output");
    }
    if images.is_empty() {
        return (
            State::Error,
            Some("The workflow finished without saving an image. Does it end in a Save Image node?".into()),
            vec![],
            seconds,
        );
    }
    (State::Done, None, images, seconds)
}

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

pub fn random_u32() -> u32 {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).expect("randomness");
    u32::from_le_bytes(bytes)
}

#[derive(Default)]
struct Inner {
    records: IndexMap<String, Record>,
    live: HashMap<String, Live>,
    /// Bumped on every change worth saving.
    version: u64,
}

/// A run to queue: the workflow already filled in.
pub struct NewJob {
    pub prompt: String,
    pub kind: Kind,
    pub graph: Graph,
    pub seed: Option<u64>,
    pub source: Option<ImageRef>,
    pub options: BTreeMap<String, bool>,
}

pub struct Jobs {
    comfy: Comfy,
    client_id: String,
    path: Option<PathBuf>,
    inner: Mutex<Inner>,
    saved_version: Mutex<u64>,
    wake: Notify,
    comfy_ok: AtomicBool,
    feed_connected: AtomicBool,
    last_reconcile_ms: AtomicU64,
}

impl Jobs {
    /// The history at `path` (None keeps it in memory only).
    pub fn open(comfy: Comfy, client_id: String, path: Option<PathBuf>) -> Self {
        let mut records = IndexMap::new();
        if let Some(raw) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            match serde_json::from_str::<IndexMap<String, Value>>(&raw) {
                Ok(saved) => {
                    for (id, mut value) in saved {
                        if let Value::Object(fields) = &mut value {
                            fields.entry("id").or_insert_with(|| Value::String(id.clone()));
                        }
                        // One unreadable record is skipped; the rest still load.
                        if let Ok(record) = serde_json::from_value::<Record>(value) {
                            records.insert(id, record);
                        }
                    }
                }
                Err(e) => tracing::warn!("Ignoring unreadable job history {}: {e}", path.as_ref().unwrap().display()),
            }
        }
        Jobs {
            comfy,
            client_id,
            path,
            inner: Mutex::new(Inner {
                records,
                ..Default::default()
            }),
            saved_version: Mutex::new(0),
            wake: Notify::new(),
            comfy_ok: AtomicBool::new(false),
            feed_connected: AtomicBool::new(false),
            last_reconcile_ms: AtomicU64::new(0),
        }
    }

    pub fn comfy_ok(&self) -> bool {
        self.comfy_ok.load(Ordering::Relaxed)
    }

    pub fn feed_connected(&self) -> bool {
        self.feed_connected.load(Ordering::Relaxed)
    }

    pub fn set_comfy_ok(&self, ok: bool) {
        self.comfy_ok.store(ok, Ordering::Relaxed);
    }

    /// Write the history if it changed since the last write. The file is written outside
    /// the job lock, and only by whoever holds the newest version.
    fn persist(&self) {
        let Some(path) = &self.path else { return };
        let (bytes, version) = {
            let inner = self.inner.lock().unwrap();
            (serde_json::to_vec(&inner.records).expect("serializable"), inner.version)
        };
        let mut saved = self.saved_version.lock().unwrap();
        if version <= *saved {
            return;
        }
        match store::save_bytes(path, &bytes, false) {
            Ok(()) => *saved = version,
            Err(e) => tracing::warn!("Could not save the job history to {} ({e}).", path.display()),
        }
    }

    pub async fn submit(&self, job: NewJob) -> Result<JobView, ComfyError> {
        let id = self.comfy.queue_prompt(&Value::Object(job.graph), &self.client_id).await?;
        let record = Record {
            id: id.clone(),
            prompt: job.prompt,
            kind: job.kind,
            source: job.source,
            options: job.options,
            created: now_ms(),
            state: State::Queued,
            seed: job.seed,
            seconds: None,
            error: None,
            images: vec![],
        };
        let job = {
            let mut inner = self.inner.lock().unwrap();
            let job = view(&record, None);
            inner.records.insert(id, record);
            while inner.records.len() > HISTORY_LIMIT {
                let Some(oldest) = inner.records.values().position(|r| !r.state.active()) else {
                    break;
                };
                if let Some((gone, _)) = inner.records.shift_remove_index(oldest) {
                    inner.live.remove(&gone);
                }
            }
            inner.version += 1;
            job
        };
        self.persist();
        self.wake.notify_one();
        Ok(job)
    }

    pub fn get(&self, id: &str) -> Option<JobView> {
        let inner = self.inner.lock().unwrap();
        inner.records.get(id).map(|r| view(r, inner.live.get(id)))
    }

    /// Newest first: up to `limit` jobs older than `before` (or the newest), and whether more remain.
    pub fn list(&self, limit: usize, before: Option<&str>) -> (Vec<JobView>, bool) {
        let inner = self.inner.lock().unwrap();
        let mut newest_first = inner.records.values().rev().peekable();
        if let Some(before) = before {
            for record in newest_first.by_ref() {
                if record.id == before {
                    break;
                }
            }
        }
        let jobs: Vec<JobView> = newest_first.by_ref().take(limit).map(|r| view(r, inner.live.get(&r.id))).collect();
        let more = newest_first.peek().is_some();
        (jobs, more)
    }

    /// The picture an edit job started from.
    pub fn source(&self, id: &str) -> Option<ImageRef> {
        self.inner.lock().unwrap().records.get(id)?.source.clone()
    }

    /// The `index`th image of a finished job.
    pub fn image(&self, id: &str, index: usize) -> Option<ImageRef> {
        let inner = self.inner.lock().unwrap();
        inner.records.get(id).filter(|r| r.state == State::Done)?.images.get(index).cloned()
    }

    fn update(&self, id: &str, change: impl FnOnce(&mut Record, &mut Live)) {
        let mut inner = self.inner.lock().unwrap();
        let Inner { records, live, version } = &mut *inner;
        let Some(record) = records.get_mut(id) else { return };
        let before = (record.state, record.error.clone(), record.images.len());
        let live_entry = live.entry(id.to_string()).or_default();
        change(record, live_entry);
        if !record.state.active() {
            live.remove(id);
        }
        if before != (record.state, record.error.clone(), record.images.len()) {
            *version += 1;
        }
    }

    fn finish(&self, id: &str, entry: &Value) {
        let (state, error, images, seconds) = outcome(entry);
        self.update(id, |record, _| {
            record.state = state;
            record.error = error;
            record.images = images;
            record.seconds = seconds;
        });
    }

    /// Stop a job: take it out of the queue, or interrupt it if it is running.
    pub async fn cancel(&self, id: &str) -> Result<Option<JobView>, ComfyError> {
        let Some(job) = self.get(id) else { return Ok(None) };
        if !job.state.active() {
            return Ok(Some(job));
        }
        self.comfy.delete_from_queue(id).await?;
        let queue = self.comfy.queue().await?;
        if queue.running.iter().any(|r| r == id) {
            // The tracker sees ComfyUI's interruption in its history and marks the job.
            self.comfy.interrupt(id).await?;
        } else if let Some(entry) = self.comfy.history(id).await? {
            self.finish(id, &entry);
        } else {
            // Taken out of the queue before it ran: ComfyUI keeps no trace of it.
            self.update(id, |record, _| record.state = State::Cancelled);
        }
        self.persist();
        self.wake.notify_one();
        Ok(self.get(id))
    }

    /// Remove a job from the history, stopping it first if it is active. The image files stay
    /// in ComfyUI's output folder.
    pub async fn remove(&self, id: &str) -> bool {
        if self.get(id).is_some_and(|j| j.state.active()) {
            if let Err(e) = self.cancel(id).await {
                tracing::warn!("Removed {id} from the history, but could not stop it in ComfyUI: {e}");
            }
        }
        let removed = {
            let mut inner = self.inner.lock().unwrap();
            inner.live.remove(id);
            let removed = inner.records.shift_remove(id).is_some();
            if removed {
                inner.version += 1;
            }
            removed
        };
        self.persist();
        removed
    }

    fn has_active(&self) -> bool {
        self.inner.lock().unwrap().records.values().any(|r| r.state.active())
    }

    /// The jobs that are queued or running.
    pub fn active_ids(&self) -> Vec<String> {
        let inner = self.inner.lock().unwrap();
        inner.records.values().filter(|r| r.state.active()).map(|r| r.id.clone()).collect()
    }

    /// Bring active jobs up to date from ComfyUI's queue and history.
    pub async fn reconcile(&self) {
        self.last_reconcile_ms.store(now_ms(), Ordering::Relaxed);
        let active = self.active_ids();
        let queue = match self.comfy.queue().await {
            Ok(queue) => {
                self.set_comfy_ok(true);
                queue
            }
            Err(_) => {
                self.set_comfy_ok(false);
                return;
            }
        };
        let mut gone = Vec::new();
        for id in &active {
            if queue.running.iter().any(|r| r == id) {
                self.update(id, |record, live| {
                    record.state = State::Running;
                    live.ahead = None;
                    live.misses = 0;
                });
            } else if let Some(position) = queue.pending.iter().position(|p| p == id) {
                let ahead = queue.running.len() + position;
                self.update(id, |record, live| {
                    record.state = State::Queued;
                    live.ahead = Some(ahead);
                    live.misses = 0;
                });
            } else {
                gone.push(id);
            }
        }
        for id in gone {
            match self.comfy.history(id).await {
                Ok(Some(entry)) => self.finish(id, &entry),
                Ok(None) => self.update(id, |record, live| {
                    live.misses += 1;
                    if live.misses >= MISSES_BEFORE_LOST {
                        record.state = State::Lost;
                        record.error = Some("ComfyUI no longer has this job; it probably restarted before the job ran.".into());
                    }
                }),
                Err(_) => {
                    self.set_comfy_ok(false);
                    break;
                }
            }
        }
        self.persist();
    }

    /// Keep active jobs current: every second while any are active, every ten otherwise,
    /// and right away when ComfyUI's websocket says something changed.
    pub async fn track(self: Arc<Self>) {
        loop {
            self.reconcile().await;
            let wait = if self.has_active() {
                Duration::from_secs(1)
            } else {
                Duration::from_secs(10)
            };
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = self.wake.notified() => {
                    // Let a burst of events settle into one check.
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }

    /// Follow ComfyUI's websocket for progress and instant updates, reconnecting as needed.
    pub async fn follow(self: Arc<Self>) {
        let mut delay = Duration::from_secs(1);
        loop {
            let jobs = self.clone();
            let result = self.comfy.follow(&self.client_id, move |message| jobs.on_event(&message)).await;
            if self.feed_connected.swap(false, Ordering::Relaxed) {
                delay = Duration::from_secs(1);
                tracing::info!(
                    "Lost ComfyUI's progress feed ({}); reconnecting.",
                    result.err().unwrap_or_else(|| "closed".into())
                );
            }
            tokio::time::sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(30));
        }
    }

    pub fn on_event(&self, message: &Value) {
        let kind = message.get("type").and_then(Value::as_str).unwrap_or("");
        let data = message.get("data");
        let prompt_id = data.and_then(|d| d.get("prompt_id")).and_then(Value::as_str);
        match kind {
            "feed_connected" => {
                self.feed_connected.store(true, Ordering::Relaxed);
                tracing::info!("Following ComfyUI's progress over its websocket.");
            }
            "progress" => {
                let (Some(id), Some(value), Some(max)) = (
                    prompt_id,
                    data.and_then(|d| d["value"].as_u64()),
                    data.and_then(|d| d["max"].as_u64()),
                ) else {
                    return;
                };
                self.update(id, |record, live| {
                    if record.state.active() {
                        record.state = State::Running;
                        live.progress = Some(Progress { value, max });
                    }
                });
            }
            "execution_start" => {
                if let Some(id) = prompt_id {
                    self.update(id, |record, live| {
                        if record.state.active() {
                            record.state = State::Running;
                            live.ahead = None;
                        }
                    });
                }
            }
            // The queue moved or a run ended: check now rather than at the next tick.
            "status" | "executing" | "execution_success" | "execution_error" | "execution_interrupted"
                if self.has_active() && now_ms().saturating_sub(self.last_reconcile_ms.load(Ordering::Relaxed)) > 50 =>
            {
                self.wake.notify_one();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn outcome_of_a_successful_run() {
        let entry = json!({
            "outputs": {
                "60": {"images": [{"filename": "Qwen-Image-2512_00001_.png", "subfolder": "", "type": "output"}]},
                "61": {"images": [{"filename": "preview_00001_.png", "subfolder": "", "type": "temp"}]}
            },
            "status": {"status_str": "success", "completed": true, "messages": [
                ["execution_start", {"timestamp": 1000}], ["execution_success", {"timestamp": 42500}]
            ]}
        });
        let (state, error, images, seconds) = outcome(&entry);
        assert_eq!((state, error, seconds), (State::Done, None, Some(41.5)));
        assert_eq!(
            images.iter().map(|i| i.filename.as_str()).collect::<Vec<_>>(),
            ["Qwen-Image-2512_00001_.png"]
        );
    }

    #[test]
    fn outcome_of_failed_and_interrupted_runs() {
        let failed = json!({"outputs": {}, "status": {"status_str": "error", "messages": [
            ["execution_error", {"node_type": "UNETLoader", "exception_message": " Out of memory "}]
        ]}});
        assert_eq!(outcome(&failed).1.as_deref(), Some("UNETLoader failed: Out of memory"));
        let interrupted = json!({"outputs": {}, "status": {"status_str": "error", "messages": [["execution_interrupted", {}]]}});
        assert_eq!(outcome(&interrupted).0, State::Cancelled);
        let nothing_saved = json!({"outputs": {}, "status": {"status_str": "success", "messages": []}});
        assert!(outcome(&nothing_saved).1.unwrap().contains("Save Image"));
    }

    #[test]
    fn history_saved_by_the_first_version_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jobs.json");
        std::fs::write(
            &path,
            r#"{"3b42c898-cc25-4f9d-961c-f2d8a160623c": {"seed": 7, "seconds": 3.0,
            "images": [{"filename": "a.png", "subfolder": "", "type": "output"}]}, "broken": 5}"#,
        )
        .unwrap();
        let jobs = Jobs::open(Comfy::new("http://127.0.0.1:9"), "c".into(), Some(path));
        let job = jobs.get("3b42c898-cc25-4f9d-961c-f2d8a160623c").unwrap();
        assert_eq!((job.state, job.seed, job.images.len()), (State::Done, Some(7), 1));
        assert_eq!((job.kind, job.source), (Kind::Generate, None));
        assert!(jobs.get("broken").is_none());
    }
}
