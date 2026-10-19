//! The HTTP API and the page.
//!
//! Every route except `/health` needs the password: the app sends it as
//! `Authorization: Bearer <password>`, browsers through their own password prompt (Basic).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::extract::rejection::BytesRejection;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::{Auth, Check};
use crate::comfy::{Comfy, ComfyError, ImageRef};
use crate::jobs::{Jobs, NewJob, random_u32};
use crate::services::{Mode, Refusal, Services};
use crate::workflow::{Kind, Run, Workflow};

pub const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_BODY_BYTES: usize = 256 * 1024;
/// A full-size photo from a phone, with room to spare.
pub const MAX_UPLOAD_BYTES: usize = 30 * 1024 * 1024;
/// Pictures sent to edit go here, inside ComfyUI's input folder.
pub const UPLOAD_FOLDER: &str = "prompt-app";
const INDEX_HTML: &str = include_str!("../static/index.html");
const PAGE_CSP: &str = "default-src 'self'; img-src 'self' blob: data:; style-src 'self' 'unsafe-inline'; \
    script-src 'self' 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

pub struct App {
    /// The workflow that makes pictures from a prompt.
    pub workflow: Arc<Workflow>,
    /// The workflow that changes a picture; None when no edit workflow is set up.
    pub edit: Option<Arc<Workflow>>,
    pub comfy: Comfy,
    pub jobs: Arc<Jobs>,
    pub auth: Arc<Auth>,
    /// Switching between image generation and the LLM; None when turned off.
    pub services: Option<Arc<Services>>,
}

pub fn router(app: Arc<App>) -> Router {
    let protected = Router::new()
        .route("/", get(index))
        .route("/api/info", get(info))
        .route("/api/services", get(services))
        .route("/api/services/switch", post(switch))
        .route("/api/generate", post(generate))
        .route("/api/uploads", post(upload).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/api/jobs", get(list))
        .route("/api/jobs/{id}", get(job).delete(remove))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/jobs/{id}/images/{index}", get(image))
        .route("/api/jobs/{id}/source", get(source))
        .fallback(not_found)
        .layer(middleware::from_fn(same_origin))
        .layer(middleware::from_fn_with_state(app.clone(), require_password));
    Router::new()
        .route("/health", get(health))
        .merge(protected)
        .layer(middleware::from_fn(security_headers))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(app)
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"error": message.into()})),
    )
        .into_response()
}

fn comfy_error(e: ComfyError) -> Response {
    let (status, message) = comfy_failure(e);
    error(status, message)
}

/// A request that can't go on: the status and message it gets.
type Failure = (StatusCode, String);

fn comfy_failure(e: ComfyError) -> Failure {
    match e {
        ComfyError::Unavailable(m) => (StatusCode::SERVICE_UNAVAILABLE, m),
        ComfyError::Rejected(m) => (StatusCode::BAD_GATEWAY, m),
    }
}

fn json_ok(status: StatusCode, body: impl serde::Serialize) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

// --- middleware

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

/// Which device is asking, for the wrong-password limit. tailscale serve connects from
/// 127.0.0.1 and names the device in X-Forwarded-For.
fn client_of(request: &Request) -> String {
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let forwarded = request.headers().get("x-forwarded-for").and_then(|v| v.to_str().ok());
    match (peer, forwarded) {
        (Some(ip), Some(list)) if ip.is_loopback() => list.split(',').next().unwrap_or("").trim().to_string(),
        (Some(ip), _) => ip.to_string(),
        (None, _) => "unknown".into(),
    }
}

/// The password sent with the request: a bearer token from the app, or a browser's Basic login.
fn password_of(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    if let Some(token) = value.strip_prefix("Bearer ") {
        return Some(token.trim().to_string());
    }
    let basic = value.strip_prefix("Basic ")?;
    let decoded = B64
        .decode(basic.trim())
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();
    Some(decoded.split_once(':').map(|(_, p)| p.to_string()).unwrap_or_default())
}

async fn require_password(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let client = client_of(&request);
    let check = match password_of(request.headers()) {
        Some(password) => app.auth.check(&password, &client).await,
        None => {
            let mut response = error(StatusCode::UNAUTHORIZED, "Password required.");
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static(r#"Basic realm="ComfyUI prompt", charset="UTF-8""#),
            );
            return response;
        }
    };
    match check {
        Check::Ok => next.run(request).await,
        Check::Locked(wait) => {
            let minutes = (wait as f64 / 60.0).round().max(1.0) as u64;
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                format!(
                    "Too many wrong passwords. Try again in {minutes} minute{}.",
                    if minutes == 1 { "" } else { "s" }
                ),
            );
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(wait));
            response
        }
        Check::Wrong => {
            tracing::warn!("Wrong password from {client}");
            let mut response = error(StatusCode::UNAUTHORIZED, "Wrong password.");
            // Browsers answer this with their own password prompt; the app needs none.
            let bearer = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("Bearer "));
            if !bearer {
                response.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    HeaderValue::from_static(r#"Basic realm="ComfyUI prompt", charset="UTF-8""#),
                );
            }
            response
        }
    }
}

/// Refuse changes sent by other websites, as ComfyUI itself does: otherwise any page open in
/// a browser on this computer could queue prompts. tailscale serve keeps the browser's Host
/// header, so the Origin still matches behind it. The app sends no Origin.
async fn same_origin(request: Request, next: Next) -> Response {
    if request.method() == Method::GET || request.method() == Method::HEAD {
        return next.run(request).await;
    }
    let headers = request.headers();
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let cross_site = text("sec-fetch-site") == Some("cross-site");
    let foreign_origin = text("origin").is_some_and(|origin| {
        let netloc = origin
            .split_once("://")
            .map_or(origin, |(_, rest)| rest)
            .split('/')
            .next()
            .unwrap_or("");
        !netloc.eq_ignore_ascii_case(text("host").unwrap_or(""))
    });
    if cross_site || foreign_origin {
        return error(StatusCode::FORBIDDEN, "Requests from other websites are not accepted.");
    }
    next.run(request).await
}

// --- routes

async fn health() -> Response {
    json_ok(
        StatusCode::OK,
        json!({"status": "ok", "service": "comfyui-prompt-server", "version": env!("CARGO_PKG_VERSION")}),
    )
}

async fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Not found")
}

async fn index() -> Response {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
            (header::CONTENT_SECURITY_POLICY, PAGE_CSP),
        ],
        INDEX_HTML,
    )
        .into_response()
}

fn workflow_info(workflow: &Workflow) -> Value {
    json!({
        "title": workflow.title,
        "options": workflow.options,
    })
}

async fn info(State(app): State<Arc<App>>) -> Response {
    json_ok(
        StatusCode::OK,
        json!({
            "title": app.workflow.title,
            "workflows": {
                "generate": workflow_info(&app.workflow),
                "edit": app.edit.as_deref().map(workflow_info),
            },
            "comfy": app.jobs.comfy_ok(),
            "progress": app.jobs.feed_connected(),
            "seed": if app.workflow.seed_targets.is_empty() { "fixed" } else { "random" },
            "maxPromptChars": MAX_PROMPT_CHARS,
            "services": app.services.is_some(),
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )
}

const NO_SWITCHING: &str = "Switching between images and the LLM is turned off on this server.";
const NO_EDIT: &str = "Editing isn't set up on this server: it needs an edit workflow (--edit-workflow) and switching.";

async fn services(State(app): State<Arc<App>>) -> Response {
    match &app.services {
        Some(services) => json_ok(StatusCode::OK, services.view().await),
        None => error(StatusCode::NOT_FOUND, NO_SWITCHING),
    }
}

async fn switch(State(app): State<Arc<App>>, body: Result<Bytes, BytesRejection>) -> Response {
    let Some(services) = &app.services else {
        return error(StatusCode::NOT_FOUND, NO_SWITCHING);
    };
    let body = match json_body(body) {
        Ok(body) => body,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let to = match body.get("to").and_then(Value::as_str) {
        // "images" is what the first version of the app sends.
        Some("generate" | "images") => Mode::Generate,
        Some("edit") => Mode::Edit,
        Some("llm") => Mode::Llm,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                r#"Send {"to": "generate"}, {"to": "edit"} or {"to": "llm"}."#,
            );
        }
    };
    let cancel_jobs = body.get("cancelJobs").and_then(Value::as_bool).unwrap_or(false);
    match services.begin(to, cancel_jobs) {
        Ok(()) => json_ok(StatusCode::ACCEPTED, services.view().await),
        Err(Refusal::AlreadySwitching) => error(StatusCode::CONFLICT, "Your Mac is already switching. Wait for it to finish."),
        Err(Refusal::NoEdit) => error(StatusCode::NOT_FOUND, NO_EDIT),
        Err(Refusal::ActiveJobs(n)) => json_ok(
            StatusCode::CONFLICT,
            json!({
                "error": format!("{n} image job{} still queued or running.", if n == 1 { " is" } else { "s are" }),
                "activeJobs": n,
            }),
        ),
    }
}

/// Why a job can't go to ComfyUI right now: the Mac is switching, the LLM holds the memory, or
/// ComfyUI is set up for the other image workflow.
async fn paused(app: &App, kind: Kind) -> Option<String> {
    match &app.services {
        Some(services) => services.refusal(kind).await,
        // Without switching, nothing keeps the two image models apart.
        None => (kind == Kind::Edit).then(|| NO_EDIT.to_string()),
    }
}

/// Which picture an edit starts from: one sent to /api/uploads, or a finished job's image.
#[derive(Deserialize)]
#[serde(untagged)]
enum Source {
    Upload { upload: String },
    Job { job: String, index: Option<usize> },
}

/// An upload name this server handed out: `prompt-app/<uuid>.<ext>`.
fn upload_ref(name: &str) -> Option<ImageRef> {
    let (folder, file) = name.split_once('/')?;
    let (stem, ext) = file.rsplit_once('.')?;
    let ok = folder == UPLOAD_FOLDER
        && uuid::Uuid::parse_str(stem.split(' ').next()?).is_ok()
        && file.chars().all(|c| c.is_ascii_alphanumeric() || "-_. ()".contains(c))
        && ["png", "jpg", "webp"].contains(&ext);
    ok.then(|| ImageRef {
        filename: file.to_string(),
        subfolder: folder.to_string(),
        kind: "input".into(),
    })
}

/// The picture types ComfyUI's Load Image reads, by their first bytes.
fn picture_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else {
        None
    }
}

/// Put a picture in ComfyUI's input folder under a fresh name.
async fn store_picture(app: &App, bytes: Vec<u8>) -> Result<ImageRef, Failure> {
    let Some((mime, ext)) = picture_type(&bytes) else {
        return Err((StatusCode::UNSUPPORTED_MEDIA_TYPE, "Send a JPEG, PNG or WebP picture.".into()));
    };
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).expect("randomness");
    let name = format!("{}.{ext}", uuid::Builder::from_random_bytes(random).into_uuid());
    app.comfy
        .upload_image(bytes, UPLOAD_FOLDER, &name, mime)
        .await
        .map_err(comfy_failure)
}

async fn upload(State(app): State<Arc<App>>, body: Result<Bytes, BytesRejection>) -> Response {
    let Ok(bytes) = body else {
        return error(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("Send a picture under {} MB.", MAX_UPLOAD_BYTES / 1024 / 1024),
        );
    };
    match store_picture(&app, bytes.to_vec()).await {
        Ok(saved) => json_ok(StatusCode::CREATED, json!({"upload": saved.load_image_name()})),
        Err((status, message)) => error(status, message),
    }
}

/// The picture an edit starts from, in ComfyUI's input folder. A finished job's image is copied
/// there, so the edit keeps its source even if that job is removed.
async fn resolve_source(app: &App, source: Source) -> Result<ImageRef, Failure> {
    match source {
        Source::Upload { upload } => {
            upload_ref(&upload).ok_or_else(|| (StatusCode::BAD_REQUEST, "That upload isn't one of this server's.".into()))
        }
        Source::Job { job, index } => {
            let image = job_id(&job)
                .and_then(|id| app.jobs.image(&id, index.unwrap_or(0)))
                .ok_or_else(|| (StatusCode::NOT_FOUND, "That image isn't in the history any more.".into()))?;
            let bytes = app
                .comfy
                .image(&image, false)
                .await
                .map_err(comfy_failure)?
                .bytes()
                .await
                .map_err(|e| (StatusCode::BAD_GATEWAY, format!("Could not read the image from ComfyUI ({e}).")))?;
            store_picture(app, bytes.to_vec()).await
        }
    }
}

fn json_body(body: Result<Bytes, BytesRejection>) -> Result<serde_json::Map<String, Value>, &'static str> {
    let bytes = body.map_err(|_| "Send a JSON body under 256 KB.")?;
    match serde_json::from_slice(&bytes) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err("Send a JSON object."),
    }
}

async fn generate(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Response {
    let body = match json_body(body) {
        Ok(body) => body,
        Err(message) => return error(StatusCode::BAD_REQUEST, message),
    };
    let prompt = body.get("prompt").and_then(Value::as_str).map(str::trim).unwrap_or("");
    if prompt.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Type a prompt first.");
    }
    if prompt.chars().count() > MAX_PROMPT_CHARS {
        return error(
            StatusCode::BAD_REQUEST,
            format!("Keep the prompt under {MAX_PROMPT_CHARS} characters."),
        );
    }
    let kind = match body.get("kind").and_then(Value::as_str) {
        None | Some("generate") => Kind::Generate,
        Some("edit") => Kind::Edit,
        Some(_) => return error(StatusCode::BAD_REQUEST, r#"Send "kind": "generate" or "edit"."#),
    };
    let workflow = match kind {
        Kind::Generate => app.workflow.clone(),
        Kind::Edit => match &app.edit {
            Some(edit) => edit.clone(),
            None => return error(StatusCode::NOT_FOUND, NO_EDIT),
        },
    };
    let options: HashMap<String, bool> = match body.get("options") {
        None | Some(Value::Null) => HashMap::new(),
        Some(value) => match serde_json::from_value(value.clone()) {
            Ok(options) => options,
            Err(_) => return error(StatusCode::BAD_REQUEST, r#"Send "options" as {"Name": true or false}."#),
        },
    };
    let source = match (kind, body.get("source")) {
        (Kind::Generate, _) => None,
        (Kind::Edit, None | Some(Value::Null)) => return error(StatusCode::BAD_REQUEST, "Choose a picture to edit first."),
        (Kind::Edit, Some(value)) => match serde_json::from_value::<Source>(value.clone()) {
            Ok(source) => Some(source),
            Err(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    r#"Send "source" as {"upload": name} or {"job": id, "index": n}."#,
                );
            }
        },
    };
    if let Some(reason) = paused(&app, kind).await {
        return error(StatusCode::SERVICE_UNAVAILABLE, reason);
    }
    let source = match source {
        Some(source) => match resolve_source(&app, source).await {
            Ok(image) => Some(image),
            Err((status, message)) => return error(status, message),
        },
        None => None,
    };
    let image_name = source.as_ref().map(ImageRef::load_image_name);
    let (graph, seed) = workflow.build(&Run {
        prompt,
        seed: u64::from(random_u32()),
        image: image_name.as_deref(),
        options: &options,
    });
    let new = NewJob {
        prompt: prompt.to_string(),
        kind,
        graph,
        seed,
        source,
        options: workflow
            .options_for(&options)
            .into_iter()
            .map(|(name, on)| (name.to_string(), on))
            .collect(),
    };
    match app.jobs.submit(new).await {
        Ok(job) => {
            // tailscale serve names the tailnet user; without it, fall back to the peer address.
            let who = headers
                .get("tailscale-user-login")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .unwrap_or_else(|| peer.ip().to_string());
            tracing::info!("Queued {} for {who}", job.id);
            json_ok(StatusCode::ACCEPTED, job)
        }
        Err(ComfyError::Rejected(m)) => error(StatusCode::UNPROCESSABLE_ENTITY, m),
        Err(e) => comfy_error(e),
    }
}

#[derive(Deserialize)]
struct ListQuery {
    limit: Option<usize>,
    before: Option<String>,
}

async fn list(State(app): State<Arc<App>>, Query(query): Query<ListQuery>) -> Response {
    let limit = query.limit.unwrap_or(30).clamp(1, 100);
    let (jobs, more) = app.jobs.list(limit, query.before.as_deref());
    json_ok(StatusCode::OK, json!({"jobs": jobs, "more": more}))
}

fn job_id(raw: &str) -> Option<String> {
    uuid::Uuid::parse_str(raw).ok().map(|u| u.hyphenated().to_string())
}

async fn job(State(app): State<Arc<App>>, Path(raw): Path<String>) -> Response {
    match job_id(&raw).and_then(|id| app.jobs.get(&id)) {
        Some(job) => json_ok(StatusCode::OK, job),
        None => error(StatusCode::NOT_FOUND, "No such job."),
    }
}

async fn cancel(State(app): State<Arc<App>>, Path(raw): Path<String>) -> Response {
    let Some(id) = job_id(&raw) else {
        return error(StatusCode::NOT_FOUND, "No such job.");
    };
    match app.jobs.cancel(&id).await {
        Ok(Some(job)) => json_ok(StatusCode::OK, job),
        Ok(None) => error(StatusCode::NOT_FOUND, "No such job."),
        Err(e) => comfy_error(e),
    }
}

async fn remove(State(app): State<Arc<App>>, Path(raw): Path<String>) -> Response {
    match job_id(&raw) {
        Some(id) if app.jobs.remove(&id).await => json_ok(StatusCode::OK, json!({"id": id, "removed": true})),
        _ => error(StatusCode::NOT_FOUND, "No such job."),
    }
}

/// RFC 6266: an ASCII fallback name plus the exact UTF-8 one.
fn content_disposition(filename: &str) -> String {
    let ascii: String = filename
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || "._ -".contains(c) { c } else { '_' })
        .collect();
    let encoded: String = filename
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"._-~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

async fn image(
    State(app): State<Arc<App>>,
    Path((raw, index)): Path<(String, usize)>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    match job_id(&raw).and_then(|id| app.jobs.image(&id, index)) {
        Some(image) => send_image(&app, &image, &query).await,
        None => error(StatusCode::NOT_FOUND, "No such image."),
    }
}

async fn source(State(app): State<Arc<App>>, Path(raw): Path<String>, Query(query): Query<HashMap<String, String>>) -> Response {
    match job_id(&raw).and_then(|id| app.jobs.source(&id)) {
        Some(image) => send_image(&app, &image, &query).await,
        None => error(StatusCode::NOT_FOUND, "This job has no source picture."),
    }
}

async fn send_image(app: &App, image: &ImageRef, query: &HashMap<String, String>) -> Response {
    let download = query.contains_key("download");
    let preview = query.contains_key("preview") && !download;
    let upstream = match app.comfy.image(image, preview).await {
        Ok(upstream) => upstream,
        Err(e) => return comfy_error(e),
    };
    let mut response = Response::builder()
        .header(
            header::CONTENT_TYPE,
            upstream
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or(HeaderValue::from_static("application/octet-stream")),
        )
        .header(header::CACHE_CONTROL, "private, max-age=86400");
    if let Some(length) = upstream.headers().get(header::CONTENT_LENGTH) {
        response = response.header(header::CONTENT_LENGTH, length.clone());
    }
    if download {
        response = response.header(header::CONTENT_DISPOSITION, content_disposition(&image.filename));
    }
    // Stream straight through: the image is never held in memory whole.
    response
        .body(Body::from_stream(upstream.bytes_stream()))
        .unwrap_or_else(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Could not send the image."))
}
