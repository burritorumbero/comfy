//! Switching this Mac between three modes that each hold one heavy model: image generation and
//! image editing (two ComfyUI workflows with different models) and the LLM (MLX-Serve, which
//! Hermes uses). They don't fit in memory together, so a switch unloads one mode's models before
//! the next mode may load its own, and only once the next side answers, so a switch that can't
//! finish leaves things as they were. It opens an app that isn't running, but never quits or kills one.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::comfy::Comfy;
use crate::jobs::{Jobs, now_ms};
use crate::llm::{Llm, LoadError};
use crate::store;
use crate::workflow::Kind;

pub const DEFAULT_IMAGE_APP: &str = "/Applications/Comfy Desktop.app";
pub const DEFAULT_LLM_APP: &str = "/Applications/MLX-Serve.app";

/// What the Mac's memory is set aside for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Generate,
    Edit,
    Llm,
}

impl Mode {
    pub fn of(kind: Kind) -> Mode {
        match kind {
            Kind::Generate => Mode::Generate,
            Kind::Edit => Mode::Edit,
        }
    }

    /// The image workflow this mode runs, if any.
    pub fn kind(self) -> Option<Kind> {
        match self {
            Mode::Generate => Some(Kind::Generate),
            Mode::Edit => Some(Kind::Edit),
            Mode::Llm => None,
        }
    }

    pub fn noun(self) -> &'static str {
        match self {
            Mode::Generate => "image generation",
            Mode::Edit => "image editing",
            Mode::Llm => "the LLM",
        }
    }

    /// The app's name for it.
    pub fn label(self) -> &'static str {
        match self {
            Mode::Generate => "Generate",
            Mode::Edit => "Edit",
            Mode::Llm => "LLM",
        }
    }
}

pub type OpenApp = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;
pub type IsOpen = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// Opens apps, and tells whether one is open. Tests pass fakes.
#[derive(Clone)]
pub struct Apps {
    pub open: OpenApp,
    pub is_open: IsOpen,
}

impl Apps {
    /// `open -g -a <app>`, which starts an app without bringing it to the front, and the process list.
    pub fn system() -> Self {
        Apps {
            open: Arc::new(|app: &Path| {
                let status = Command::new("/usr/bin/open")
                    .args(["-g", "-a"])
                    .arg(app)
                    .status()
                    .map_err(|e| format!("Could not open {} ({e}).", app.display()))?;
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("Could not open {}.", app.display()))
                }
            }),
            is_open: Arc::new(app_is_open),
        }
    }
}

/// An app is open when launchd runs a program from inside its bundle (helpers and servers it
/// starts have the app as their parent).
fn app_is_open(app: &Path) -> bool {
    let inside = format!("{}/", app.join("Contents/MacOS").display());
    let Ok(ps) = Command::new("ps").args(["-axo", "ppid=,comm="]).output() else {
        return false;
    };
    String::from_utf8_lossy(&ps.stdout).lines().any(|line| {
        let (ppid, program) = line.trim_start().split_once(' ').unwrap_or_default();
        ppid == "1" && program.trim_start().starts_with(&inside)
    })
}

/// How long each step may wait. Tests shorten them.
#[derive(Clone, Debug)]
pub struct Waits {
    pub poll: Duration,
    /// Comfy Desktop sets up Python and its custom nodes before ComfyUI answers.
    pub comfy_start: Duration,
    pub llm_start: Duration,
    pub unload: Duration,
    /// How long to keep retrying a load MLX-Serve refuses for lack of free memory.
    pub room: Duration,
    /// How long ComfyUI may take to release its memory.
    pub free: Duration,
    /// How often the watcher looks for the LLM loaded behind a switch's back.
    pub watch: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Waits {
            poll: Duration::from_secs(1),
            comfy_start: Duration::from_secs(180),
            llm_start: Duration::from_secs(60),
            unload: Duration::from_secs(60),
            room: Duration::from_secs(120),
            free: Duration::from_secs(30),
            watch: Duration::from_secs(5),
        }
    }
}

pub struct Config {
    pub image_app: Option<PathBuf>,
    pub llm_app: Option<PathBuf>,
    /// Loaded when switching to the LLM if no model was unloaded before.
    pub llm_model: Option<String>,
    /// Where the unloaded LLM models are remembered (None: in memory only).
    pub path: Option<PathBuf>,
    /// An edit workflow is set up, so the edit mode is offered.
    pub edit: bool,
    /// ComfyUI holds a model when it uses more memory than this (it idles at about 4 GB).
    pub busy_bytes: u64,
    pub apps: Apps,
    pub waits: Waits,
}

pub const DEFAULT_BUSY_BYTES: u64 = 10_000_000_000;

#[derive(Clone, Debug, Serialize)]
pub struct Switching {
    pub to: Mode,
    pub step: String,
    /// Unix milliseconds.
    pub since: u64,
}

#[derive(Debug, Serialize)]
pub struct SideView {
    pub name: &'static str,
    /// The app that runs it, opened when needed.
    pub app: Option<String>,
    /// It answers right now.
    pub running: bool,
    /// Its app is open. MLX-Serve can be open with its server stopped.
    #[serde(rename = "appOpen")]
    pub app_open: bool,
    /// Bytes in memory, when known.
    pub memory: Option<u64>,
    /// The LLM models held in memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct View {
    /// What the memory is set aside for now: the LLM while it holds a model, otherwise the image
    /// mode ComfyUI is set up for. `switching.to` says where a running switch is headed.
    pub mode: Mode,
    /// The modes this server offers.
    pub modes: Vec<Mode>,
    pub switching: Option<Switching>,
    /// Why the last switch failed.
    pub error: Option<String>,
    /// Something the server did on its own, such as freeing ComfyUI for Hermes.
    pub notice: Option<String>,
    pub images: SideView,
    pub llm: SideView,
}

#[derive(Debug, PartialEq)]
pub enum Refusal {
    AlreadySwitching,
    /// Leaving the current image mode would stop this many queued or running jobs.
    ActiveJobs(usize),
    /// No edit workflow is set up.
    NoEdit,
}

#[derive(Default)]
struct State {
    switching: Option<Switching>,
    error: Option<String>,
    notice: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    /// The LLM models the last switch to images unloaded, loaded again on the way back.
    #[serde(default)]
    llm_models: Vec<String>,
    /// The image workflow ComfyUI was last set up for: only its models may be in memory.
    #[serde(default)]
    image_mode: Kind,
}

pub struct Services {
    comfy: Comfy,
    llm: Llm,
    jobs: Arc<Jobs>,
    config: Config,
    state: Mutex<State>,
    saved: Mutex<Saved>,
}

/// The last part of a model id, short enough for a status line.
pub fn short_name(id: &str) -> &str {
    id.rsplit('/').next().unwrap_or(id)
}

fn app_name(app: &Option<PathBuf>) -> Option<String> {
    app.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned())
}

/// Resident memory of the process listening on `base`'s port, if it is on this computer.
fn listener_memory(base: &str) -> Option<u64> {
    let url = reqwest::Url::parse(base).ok()?;
    let local = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "::1" | "[::1]"));
    let port = url.port_or_known_default()?;
    if !local {
        return None;
    }
    let lsof = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    let pid = String::from_utf8_lossy(&lsof.stdout).lines().next()?.trim().to_string();
    let ps = Command::new("ps").args(["-o", "rss=", "-p", &pid]).output().ok()?;
    let kilobytes: u64 = String::from_utf8_lossy(&ps.stdout).trim().parse().ok()?;
    Some(kilobytes * 1024)
}

async fn memory_of(base: &str) -> Option<u64> {
    let base = base.to_string();
    tokio::task::spawn_blocking(move || listener_memory(&base)).await.ok().flatten()
}

impl Services {
    pub fn new(comfy: Comfy, llm: Llm, jobs: Arc<Jobs>, config: Config) -> Self {
        let saved = config
            .path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Services {
            comfy,
            llm,
            jobs,
            config,
            state: Mutex::default(),
            saved: Mutex::new(saved),
        }
    }

    pub fn llm_url(&self) -> &str {
        &self.llm.base
    }

    /// Where a switch is headed, while one runs.
    pub fn switching(&self) -> Option<Mode> {
        self.state.lock().unwrap().switching.as_ref().map(|s| s.to)
    }

    /// The image workflow ComfyUI is set up for.
    pub fn image_mode(&self) -> Kind {
        self.saved.lock().unwrap().image_mode
    }

    pub fn modes(&self) -> Vec<Mode> {
        let mut modes = vec![Mode::Generate];
        if self.config.edit {
            modes.push(Mode::Edit);
        }
        modes.push(Mode::Llm);
        modes
    }

    /// Why a `kind` job can't go to ComfyUI right now, if it can't.
    pub async fn refusal(&self, kind: Kind) -> Option<String> {
        let wanted = Mode::of(kind);
        if let Some(to) = self.switching() {
            return Some(if to == wanted {
                format!("Your Mac is switching to {}. Send it when it's ready.", to.noun())
            } else {
                format!("Your Mac is switching to {}, so {} is paused.", to.noun(), kind.noun())
            });
        }
        if let Some(models) = self.llm_models().await {
            return Some(format!(
                "The LLM ({}) is loaded on your Mac, so {} is paused. Switch to {} first.",
                short_name(&models[0]),
                kind.noun(),
                wanted.label()
            ));
        }
        let mode = self.image_mode();
        (mode != kind).then(|| {
            format!(
                "Your Mac is set up for {}. Switch to {} first, so the two models are never in memory together.",
                mode.noun(),
                wanted.label()
            )
        })
    }

    /// The LLM models in memory, when MLX-Serve answers and holds any.
    pub async fn llm_models(&self) -> Option<Vec<String>> {
        self.llm.loaded().await.ok().map(|l| l.ids).filter(|ids| !ids.is_empty())
    }

    pub async fn view(&self) -> View {
        // Read before probing: a view that says no switch is running then shows where it ended.
        let (switching, error, notice) = {
            let state = self.state.lock().unwrap();
            (state.switching.clone(), state.error.clone(), state.notice.clone())
        };
        let (comfy_up, loaded, llm_up, image_app_open, llm_app_open) = tokio::join!(
            self.comfy.reachable(),
            self.llm.loaded(),
            self.llm.answers(),
            self.is_open(&self.config.image_app),
            self.is_open(&self.config.llm_app),
        );
        let (comfy_memory, llm_memory) = tokio::join!(async { if comfy_up { memory_of(&self.comfy.base).await } else { None } }, async {
            match &loaded {
                Ok(l) if l.bytes > 0 => Some(l.bytes),
                Ok(_) => memory_of(&self.llm.base).await,
                Err(_) => None,
            }
        });
        let mode = match &loaded {
            Ok(l) if !l.ids.is_empty() => Mode::Llm,
            _ => Mode::of(self.image_mode()),
        };
        View {
            mode,
            modes: self.modes(),
            switching,
            error,
            notice,
            images: SideView {
                name: "ComfyUI",
                app: app_name(&self.config.image_app),
                running: comfy_up,
                app_open: image_app_open,
                memory: comfy_memory,
                models: None,
            },
            llm: SideView {
                name: "MLX-Serve",
                app: app_name(&self.config.llm_app),
                running: loaded.is_ok() || llm_up,
                app_open: llm_app_open,
                memory: llm_memory,
                models: Some(loaded.map(|l| l.ids).unwrap_or_default()),
            },
        }
    }

    /// Start switching to `to` in the background. Leaving the current image mode stops queued and
    /// running image jobs, so it is refused while there are any unless `cancel_jobs` says to
    /// cancel them.
    pub fn begin(self: &Arc<Self>, to: Mode, cancel_jobs: bool) -> Result<(), Refusal> {
        if to == Mode::Edit && !self.config.edit {
            return Err(Refusal::NoEdit);
        }
        {
            let mut state = self.state.lock().unwrap();
            if state.switching.is_some() {
                return Err(Refusal::AlreadySwitching);
            }
            let active = self.jobs.active_ids().len();
            let leaving = to.kind() != Some(self.image_mode());
            if leaving && active > 0 && !cancel_jobs {
                return Err(Refusal::ActiveJobs(active));
            }
            state.error = None;
            state.notice = None;
            state.switching = Some(Switching {
                to,
                step: "Starting…".into(),
                since: now_ms(),
            });
        }
        let services = self.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let result = match to.kind() {
                Some(kind) => services.to_images(kind).await,
                None => services.to_llm().await,
            };
            let mut state = services.state.lock().unwrap();
            state.switching = None;
            match result {
                Ok(()) => tracing::info!("Switched to {to:?} in {:.0} s", started.elapsed().as_secs_f64()),
                Err(e) => {
                    tracing::warn!("Switching to {to:?} failed: {e}");
                    state.error = Some(e);
                }
            }
        });
        Ok(())
    }

    fn step(&self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("Switch: {text}");
        if let Some(switching) = &mut self.state.lock().unwrap().switching {
            switching.step = text;
        }
    }

    /// Poll `ready` until it is true or `limit` passes.
    async fn wait_for<F: Future<Output = bool>>(&self, limit: Duration, mut ready: impl FnMut() -> F) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if ready().await {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(self.config.waits.poll).await;
        }
    }

    fn remember(&self, ids: Vec<String>) {
        let mut saved = self.saved.lock().unwrap();
        saved.llm_models = ids;
        if let Some(path) = &self.config.path {
            if let Err(e) = store::save_json(path, &*saved, false) {
                tracing::warn!("Could not save {} ({e}).", path.display());
            }
        }
    }

    fn open(&self, app: &Option<PathBuf>, what: &str, flag: &str) -> Result<(), String> {
        let app = app
            .as_ref()
            .ok_or_else(|| format!("{what} isn't running, and no app is set to open it (--{flag})."))?;
        (self.config.apps.open)(app)
    }

    async fn is_open(&self, app: &Option<PathBuf>) -> bool {
        let Some(app) = app.clone() else { return false };
        let is_open = self.config.apps.is_open.clone();
        tokio::task::spawn_blocking(move || is_open(&app)).await.unwrap_or(false)
    }

    /// Make sure ComfyUI answers, opening Comfy Desktop if it is closed.
    async fn start_comfy(&self) -> Result<(), String> {
        if self.comfy.reachable().await {
            return Ok(());
        }
        let limit = self.config.waits.comfy_start;
        // Reopening an open app only brings up its window, so an open one is left to finish starting.
        let open = self.is_open(&self.config.image_app).await;
        if !open {
            self.step("Opening ComfyUI…");
            self.open(&self.config.image_app, "ComfyUI", "image-app")?;
        }
        self.step("Waiting for ComfyUI to start…");
        if self.wait_for(limit, || self.comfy.reachable()).await {
            return Ok(());
        }
        let app = app_name(&self.config.image_app).unwrap_or_else(|| "ComfyUI".into());
        Err(if open {
            format!(
                "{app} is open, but ComfyUI didn't answer within {} s. Check {app}'s window.",
                limit.as_secs()
            )
        } else {
            format!("ComfyUI didn't answer within {} s of opening it.", limit.as_secs())
        })
    }

    /// Make sure MLX-Serve's server answers. It starts when MLX-Serve opens, but once it's stopped
    /// from the menu-bar icon, only that menu starts it again: reopening the app doesn't.
    async fn start_llm(&self) -> Result<(), String> {
        if self.llm.answers().await {
            return Ok(());
        }
        let limit = self.config.waits.llm_start;
        if self.is_open(&self.config.llm_app).await {
            self.step("Waiting for MLX-Serve's server…");
            // It may be starting, or restarting after a settings change.
            if self.wait_for(limit.min(Duration::from_secs(10)), || self.llm.answers()).await {
                return Ok(());
            }
            return Err("MLX-Serve is open, but its server is stopped. Start it from MLX-Serve's menu-bar icon, then switch again.".into());
        }
        self.step("Opening MLX-Serve…");
        self.open(&self.config.llm_app, "MLX-Serve", "llm-app")?;
        self.step("Waiting for MLX-Serve to start…");
        if self.wait_for(limit, || self.llm.answers()).await {
            return Ok(());
        }
        Err(format!(
            "MLX-Serve opened, but its server didn't start within {} s. Start it from MLX-Serve's menu-bar icon, then switch again.",
            limit.as_secs()
        ))
    }

    /// Stop the image jobs that are queued or running.
    async fn cancel_jobs(&self) {
        let active = self.jobs.active_ids();
        if active.is_empty() {
            return;
        }
        self.step(format!(
            "Cancelling {} image job{}…",
            active.len(),
            if active.len() == 1 { "" } else { "s" }
        ));
        for id in &active {
            if let Err(e) = self.jobs.cancel(id).await {
                tracing::warn!("Could not cancel {id} before switching: {e}");
            }
        }
    }

    /// Unload ComfyUI's models and wait until its memory stops falling.
    async fn free_comfy(&self) -> Result<(), String> {
        if !self.comfy.reachable().await {
            return Ok(());
        }
        self.step("Freeing ComfyUI's memory…");
        let before = memory_of(&self.comfy.base).await;
        self.comfy.free().await.map_err(|e| e.to_string())?;
        // ComfyUI frees between jobs; wait until its memory stops falling.
        if before.is_some() {
            let poll = self.config.waits.poll;
            let mut last = before;
            let mut steady = 0;
            let deadline = Instant::now() + self.config.waits.free;
            while steady < 2 && Instant::now() < deadline {
                tokio::time::sleep(poll).await;
                let now = memory_of(&self.comfy.base).await;
                let close = matches!((last, now), (Some(a), Some(b)) if a.abs_diff(b) <= a / 50);
                steady = if close { steady + 1 } else { 0 };
                last = now;
            }
        }
        Ok(())
    }

    fn set_image_mode(&self, kind: Kind) {
        let mut saved = self.saved.lock().unwrap();
        if saved.image_mode == kind {
            return;
        }
        saved.image_mode = kind;
        if let Some(path) = &self.config.path {
            if let Err(e) = store::save_json(path, &*saved, false) {
                tracing::warn!("Could not save {} ({e}).", path.display());
            }
        }
    }

    async fn to_images(&self, kind: Kind) -> Result<(), String> {
        let waits = self.config.waits.clone();
        // ComfyUI loads its models with the first prompt, so it can start beside the LLM.
        self.start_comfy().await?;
        if let Ok(loaded) = self.llm.loaded().await {
            if !loaded.ids.is_empty() {
                self.remember(loaded.ids.clone());
                for id in &loaded.ids {
                    self.step(format!("Unloading {}…", short_name(id)));
                    self.llm.unload(id).await?;
                }
                self.step("Waiting for the LLM's memory to free…");
                let freed = self
                    .wait_for(waits.unload, || async {
                        matches!(self.llm.loaded().await, Ok(l) if l.ids.is_empty())
                    })
                    .await;
                if !freed {
                    return Err("MLX-Serve still holds the LLM after unloading it.".into());
                }
            }
        }
        if kind != self.image_mode() {
            // The other workflow's model goes before this one's can load with the next job.
            self.cancel_jobs().await;
            self.free_comfy().await?;
            self.set_image_mode(kind);
        }
        self.jobs.set_comfy_ok(true);
        Ok(())
    }

    async fn to_llm(&self) -> Result<(), String> {
        let waits = self.config.waits.clone();
        // MLX-Serve's server holds no model until asked, so it starts before ComfyUI lets go.
        self.start_llm().await?;
        self.cancel_jobs().await;
        self.free_comfy().await?;
        let remembered = self.saved.lock().unwrap().llm_models.clone();
        let models = if remembered.is_empty() {
            self.config.llm_model.clone().into_iter().collect()
        } else {
            remembered
        };
        let held = self.llm.loaded().await.map(|l| l.ids).unwrap_or_default();
        for id in models.iter().filter(|id| !held.contains(id)) {
            self.step(format!("Loading {}…", short_name(id)));
            let deadline = Instant::now() + waits.room;
            loop {
                match self.llm.load(id).await {
                    Ok(()) => break,
                    Err(LoadError::NoRoom(message)) if Instant::now() < deadline => {
                        tracing::info!("MLX-Serve has no room for {id} yet ({message}); retrying");
                        self.step(format!("Waiting for memory to load {}…", short_name(id)));
                        tokio::time::sleep(waits.poll * 5).await;
                    }
                    Err(LoadError::NoRoom(message) | LoadError::Failed(message)) => return Err(message),
                }
            }
        }
        Ok(())
    }

    /// Hermes (or anything else) can make MLX-Serve load the LLM while ComfyUI holds an image
    /// model. When that happens outside a switch, the LLM wins: image jobs stop and ComfyUI's
    /// memory is freed, so the two models stay together only as long as it takes to notice.
    pub async fn watch(self: Arc<Self>) {
        // Once per load of the LLM, so a ComfyUI that stays big isn't freed over and over.
        let mut yielded = false;
        loop {
            tokio::time::sleep(self.config.waits.watch).await;
            match self.yield_to_llm(yielded).await {
                Ok(done) => yielded = done,
                Err(e) => tracing::warn!("Could not free ComfyUI for the LLM: {e}"),
            }
        }
    }

    /// Free ComfyUI if the LLM is loaded beside an image model. Returns whether the LLM is loaded
    /// and ComfyUI was freed for it.
    async fn yield_to_llm(&self, already: bool) -> Result<bool, String> {
        if self.switching().is_some() {
            return Ok(false);
        }
        let Some(models) = self.llm_models().await else {
            return Ok(false);
        };
        if already {
            return Ok(true);
        }
        let holding = memory_of(&self.comfy.base)
            .await
            .is_some_and(|bytes| bytes > self.config.busy_bytes);
        let active = self.jobs.active_ids();
        if !holding && active.is_empty() {
            return Ok(false);
        }
        let model = short_name(&models[0]).to_string();
        tracing::info!("MLX-Serve loaded {model} outside a switch; freeing ComfyUI");
        for id in &active {
            if let Err(e) = self.jobs.cancel(id).await {
                tracing::warn!("Could not cancel {id} for the LLM: {e}");
            }
        }
        self.comfy.free().await.map_err(|e| e.to_string())?;
        let cancelled = match active.len() {
            0 => String::new(),
            1 => " and cancelled 1 image job".into(),
            n => format!(" and cancelled {n} image jobs"),
        };
        self.state.lock().unwrap().notice = Some(format!(
            "Another app loaded the LLM ({model}), so Prompt freed ComfyUI's memory{cancelled}."
        ));
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_drop_the_owner() {
        assert_eq!(
            short_name("ddalcu/Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit"),
            "Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit"
        );
        assert_eq!(short_name("gemma"), "gemma");
    }

    #[test]
    fn app_names_come_from_the_bundle() {
        assert_eq!(app_name(&Some(PathBuf::from(DEFAULT_IMAGE_APP))).as_deref(), Some("Comfy Desktop"));
        assert_eq!(app_name(&None), None);
    }

    #[test]
    fn apps_are_open_only_when_launchd_runs_them() {
        // This test process isn't an app, and a path that is no app is never open.
        assert!(!app_is_open(Path::new("/Applications/No Such App.app")));
    }

    #[test]
    fn memory_is_only_read_for_this_computer() {
        assert_eq!(listener_memory("http://192.168.1.14:11234"), None);
    }
}
