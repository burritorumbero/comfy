//! The HTTP API against a fake ComfyUI that queues, runs and finishes jobs on command.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Multipart, Path as UrlPath, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use comfyui_prompt_server::auth::{Auth, WRONG_PASSWORDS_ALLOWED};
use comfyui_prompt_server::comfy::Comfy;
use comfyui_prompt_server::http::{App, MAX_PROMPT_CHARS, router};
use comfyui_prompt_server::jobs::Jobs;
use comfyui_prompt_server::llm::Llm;
use comfyui_prompt_server::services::{self, Services};
use comfyui_prompt_server::workflow::{Kind, Workflow};
use reqwest::StatusCode;
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse battery";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really a png";
const IMAGE_APP: &str = "/Applications/Comfy Desktop.app";
const LLM_APP: &str = "/Applications/MLX-Serve.app";

#[derive(Default)]
struct Fake {
    /// ComfyUI isn't running.
    down: bool,
    /// Comfy Desktop is open (it may be, with ComfyUI down).
    app_open: bool,
    prompts: Vec<Value>,
    running: Vec<String>,
    pending: Vec<String>,
    history: HashMap<String, Value>,
    /// Calls to /free.
    freed: usize,
    /// Pictures put in the input folder, by "subfolder/filename".
    uploads: HashMap<String, Vec<u8>>,
}

type Shared = Arc<Mutex<Fake>>;

/// MLX-Serve: which models it holds, and what it was asked to load and unload.
#[derive(Default)]
struct FakeLlm {
    down: bool,
    /// MLX-Serve is open (it may be, with its server stopped).
    app_open: bool,
    /// Opening its app doesn't start the server.
    stays_down: bool,
    loaded: Vec<String>,
    loads: Vec<String>,
    unloads: Vec<String>,
    /// Loads to refuse for lack of memory before one succeeds.
    no_room: usize,
}

type SharedLlm = Arc<Mutex<FakeLlm>>;

fn unavailable() -> (StatusCode, Json<Value>) {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({"error": "down"})))
}

async fn fake_llm_models(State(llm): State<SharedLlm>) -> (StatusCode, Json<Value>) {
    let llm = llm.lock().unwrap();
    if llm.down {
        return unavailable();
    }
    let mut data: Vec<Value> = llm
        .loaded
        .iter()
        .map(|id| json!({"id": id, "object": "model", "loaded": true, "state": "ready", "bytes_resident": 1000}))
        .collect();
    data.push(json!({"id": "org/on-disk", "object": "model", "loaded": false, "state": "unloaded", "bytes_resident": 0}));
    (StatusCode::OK, Json(json!({"object": "list", "data": data})))
}

async fn fake_llm_load(State(llm): State<SharedLlm>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    let mut llm = llm.lock().unwrap();
    if llm.no_room > 0 {
        llm.no_room -= 1;
        let error = json!({"error": {"message": "Refusing to load: needs ~100 GB", "type": "out_of_memory"}});
        return (StatusCode::SERVICE_UNAVAILABLE, Json(error));
    }
    let id = body["model"].as_str().unwrap().to_string();
    llm.loads.push(id.clone());
    llm.loaded.push(id.clone());
    (StatusCode::OK, Json(json!({"model": {"id": id, "loaded": true}})))
}

async fn fake_llm_unload(State(llm): State<SharedLlm>, Json(body): Json<Value>) -> Json<Value> {
    let mut llm = llm.lock().unwrap();
    let id = body["model"].as_str().unwrap().to_string();
    llm.loaded.retain(|m| m != &id);
    llm.unloads.push(id.clone());
    Json(json!({"model": {"id": id, "loaded": false, "state": "unloaded"}}))
}

fn fake_llm(llm: SharedLlm) -> Router {
    Router::new()
        .route(
            "/health",
            get(|State(llm): State<SharedLlm>| async move {
                if llm.lock().unwrap().down {
                    unavailable()
                } else {
                    (StatusCode::OK, Json(json!({"status": "ok"})))
                }
            }),
        )
        .route("/v1/models", get(fake_llm_models))
        .route("/v1/load-model", post(fake_llm_load))
        .route("/v1/unload-model", post(fake_llm_unload))
        .with_state(llm)
}

fn finished(images: Value, messages: Value) -> Value {
    json!({"outputs": {"60": {"images": images}}, "status": {"status_str": "success", "completed": true, "messages": messages}})
}

impl Fake {
    fn start_next(&mut self) -> String {
        let id = self.pending.remove(0);
        self.running.push(id.clone());
        id
    }

    fn finish(&mut self, id: &str) {
        self.running.retain(|r| r != id);
        self.pending.retain(|p| p != id);
        let images = json!([{"filename": "Qwen-Image-2512_00001_.png", "subfolder": "", "type": "output"}, {"filename": "tmp.png", "subfolder": "", "type": "temp"}]);
        let messages = json!([["execution_start", {"timestamp": 1000}], ["execution_success", {"timestamp": 42500}]]);
        self.history.insert(id.to_string(), finished(images, messages));
    }
}

async fn fake_prompt(State(fake): State<Shared>, Json(body): Json<Value>) -> Json<Value> {
    let mut fake = fake.lock().unwrap();
    let id = format!("00000000-0000-4000-8000-{:012x}", fake.prompts.len() + 1);
    fake.prompts.push(body);
    fake.pending.push(id.clone());
    Json(json!({"prompt_id": id, "number": fake.prompts.len(), "node_errors": {}}))
}

async fn fake_queue(State(fake): State<Shared>) -> Json<Value> {
    let fake = fake.lock().unwrap();
    let item = |n: usize, id: &String| json!([n, id, {}, {}, []]);
    // Pending in reverse, as ComfyUI's heap may list it: the server sorts by number.
    let pending: Vec<Value> = fake.pending.iter().enumerate().rev().map(|(n, id)| item(n + 10, id)).collect();
    Json(json!({"queue_running": fake.running.iter().map(|id| item(0, id)).collect::<Vec<_>>(), "queue_pending": pending}))
}

async fn fake_queue_change(State(fake): State<Shared>, Json(body): Json<Value>) {
    let mut fake = fake.lock().unwrap();
    for id in body["delete"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        fake.pending.retain(|p| p != id);
    }
}

async fn fake_interrupt(State(fake): State<Shared>, Json(body): Json<Value>) {
    let mut fake = fake.lock().unwrap();
    let id = body["prompt_id"].as_str().unwrap().to_string();
    fake.running.retain(|r| r != &id);
    let interrupted =
        json!({"outputs": {}, "status": {"status_str": "error", "completed": false, "messages": [["execution_interrupted", {}]]}});
    fake.history.insert(id, interrupted);
}

async fn fake_history(State(fake): State<Shared>, UrlPath(id): UrlPath<String>) -> Json<Value> {
    let fake = fake.lock().unwrap();
    Json(fake.history.get(&id).map_or(json!({}), |entry| json!({ id: entry })))
}

async fn fake_upload(State(fake): State<Shared>, mut form: Multipart) -> Json<Value> {
    let (mut subfolder, mut name, mut bytes) = (String::new(), String::new(), Vec::new());
    while let Some(field) = form.next_field().await.unwrap() {
        match field.name().unwrap_or("") {
            "image" => {
                name = field.file_name().unwrap_or("").to_string();
                bytes = field.bytes().await.unwrap().to_vec();
            }
            "subfolder" => subfolder = field.text().await.unwrap(),
            _ => {}
        }
    }
    fake.lock().unwrap().uploads.insert(format!("{subfolder}/{name}"), bytes);
    Json(json!({"name": name, "subfolder": subfolder, "type": "input"}))
}

async fn fake_view(State(fake): State<Shared>, Query(query): Query<HashMap<String, String>>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let text = |key: &str| query.get(key).map(String::as_str).unwrap_or("");
    if text("type") == "input" {
        let key = format!("{}/{}", text("subfolder"), text("filename"));
        return match fake.lock().unwrap().uploads.get(&key) {
            Some(bytes) => ([(axum::http::header::CONTENT_TYPE, "image/png")], bytes.clone()).into_response(),
            None => axum::http::StatusCode::NOT_FOUND.into_response(),
        };
    }
    if query.get("filename").map(String::as_str) != Some("Qwen-Image-2512_00001_.png")
        || query.get("type").map(String::as_str) != Some("output")
    {
        return axum::http::StatusCode::NOT_FOUND.into_response();
    }
    let kind = if query.contains_key("preview") { "image/webp" } else { "image/png" };
    ([(axum::http::header::CONTENT_TYPE, kind)], PNG).into_response()
}

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    format!("http://{address}")
}

struct Harness {
    base: String,
    fake: Shared,
    llm: SharedLlm,
    /// The apps the server opened.
    opened: Arc<Mutex<Vec<PathBuf>>>,
    jobs: Arc<Jobs>,
    services: Option<Arc<Services>>,
    http: reqwest::Client,
    _dir: tempfile::TempDir,
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/qwen_image_2512_api.json")
}

fn edit_fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("workflows/qwen_image_edit_2511_api.json")
}

async fn build(comfy_url: Option<String>, switch: bool) -> Harness {
    let fake: Shared = Arc::default();
    let fake_url = spawn(
        Router::new()
            .route(
                "/system_stats",
                get(|State(fake): State<Shared>| async move {
                    if fake.lock().unwrap().down {
                        unavailable()
                    } else {
                        (StatusCode::OK, Json(json!({"system": {}})))
                    }
                }),
            )
            .route("/prompt", post(fake_prompt))
            .route("/queue", get(fake_queue).post(fake_queue_change))
            .route("/interrupt", post(fake_interrupt))
            .route("/history/{id}", get(fake_history))
            .route("/view", get(fake_view))
            .route("/upload/image", post(fake_upload).layer(axum::extract::DefaultBodyLimit::disable()))
            .route(
                "/free",
                post(|State(fake): State<Shared>| async move { fake.lock().unwrap().freed += 1 }),
            )
            .with_state(fake.clone()),
    )
    .await;
    let llm: SharedLlm = Arc::default();
    let llm_url = spawn(fake_llm(llm.clone())).await;
    let dir = tempfile::tempdir().unwrap();
    let auth = Arc::new(Auth::with_iterations(None, 1000));
    auth.set_password(PASSWORD).unwrap();
    let workflow = Arc::new(Workflow::open(&fixture(), Kind::Generate, None, None, false, Some("Test".into())).unwrap());
    let edit = switch.then(|| Arc::new(Workflow::open(&edit_fixture(), Kind::Edit, None, None, false, None).unwrap()));
    let comfy = Comfy::new(&comfy_url.unwrap_or(fake_url));
    let jobs = Arc::new(Jobs::open(comfy.clone(), "client-1".into(), Some(dir.path().join("jobs.json"))));
    jobs.set_comfy_ok(true);

    // Opening an app brings its fake server up, as the real apps start theirs.
    let opened: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let apps = {
        let (opened, fake, llm) = (opened.clone(), fake.clone(), llm.clone());
        let (fake_open, llm_open) = (fake.clone(), llm.clone());
        services::Apps {
            open: Arc::new(move |app: &Path| {
                opened.lock().unwrap().push(app.to_path_buf());
                if app == Path::new(IMAGE_APP) {
                    let mut fake = fake.lock().unwrap();
                    (fake.app_open, fake.down) = (true, false);
                } else {
                    let mut llm = llm.lock().unwrap();
                    (llm.app_open, llm.down) = (true, llm.stays_down);
                }
                Ok(())
            }),
            is_open: Arc::new(move |app: &Path| {
                if app == Path::new(IMAGE_APP) {
                    fake_open.lock().unwrap().app_open
                } else {
                    llm_open.lock().unwrap().app_open
                }
            }),
        }
    };
    let services = switch.then(|| {
        Arc::new(Services::new(
            comfy.clone(),
            Llm::new(&llm_url),
            jobs.clone(),
            services::Config {
                image_app: Some(IMAGE_APP.into()),
                llm_app: Some(LLM_APP.into()),
                llm_model: Some("org/default".into()),
                path: Some(dir.path().join("services.json")),
                edit: true,
                // Any memory at all counts as holding a model, so the watcher has something to free.
                busy_bytes: 0,
                apps,
                waits: services::Waits {
                    poll: Duration::from_millis(10),
                    comfy_start: Duration::from_millis(500),
                    llm_start: Duration::from_millis(500),
                    unload: Duration::from_millis(500),
                    room: Duration::from_secs(2),
                    free: Duration::from_millis(200),
                    watch: Duration::from_millis(10),
                },
            },
        ))
    });
    let base = spawn(router(Arc::new(App {
        workflow,
        edit,
        comfy,
        jobs: jobs.clone(),
        auth,
        services: services.clone(),
    })))
    .await;
    Harness {
        base,
        fake,
        llm,
        opened,
        jobs,
        services,
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        _dir: dir,
    }
}

async fn harness_with(comfy_url: Option<String>) -> Harness {
    build(comfy_url, true).await
}

async fn harness() -> Harness {
    harness_with(None).await
}

impl Harness {
    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http.request(method, format!("{}{path}", self.base)).bearer_auth(PASSWORD)
    }

    async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response = self.request(reqwest::Method::GET, path).send().await.unwrap();
        (response.status(), response.json().await.unwrap_or(Value::Null))
    }

    async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        let response = self.request(reqwest::Method::POST, path).json(&body).send().await.unwrap();
        (response.status(), response.json().await.unwrap_or(Value::Null))
    }

    async fn generate(&self, prompt: &str) -> Value {
        let (status, job) = self.post("/api/generate", json!({"prompt": prompt})).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{job}");
        job
    }

    async fn state_of(&self, id: &str) -> Value {
        self.get(&format!("/api/jobs/{id}")).await.1
    }

    async fn upload(&self, bytes: &[u8]) -> (StatusCode, Value) {
        let response = self
            .request(reqwest::Method::POST, "/api/uploads")
            .body(bytes.to_vec())
            .send()
            .await
            .unwrap();
        (response.status(), response.json().await.unwrap_or(Value::Null))
    }

    /// Ask for a switch and wait for it to finish; returns the services afterwards.
    async fn switch(&self, body: Value) -> Value {
        let (status, started) = self.post("/api/services/switch", body).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{started}");
        for _ in 0..500 {
            let (_, services) = self.get("/api/services").await;
            if services["switching"].is_null() {
                return services;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the switch did not finish");
    }
}

#[tokio::test]
async fn generate_follow_and_download() {
    let h = harness().await;
    let first = h.generate("  a red fox in the snow  ").await;
    let second = h.generate("a lighthouse").await;
    let (id, second_id) = (first["id"].as_str().unwrap(), second["id"].as_str().unwrap());
    assert_eq!(first["state"], "queued");
    assert_eq!(first["prompt"], "a red fox in the snow");

    {
        let fake = h.fake.lock().unwrap();
        let sent = &fake.prompts[0];
        assert_eq!(sent["client_id"], "client-1");
        assert_eq!(sent["prompt"]["238:227"]["inputs"]["text"], "a red fox in the snow");
        assert_eq!(sent["prompt"]["238:230"]["inputs"]["seed"], first["seed"]);
    }

    h.fake.lock().unwrap().start_next();
    h.jobs.reconcile().await;
    assert_eq!(h.state_of(id).await["state"], "running");
    let waiting = h.state_of(second_id).await;
    assert_eq!((waiting["state"].as_str(), waiting["ahead"].as_u64()), (Some("queued"), Some(1)));

    // The websocket's progress shows up in the job.
    h.jobs
        .on_event(&json!({"type": "progress", "data": {"prompt_id": id, "value": 23, "max": 50}}));
    assert_eq!(h.state_of(id).await["progress"], json!({"value": 23, "max": 50}));

    h.fake.lock().unwrap().finish(id);
    h.jobs.reconcile().await;
    let done = h.state_of(id).await;
    assert_eq!(done["state"], "done");
    assert_eq!(done["seconds"], 41.5);
    assert!(done.get("progress").is_none());
    let images = done["images"].as_array().unwrap();
    assert_eq!(images.len(), 1, "Save Image wins over Preview Image");

    let response = h
        .request(reqwest::Method::GET, images[0]["download"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/png");
    assert!(
        response.headers()["content-disposition"]
            .to_str()
            .unwrap()
            .contains(r#"attachment; filename="Qwen-Image-2512_00001_.png""#)
    );
    assert_eq!(response.bytes().await.unwrap().as_ref(), PNG);
    let preview = h
        .request(reqwest::Method::GET, images[0]["preview"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(preview.headers()["content-type"], "image/webp");

    // ComfyUI restarts and forgets its history; the job and its image are still served.
    h.fake.lock().unwrap().history.clear();
    assert_eq!(h.state_of(id).await["state"], "done");
    let again = h
        .request(reqwest::Method::GET, images[0]["url"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::OK);
}

#[tokio::test]
async fn history_is_shared_paged_and_survives_a_restart() {
    let h = harness().await;
    let ids: Vec<String> = futures_ids(&h, &["one", "two", "three"]).await;

    let (_, page) = h.get("/api/jobs?limit=2").await;
    let prompts: Vec<&str> = page["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|j| j["prompt"].as_str().unwrap())
        .collect();
    assert_eq!((prompts, page["more"].as_bool()), (vec!["three", "two"], Some(true)));
    let (_, rest) = h.get(&format!("/api/jobs?limit=2&before={}", ids[1])).await;
    assert_eq!(rest["jobs"][0]["prompt"], "one");
    assert_eq!(rest["more"], false);

    let response = h
        .request(reqwest::Method::DELETE, &format!("/api/jobs/{}", ids[0]))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(h.get(&format!("/api/jobs/{}", ids[0])).await.0, StatusCode::NOT_FOUND);
    assert!(
        !h.fake.lock().unwrap().pending.contains(&ids[0]),
        "removing an active job takes it out of ComfyUI's queue"
    );

    let reopened = Jobs::open(Comfy::new("http://127.0.0.1:9"), "c".into(), Some(h._dir.path().join("jobs.json")));
    let (jobs, _) = reopened.list(10, None);
    assert_eq!(jobs.iter().map(|j| j.prompt.as_str()).collect::<Vec<_>>(), ["three", "two"]);
}

async fn futures_ids(h: &Harness, prompts: &[&str]) -> Vec<String> {
    let mut ids = Vec::new();
    for prompt in prompts {
        ids.push(h.generate(prompt).await["id"].as_str().unwrap().to_string());
    }
    ids
}

#[tokio::test]
async fn cancel_waiting_and_running_jobs() {
    let h = harness().await;
    let ids = futures_ids(&h, &["runs", "waits"]).await;
    h.fake.lock().unwrap().start_next();
    h.jobs.reconcile().await;

    let (status, waiting) = h.post(&format!("/api/jobs/{}/cancel", ids[1]), json!({})).await;
    assert_eq!((status, waiting["state"].as_str()), (StatusCode::OK, Some("cancelled")));

    h.post(&format!("/api/jobs/{}/cancel", ids[0]), json!({})).await;
    h.jobs.reconcile().await;
    assert_eq!(h.state_of(&ids[0]).await["state"], "cancelled");
}

#[tokio::test]
async fn a_job_comfyui_forgot_is_reported_lost() {
    let h = harness().await;
    let id = h.generate("x").await["id"].as_str().unwrap().to_string();
    h.fake.lock().unwrap().pending.clear(); // ComfyUI restarted
    for _ in 0..3 {
        h.jobs.reconcile().await;
    }
    let job = h.state_of(&id).await;
    assert_eq!(job["state"], "lost");
    assert!(job["error"].as_str().unwrap().contains("restarted"));
}

#[tokio::test]
async fn every_route_but_health_needs_the_password() {
    let h = harness().await;
    let open = h.http.get(format!("{}/health", h.base)).send().await.unwrap();
    assert_eq!(open.json::<Value>().await.unwrap()["status"], "ok");

    for path in [
        "/",
        "/api/info",
        "/api/jobs",
        "/api/jobs/00000000-0000-4000-8000-000000000001",
        "/api/services",
        "/nothing-here",
    ] {
        let response = h.http.get(format!("{}{path}", h.base)).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
        assert!(
            response.headers()["www-authenticate"].to_str().unwrap().starts_with("Basic"),
            "browsers show their prompt"
        );
        assert_eq!(response.json::<Value>().await.unwrap()["error"], "Password required.");
    }
    let wrong = h
        .http
        .get(format!("{}/api/info", h.base))
        .bearer_auth("wrong password")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    assert!(wrong.headers().get("www-authenticate").is_none(), "no browser prompt for the app");

    let browser = h
        .http
        .get(format!("{}/", h.base))
        .basic_auth("anyone", Some(PASSWORD))
        .send()
        .await
        .unwrap();
    assert_eq!(browser.status(), StatusCode::OK);
    assert!(
        browser.headers()["content-security-policy"]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
    assert_eq!(browser.headers()["x-content-type-options"], "nosniff");

    let (_, info) = h.get("/api/info").await;
    assert_eq!(
        (info["title"].as_str(), info["seed"].as_str(), info["comfy"].as_bool()),
        (Some("Test"), Some("random"), Some(true))
    );
    assert_eq!(info["services"], true);

    let switch = h
        .http
        .post(format!("{}/api/services/switch", h.base))
        .json(&json!({"to": "llm"}))
        .send()
        .await
        .unwrap();
    assert_eq!(switch.status(), StatusCode::UNAUTHORIZED);
    assert!(h.fake.lock().unwrap().prompts.is_empty());
    assert_eq!(h.fake.lock().unwrap().freed, 0, "nothing was switched");
}

#[tokio::test]
async fn too_many_wrong_passwords_lock_out_that_device() {
    let h = harness().await;
    let from = |ip: &str, password: &str| {
        h.http
            .get(format!("{}/api/info", h.base))
            .header("x-forwarded-for", ip)
            .bearer_auth(password)
    };
    for _ in 0..WRONG_PASSWORDS_ALLOWED {
        assert_eq!(
            from("100.64.0.7", "guess guess").send().await.unwrap().status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let locked = from("100.64.0.7", PASSWORD).send().await.unwrap();
    assert_eq!(locked.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(locked.headers()["retry-after"].to_str().unwrap().parse::<u64>().unwrap() > 0);
    assert!(
        locked.json::<Value>().await.unwrap()["error"]
            .as_str()
            .unwrap()
            .contains("Try again in 15 minutes")
    );
    assert_eq!(
        from("100.64.0.8", PASSWORD).send().await.unwrap().status(),
        StatusCode::OK,
        "other devices are not locked out"
    );
}

#[tokio::test]
async fn other_websites_cannot_post() {
    let h = harness().await;
    let host = h.base.trim_start_matches("http://").to_string();
    let attempt = |headers: Vec<(&str, String)>| {
        let mut request = h.request(reqwest::Method::POST, "/api/generate").json(&json!({"prompt": "x"}));
        for (name, value) in headers {
            request = request.header(name, value);
        }
        request.send()
    };
    assert_eq!(
        attempt(vec![("origin", "https://evil.example".into())]).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        attempt(vec![("origin", "null".into())]).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        attempt(vec![("sec-fetch-site", "cross-site".into())]).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    assert!(h.fake.lock().unwrap().prompts.is_empty());
    assert_eq!(
        attempt(vec![("origin", format!("http://{host}")), ("sec-fetch-site", "same-origin".into())])
            .await
            .unwrap()
            .status(),
        StatusCode::ACCEPTED
    );
    // Behind tailscale serve the browser's Host header is kept, so its Origin matches.
    let behind_tailscale = vec![
        ("origin", "https://gaming-pc.tail1a2b3c.ts.net".into()),
        ("host", "gaming-pc.tail1a2b3c.ts.net".into()),
    ];
    assert_eq!(attempt(behind_tailscale).await.unwrap().status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn bad_requests() {
    let h = harness().await;
    assert_eq!(h.post("/api/generate", json!({"prompt": "   "})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        h.post("/api/generate", json!({"prompt": "x".repeat(MAX_PROMPT_CHARS + 1)})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(h.post("/api/generate", json!({"text": "x"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        h.post("/api/services/switch", json!({"to": "both"})).await.0,
        StatusCode::BAD_REQUEST
    );
    let not_json = h
        .request(reqwest::Method::POST, "/api/generate")
        .body("hello")
        .send()
        .await
        .unwrap();
    assert_eq!(not_json.json::<Value>().await.unwrap()["error"], "Send a JSON object.");
    // The server answers before reading all of it and drops the connection, so don't reuse it.
    let huge = h
        .request(reqwest::Method::POST, "/api/generate")
        .header("connection", "close")
        .body(vec![b'x'; 300_000])
        .send()
        .await
        .unwrap();
    assert_eq!(huge.status(), StatusCode::BAD_REQUEST);
    assert_eq!(h.get("/api/jobs/not-a-uuid").await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        h.get("/api/jobs/00000000-0000-4000-8000-0000000000ff").await.0,
        StatusCode::NOT_FOUND
    );
    let id = h.generate("x").await["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.get(&format!("/api/jobs/{id}/images/0")).await.0,
        StatusCode::NOT_FOUND,
        "no image before it is done"
    );
    assert_eq!(h.fake.lock().unwrap().prompts.len(), 1);
}

#[tokio::test]
async fn comfyui_down() {
    let h = harness_with(Some("http://127.0.0.1:9".into())).await;
    let (status, body) = h.post("/api/generate", json!({"prompt": "x"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("not reachable"));
    h.jobs.reconcile().await;
    assert_eq!(h.get("/api/info").await.1["comfy"], false);
}

#[tokio::test]
async fn switching_between_the_llm_and_images() {
    let h = harness().await;
    h.llm.lock().unwrap().loaded.push("org/big-llm".into());

    let (_, services) = h.get("/api/services").await;
    assert_eq!(services["llm"]["models"], json!(["org/big-llm"]));
    assert_eq!(
        (services["llm"]["running"].as_bool(), services["images"]["running"].as_bool()),
        (Some(true), Some(true))
    );
    assert_eq!(
        (services["images"]["app"].as_str(), services["llm"]["app"].as_str()),
        (Some("Comfy Desktop"), Some("MLX-Serve"))
    );
    assert_eq!(services["llm"]["memory"], 1000);

    // While the LLM holds the memory, prompts are paused.
    let (status, body) = h.post("/api/generate", json!({"prompt": "x"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body["error"].as_str().unwrap().contains("big-llm"), "{body}");
    assert!(h.fake.lock().unwrap().prompts.is_empty());

    let services = h.switch(json!({"to": "images"})).await;
    assert_eq!(services["error"], Value::Null);
    assert_eq!(services["llm"]["models"], json!([]));
    assert_eq!(h.llm.lock().unwrap().unloads, ["org/big-llm"]);
    assert_eq!(h.fake.lock().unwrap().freed, 0, "ComfyUI keeps its models");
    assert!(h.opened.lock().unwrap().is_empty(), "both were already running");
    let id = h.generate("a red fox").await["id"].as_str().unwrap().to_string();

    // Switching back would stop that job: the app asks first, then retries with cancelJobs.
    let (status, refused) = h.post("/api/services/switch", json!({"to": "llm"})).await;
    assert_eq!((status, refused["activeJobs"].as_u64()), (StatusCode::CONFLICT, Some(1)));
    assert!(refused["error"].as_str().unwrap().contains("1 image job is"), "{refused}");

    h.llm.lock().unwrap().no_room = 2; // ComfyUI's memory takes a moment to free
    let services = h.switch(json!({"to": "llm", "cancelJobs": true})).await;
    assert_eq!(services["error"], Value::Null);
    assert_eq!(h.state_of(&id).await["state"], "cancelled");
    assert_eq!(h.fake.lock().unwrap().freed, 1);
    assert_eq!(
        h.llm.lock().unwrap().loads,
        ["org/big-llm"],
        "the model it unloaded comes back, not --llm-model"
    );
    assert_eq!(services["llm"]["models"], json!(["org/big-llm"]));

    let saved = std::fs::read_to_string(h._dir.path().join("services.json")).unwrap();
    assert!(saved.contains("org/big-llm"), "{saved}");
}

#[tokio::test]
async fn switching_opens_apps_that_are_not_running() {
    let h = harness().await;
    h.llm.lock().unwrap().down = true;
    let (_, services) = h.get("/api/services").await;
    assert_eq!(
        (services["llm"]["running"].as_bool(), services["llm"]["memory"].as_u64()),
        (Some(false), None)
    );

    // Nothing was unloaded before, so --llm-model loads.
    let services = h.switch(json!({"to": "llm"})).await;
    assert_eq!(services["error"], Value::Null);
    assert_eq!(*h.opened.lock().unwrap(), [PathBuf::from(LLM_APP)]);
    assert_eq!(h.llm.lock().unwrap().loads, ["org/default"]);
    assert_eq!(h.fake.lock().unwrap().freed, 1);

    h.fake.lock().unwrap().down = true;
    let services = h.switch(json!({"to": "images"})).await;
    assert_eq!(services["error"], Value::Null);
    assert_eq!(*h.opened.lock().unwrap(), [PathBuf::from(LLM_APP), PathBuf::from(IMAGE_APP)]);
    assert_eq!(h.llm.lock().unwrap().unloads, ["org/default"]);
    assert_eq!(services["images"]["running"], true);
}

#[tokio::test]
async fn one_switch_at_a_time_and_failures_are_reported() {
    let h = harness().await;
    {
        let mut llm = h.llm.lock().unwrap();
        llm.down = true;
        llm.stays_down = true; // the app opens, but its server doesn't start
    }
    let (status, _) = h.post("/api/services/switch", json!({"to": "llm"})).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let (status, busy) = h.post("/api/services/switch", json!({"to": "images"})).await;
    assert_eq!(status, StatusCode::CONFLICT, "{busy}");
    let (status, paused) = h.post("/api/generate", json!({"prompt": "x"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(paused["error"].as_str().unwrap().contains("switching to the LLM"), "{paused}");

    let mut services = Value::Null;
    for _ in 0..200 {
        services = h.get("/api/services").await.1;
        if services["switching"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(services["error"].as_str().unwrap().contains("didn't start"), "{services}");
    assert!(h.llm.lock().unwrap().loads.is_empty());
    assert_eq!(h.fake.lock().unwrap().freed, 0, "ComfyUI keeps its models when the LLM can't start");

    // The next switch clears the error.
    let services = h.switch(json!({"to": "images"})).await;
    assert_eq!(services["error"], Value::Null);
}

#[tokio::test]
async fn open_apps_are_waited_for_not_reopened() {
    let h = harness().await;
    {
        let mut llm = h.llm.lock().unwrap();
        (llm.app_open, llm.down) = (true, true); // its server was stopped from the menu-bar icon
    }
    let (_, services) = h.get("/api/services").await;
    assert_eq!(
        (services["llm"]["running"].as_bool(), services["llm"]["appOpen"].as_bool()),
        (Some(false), Some(true))
    );
    let services = h.switch(json!({"to": "llm"})).await;
    assert!(services["error"].as_str().unwrap().contains("server is stopped"), "{services}");
    assert!(h.opened.lock().unwrap().is_empty(), "reopening MLX-Serve doesn't start its server");
    assert_eq!(h.fake.lock().unwrap().freed, 0);
    h.generate("images still work").await;

    // Comfy Desktop open with ComfyUI not answering: the LLM stays until ComfyUI does.
    h.llm.lock().unwrap().down = false;
    h.llm.lock().unwrap().loaded.push("org/big-llm".into());
    {
        let mut fake = h.fake.lock().unwrap();
        (fake.app_open, fake.down) = (true, true);
    }
    let services = h.switch(json!({"to": "images"})).await;
    assert!(services["error"].as_str().unwrap().contains("Comfy Desktop is open"), "{services}");
    assert!(h.opened.lock().unwrap().is_empty());
    assert!(
        h.llm.lock().unwrap().unloads.is_empty(),
        "the LLM stays while ComfyUI can't take over"
    );
}

#[tokio::test]
async fn switching_can_be_turned_off() {
    let h = build(None, false).await;
    assert_eq!(h.get("/api/info").await.1["services"], false);
    assert_eq!(h.get("/api/services").await.0, StatusCode::NOT_FOUND);
    assert_eq!(h.post("/api/services/switch", json!({"to": "llm"})).await.0, StatusCode::NOT_FOUND);
    h.llm.lock().unwrap().loaded.push("org/big-llm".into());
    h.generate("prompts are not paused for an LLM this server doesn't manage").await;
}

#[tokio::test]
async fn generate_and_edit_never_share_memory() {
    let h = harness().await;
    let (_, services) = h.get("/api/services").await;
    assert_eq!(
        (services["mode"].as_str(), &services["modes"]),
        (Some("generate"), &json!(["generate", "edit", "llm"]))
    );
    let (_, info) = h.get("/api/info").await;
    assert_eq!(info["workflows"]["edit"]["options"], json!([{"name": "Realism", "on": true}]));
    assert_eq!(info["workflows"]["generate"]["options"], json!([]));

    let (status, uploaded) = h.upload(PNG).await;
    assert_eq!(status, StatusCode::CREATED, "{uploaded}");
    let upload = uploaded["upload"].as_str().unwrap().to_string();
    assert!(upload.starts_with("prompt-app/") && upload.ends_with(".png"), "{upload}");

    // Edits wait for the switch: the generate model may still be in memory.
    let edit = json!({"prompt": "make it night", "kind": "edit", "source": {"upload": upload}, "options": {"Realism": false}});
    let (status, refused) = h.post("/api/generate", edit.clone()).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        refused["error"].as_str().unwrap().contains("set up for image generation"),
        "{refused}"
    );

    // Leaving generate would stop its job: the app asks, then cancels.
    let id = h.generate("a red fox").await["id"].as_str().unwrap().to_string();
    let (status, conflict) = h.post("/api/services/switch", json!({"to": "edit"})).await;
    assert_eq!((status, conflict["activeJobs"].as_u64()), (StatusCode::CONFLICT, Some(1)));
    let services = h.switch(json!({"to": "edit", "cancelJobs": true})).await;
    assert_eq!((services["mode"].as_str(), &services["error"]), (Some("edit"), &Value::Null));
    assert_eq!(h.state_of(&id).await["state"], "cancelled");
    assert_eq!(
        h.fake.lock().unwrap().freed,
        1,
        "the generate model goes before the edit model can load"
    );

    let (status, refused) = h.post("/api/generate", json!({"prompt": "x"})).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(refused["error"].as_str().unwrap().contains("Switch to Generate"), "{refused}");

    let (status, job) = h.post("/api/generate", edit).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    assert_eq!((job["kind"].as_str(), &job["options"]), (Some("edit"), &json!({"Realism": false})));
    {
        let fake = h.fake.lock().unwrap();
        let sent = &fake.prompts.last().unwrap()["prompt"];
        assert_eq!(sent["8"]["inputs"]["image"], upload.as_str());
        assert_eq!(sent["10"]["inputs"]["prompt"], "make it night");
        assert_eq!(sent["7"]["inputs"]["strength_model"], 0.0);
        assert_eq!(sent["15"]["inputs"]["seed"], job["seed"]);
    }
    let response = h
        .request(reqwest::Method::GET, job["source"].as_str().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.bytes().await.unwrap().as_ref(), PNG);

    let services = h.switch(json!({"to": "generate", "cancelJobs": true})).await;
    assert_eq!(services["mode"], "generate");
    assert_eq!(h.fake.lock().unwrap().freed, 2);
    let saved = std::fs::read_to_string(h._dir.path().join("services.json")).unwrap();
    assert!(saved.contains(r#""image_mode":"generate""#), "{saved}");

    // Already there: nothing to free.
    h.switch(json!({"to": "generate"})).await;
    assert_eq!(h.fake.lock().unwrap().freed, 2);
}

#[tokio::test]
async fn editing_a_finished_image_and_bad_sources() {
    let h = harness().await;
    let id = h.generate("a lighthouse").await["id"].as_str().unwrap().to_string();
    h.fake.lock().unwrap().finish(&id);
    h.jobs.reconcile().await;
    h.switch(json!({"to": "edit"})).await;

    let (status, job) = h
        .post(
            "/api/generate",
            json!({"prompt": "add a storm", "kind": "edit", "source": {"job": id, "index": 0}}),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{job}");
    {
        let fake = h.fake.lock().unwrap();
        let (copied, bytes) = fake.uploads.iter().next().unwrap();
        assert_eq!(bytes.as_slice(), PNG, "the result is copied into ComfyUI's input folder");
        assert_eq!(fake.prompts.last().unwrap()["prompt"]["8"]["inputs"]["image"], copied.as_str());
        assert_eq!(
            fake.prompts.last().unwrap()["prompt"]["7"]["inputs"]["strength_model"],
            0.85,
            "Realism is on by default"
        );
    }

    let edit = |source: Value| json!({"prompt": "x", "kind": "edit", "source": source});
    let missing = json!({"job": "00000000-0000-4000-8000-0000000000ff"});
    assert_eq!(h.post("/api/generate", edit(missing)).await.0, StatusCode::NOT_FOUND);
    for name in [
        "../../etc/passwd",
        "other/0b4f.png",
        "prompt-app/notauuid.png",
        "prompt-app/../x.png",
    ] {
        let (status, body) = h.post("/api/generate", edit(json!({"upload": name}))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{name}: {body}");
    }
    let (status, body) = h.post("/api/generate", json!({"prompt": "x", "kind": "edit"})).await;
    assert!(status == StatusCode::BAD_REQUEST && body["error"].as_str().unwrap().contains("Choose a picture"));
    assert_eq!(
        h.post("/api/generate", json!({"prompt": "x", "kind": "paint"})).await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        h.post(
            "/api/generate",
            json!({"prompt": "x", "kind": "edit", "source": {"upload": "a"}, "options": [1]})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );

    assert_eq!(h.upload(b"GIF89a not supported").await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let huge = [PNG, &vec![0u8; 31 * 1024 * 1024]].concat();
    assert_eq!(h.upload(&huge).await.0, StatusCode::PAYLOAD_TOO_LARGE);
    let big_but_fine = [PNG, &vec![0u8; 2 * 1024 * 1024]].concat();
    assert_eq!(
        h.upload(&big_but_fine).await.0,
        StatusCode::CREATED,
        "uploads may exceed the 256 KB JSON limit"
    );
}

#[tokio::test]
async fn the_llm_loaded_by_another_app_frees_comfyui() {
    let h = harness().await;
    let id = h.generate("a red fox").await["id"].as_str().unwrap().to_string();
    tokio::spawn(h.services.clone().unwrap().watch());

    // Hermes asks MLX-Serve for the LLM while an image job waits for ComfyUI.
    h.llm.lock().unwrap().loaded.push("org/big-llm".into());
    let mut services = Value::Null;
    for _ in 0..200 {
        services = h.get("/api/services").await.1;
        if !services["notice"].is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let notice = services["notice"].as_str().unwrap();
    assert!(notice.contains("big-llm") && notice.contains("cancelled 1 image job"), "{notice}");
    assert_eq!(services["mode"], "llm");
    assert_eq!(h.state_of(&id).await["state"], "cancelled");
    assert_eq!(h.fake.lock().unwrap().freed, 1);

    // Once per load: ComfyUI isn't freed again while the LLM stays.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(h.fake.lock().unwrap().freed, 1);

    // A switch clears the notice.
    let services = h.switch(json!({"to": "generate"})).await;
    assert_eq!((services["mode"].as_str(), &services["notice"]), (Some("generate"), &Value::Null));
}
