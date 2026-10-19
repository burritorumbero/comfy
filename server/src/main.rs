//! A private page and API for one fixed ComfyUI workflow: send a prompt, get the image.
//!
//! Every setting except the positive prompt comes from the exported workflow, and nothing
//! talks to ComfyUI but this server, so ComfyUI stays on 127.0.0.1 where nobody can change
//! the workflow. This server also listens on 127.0.0.1 only; `--tailscale` publishes it to
//! your tailnet over HTTPS with `tailscale serve`.

use std::io::{BufRead, IsTerminal};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use comfyui_prompt_server::auth::{Auth, MIN_PASSWORD_CHARS};
use comfyui_prompt_server::comfy::{self, Comfy};
use comfyui_prompt_server::http::{self, App};
use comfyui_prompt_server::jobs::Jobs;
use comfyui_prompt_server::llm::{self, Llm};
use comfyui_prompt_server::services::{self, Services};
use comfyui_prompt_server::tailscale;
use comfyui_prompt_server::workflow::{self, Kind, Workflow};

const DEFAULT_PORT: u16 = 8189;
/// The Qwen-Image-Edit-2511 workflow kept with this server.
const DEFAULT_EDIT_WORKFLOW: &str = "workflows/qwen_image_edit_2511_api.json";

#[derive(Parser)]
#[command(
    version,
    // start.sh passes --tailscale; one given after it wins.
    args_override_self = true,
    about = "Serve a one-box prompt page and API for a fixed ComfyUI workflow, privately over Tailscale."
)]
struct Args {
    /// API-format workflow (File > Export (API) in ComfyUI). Default: workflow_api.json, or the only .json file, in this folder.
    #[arg(long, env = "PROMPT_SERVER_WORKFLOW")]
    workflow: Option<PathBuf>,
    /// API-format workflow that edits a picture: it needs a Load Image node. Default: workflows/qwen_image_edit_2511_api.json if present.
    #[arg(long, env = "PROMPT_SERVER_EDIT_WORKFLOW")]
    edit_workflow: Option<PathBuf>,
    /// Don't offer editing pictures.
    #[arg(long)]
    no_edit: bool,
    /// ComfyUI address. Default: the first of http://127.0.0.1:8188 and http://127.0.0.1:8000 that answers.
    #[arg(long, env = "COMFY_URL")]
    comfy: Option<String>,
    /// Publish the page to your tailnet with tailscale serve: on the HTTPS port given, or else the
    /// first of 443, 8443 and 10000 that publishes nothing else. Other servers' ports are left alone.
    #[arg(long, env = "PROMPT_SERVER_TAILSCALE", value_name = "HTTPS_PORT", num_args = 0..=1)]
    tailscale: Option<Option<u16>>,
    /// MLX-Serve's address. The app switches the Mac between image generation and this LLM server.
    #[arg(long, env = "PROMPT_SERVER_LLM_URL", default_value = llm::DEFAULT_URL)]
    llm_url: String,
    /// The LLM server's app, opened when switching to the LLM finds it closed. Default: /Applications/MLX-Serve.app if present.
    #[arg(long, env = "PROMPT_SERVER_LLM_APP")]
    llm_app: Option<PathBuf>,
    /// ComfyUI's app, opened when switching to images finds ComfyUI closed. Default: /Applications/Comfy Desktop.app if present.
    #[arg(long, env = "PROMPT_SERVER_IMAGE_APP")]
    image_app: Option<PathBuf>,
    /// LLM model to load when switching to the LLM, if no switch unloaded one before (e.g. the model Hermes uses).
    #[arg(long, env = "PROMPT_SERVER_LLM_MODEL")]
    llm_model: Option<String>,
    /// Don't offer switching between image generation and the LLM.
    #[arg(long)]
    no_switch: bool,
    /// Address to listen on. 127.0.0.1 (this computer) is all --tailscale needs; 0.0.0.0 opens the page to your whole LAN.
    #[arg(long, env = "PROMPT_SERVER_HOST", default_value = "127.0.0.1")]
    host: IpAddr,
    #[arg(long, env = "PROMPT_SERVER_PORT", default_value_t = DEFAULT_PORT)]
    port: u16,
    /// Where the password hash and the job history live.
    #[arg(long, env = "PROMPT_SERVER_DATA", default_value = "data")]
    data_dir: PathBuf,
    /// Choose a new password for the page and the app, and exit. Devices using the old one stop working until you enter the new one there.
    #[arg(long)]
    set_password: bool,
    /// Id of the node that takes the prompt, if the automatic choice is wrong.
    #[arg(long, env = "PROMPT_SERVER_PROMPT_NODE")]
    prompt_node: Option<String>,
    /// Input on --prompt-node that takes the prompt (default: its text input).
    #[arg(long, env = "PROMPT_SERVER_PROMPT_INPUT")]
    prompt_input: Option<String>,
    /// Use the seed saved in the workflow every time, so the same prompt gives the same image.
    #[arg(long)]
    keep_seed: bool,
    /// Page title. Default: the workflow file name.
    #[arg(long, env = "PROMPT_SERVER_TITLE")]
    title: Option<String>,
    /// Log more detail.
    #[arg(long)]
    verbose: bool,
}

/// Ask for a new password twice. Reads one line instead when stdin is not a terminal
/// (`echo ... | comfyui-prompt-server --set-password`).
fn ask_password() -> Option<String> {
    if !std::io::stdin().is_terminal() {
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line).ok()?;
        let line = line.trim_end_matches(['\r', '\n']).to_string();
        return (line.chars().count() >= MIN_PASSWORD_CHARS).then_some(line);
    }
    for _ in 0..3 {
        let first = rpassword::prompt_password(format!(
            "  New password for the page and the app ({MIN_PASSWORD_CHARS}+ characters; a few random words work well): "
        ))
        .ok()?;
        if first.chars().count() < MIN_PASSWORD_CHARS {
            println!("  Too short: use at least {MIN_PASSWORD_CHARS} characters.");
            continue;
        }
        if rpassword::prompt_password("  Same password again: ").ok()? != first {
            println!("  The two did not match.");
            continue;
        }
        return Some(first);
    }
    None
}

fn lan_addresses() -> Vec<IpAddr> {
    // Connecting a UDP socket sends nothing; it only picks the outgoing interface.
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("10.255.255.255:1").and_then(|_| s.local_addr()))
        .map(|a| a.ip())
        .into_iter()
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect()
}

/// The workflow's switchable LoRAs, if it has any.
fn print_options(workflow: &Workflow) {
    if workflow.options.is_empty() {
        return;
    }
    let options: Vec<String> = workflow
        .options
        .iter()
        .map(|o| format!("{} ({})", o.name, if o.on { "on" } else { "off" }))
        .collect();
    println!("            switches in the app: {}", options.join(", "));
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}

fn main() -> std::process::ExitCode {
    let args = Args::parse();
    tracing_subscriber::fmt()
        .with_max_level(if args.verbose {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .with_target(false)
        .with_ansi(std::io::stdout().is_terminal())
        .init();

    let auth = Arc::new(Auth::open(Some(args.data_dir.join("auth.json"))));
    if args.set_password || !auth.has_password() {
        if !args.set_password {
            // Started without a terminal (e.g. at login): nobody is there to type a password.
            if !std::io::stdin().is_terminal() {
                eprintln!("No password is set yet. Run this once in Terminal first: ./start.sh, or comfyui-prompt-server --set-password");
                return std::process::ExitCode::from(2);
            }
            println!("\n  The page and the app need a password before the server starts.");
        }
        let Some(password) = ask_password() else {
            eprintln!("No password set. Run again and enter at least {MIN_PASSWORD_CHARS} characters, twice.");
            return std::process::ExitCode::from(2);
        };
        if let Err(e) = auth.set_password(&password) {
            eprintln!("{e}");
            return std::process::ExitCode::from(2);
        }
        println!("  Password saved. Enter it once in the app's connection settings and in each browser.");
        if args.set_password {
            return std::process::ExitCode::SUCCESS;
        }
    }

    let workflow = match workflow::resolve_path(args.workflow.clone(), std::path::Path::new(".")).and_then(|path| {
        Workflow::open(
            &path,
            Kind::Generate,
            args.prompt_node.as_deref(),
            args.prompt_input.as_deref(),
            args.keep_seed,
            args.title.clone(),
        )
    }) {
        Ok(workflow) => Arc::new(workflow),
        Err(e) => {
            eprintln!("Workflow problem: {e}");
            return std::process::ExitCode::from(2);
        }
    };

    // Editing keeps the two image models apart with the switch, so it needs switching on.
    let edit_path = args
        .edit_workflow
        .clone()
        .or_else(|| Some(PathBuf::from(DEFAULT_EDIT_WORKFLOW)).filter(|p| p.exists()))
        .filter(|_| !args.no_edit && !args.no_switch);
    let edit = match edit_path.map(|path| Workflow::open(&path, Kind::Edit, None, None, args.keep_seed, None)) {
        None => None,
        Some(Ok(edit)) => Some(Arc::new(edit)),
        Some(Err(e)) => {
            eprintln!("Edit workflow problem: {e}");
            return std::process::ExitCode::from(2);
        }
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(serve(args, auth, workflow, edit))
}

async fn serve(args: Args, auth: Arc<Auth>, workflow: Arc<Workflow>, edit: Option<Arc<Workflow>>) -> std::process::ExitCode {
    let comfy_url = match args.comfy.clone() {
        Some(url) => url,
        None => comfy::detect(comfy::CANDIDATES)
            .await
            .unwrap_or_else(|| comfy::CANDIDATES[0].to_string()),
    };
    let comfy = Comfy::new(&comfy_url);
    let reachable = comfy.reachable().await;

    let mut client_id = [0u8; 16];
    getrandom::fill(&mut client_id).expect("randomness");
    let client_id: String = client_id.iter().map(|b| format!("{b:02x}")).collect();
    let jobs = Arc::new(Jobs::open(comfy.clone(), client_id, Some(args.data_dir.join("jobs.json"))));
    jobs.set_comfy_ok(reachable);
    tokio::spawn(jobs.clone().track());
    tokio::spawn(jobs.clone().follow());

    let app_or_default = |given: Option<PathBuf>, default: &str| given.or_else(|| Some(PathBuf::from(default)).filter(|p| p.exists()));
    let services = (!args.no_switch).then(|| {
        Arc::new(Services::new(
            comfy.clone(),
            Llm::new(&args.llm_url),
            jobs.clone(),
            services::Config {
                image_app: app_or_default(args.image_app.clone(), services::DEFAULT_IMAGE_APP),
                llm_app: app_or_default(args.llm_app.clone(), services::DEFAULT_LLM_APP),
                llm_model: args.llm_model.clone(),
                path: Some(args.data_dir.join("services.json")),
                edit: edit.is_some(),
                busy_bytes: services::DEFAULT_BUSY_BYTES,
                apps: services::Apps::system(),
                waits: services::Waits::default(),
            },
        ))
    });
    if let Some(services) = &services {
        tokio::spawn(services.clone().watch());
    }

    let address = SocketAddr::new(args.host, args.port);
    let listener = match tokio::net::TcpListener::bind(address).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("Cannot listen on {address} ({e}). Is something else using the port? Try --port.");
            return std::process::ExitCode::from(1);
        }
    };
    let everywhere = args.host.is_unspecified();
    let local = if everywhere { IpAddr::from([127, 0, 0, 1]) } else { args.host };
    let local_url = format!("http://{}", SocketAddr::new(local, args.port));

    let graph = &workflow.graph;
    println!("\n  {}", workflow.title);
    println!("  Workflow  {} ({} nodes)", workflow.path.display(), graph.len());
    for (id, name) in &workflow.prompt_targets {
        println!("  Prompt    goes into node {}, input '{name}'", workflow::node_label(graph, id));
    }
    print_options(&workflow);
    match &edit {
        None if args.no_edit || args.no_switch => println!("  Edit      off"),
        None => println!("  Edit      off (no {DEFAULT_EDIT_WORKFLOW}; pass --edit-workflow)"),
        Some(edit) => {
            println!("  Edit      {} ({} nodes)", edit.path.display(), edit.graph.len());
            for (id, name) in &edit.prompt_targets {
                println!(
                    "            instruction into node {}, input '{name}'",
                    workflow::node_label(&edit.graph, id)
                );
            }
            for (id, _) in &edit.image_targets {
                println!("            picture into node {}", workflow::node_label(&edit.graph, id));
            }
            print_options(edit);
        }
    }
    if workflow.seed_targets.is_empty() {
        println!("  Seed      as saved in the workflow");
    } else {
        let places: Vec<String> = workflow.seed_targets.iter().map(|(id, name)| format!("{id}.{name}")).collect();
        println!("  Seed      new random seed every run ({})", places.join(", "));
    }
    println!(
        "  ComfyUI   {comfy_url} ({})",
        if reachable {
            "answering"
        } else {
            "NOT answering yet; start ComfyUI or pass --comfy"
        }
    );
    match &services {
        None => println!("  Switch    off (--no-switch)"),
        Some(services) => {
            let view = services.view().await;
            let llm = match (view.llm.running, view.llm.models.as_deref().unwrap_or_default()) {
                (false, _) => "not running".to_string(),
                (true, []) => "no model loaded".to_string(),
                (true, models) => format!(
                    "{} loaded",
                    models.iter().map(|m| services::short_name(m)).collect::<Vec<_>>().join(", ")
                ),
            };
            let modes: Vec<&str> = view.modes.iter().map(|m| m.label()).collect();
            println!(
                "  Switch    {} (now {}); MLX-Serve at {} ({llm})",
                modes.join(" ⇄ "),
                view.mode.label(),
                services.llm_url()
            );
        }
    }

    let (tailnet_url, tailnet_port) = match args.tailscale {
        None => (None, None),
        Some(requested) => {
            let target = format!("http://{}", SocketAddr::new(local, args.port));
            match tokio::task::spawn_blocking(move || tailscale::serve(&target, requested))
                .await
                .expect("tailscale task")
            {
                Ok((url, port)) => (Some(url), Some(port)),
                Err(e) => {
                    eprintln!("  Problem   {e}");
                    return std::process::ExitCode::from(1);
                }
            }
        }
    };

    let reach = if tailnet_url.is_some() {
        "devices on your tailnet, over HTTPS"
    } else if everywhere {
        "your whole network, over plain HTTP: the password is not encrypted on the way"
    } else {
        "this computer only"
    };
    println!("  Access    {reach}; password required");
    if let Some(url) = &tailnet_url {
        println!("  Open      {url}  (from your Tailscale devices; also the app's server address)");
    }
    if everywhere {
        for ip in lan_addresses() {
            println!(
                "  Open      http://{}  (from other devices on your network)",
                SocketAddr::new(ip, args.port)
            );
        }
    }
    println!("  Open      {local_url}  (on this computer)");
    if let Some(port) = tailnet_port {
        println!("  Unpublish tailscale serve --https={port} off");
    }
    println!("  Stop with Ctrl+C\n");

    let app = Arc::new(App {
        workflow,
        edit,
        comfy,
        jobs,
        auth,
        services,
    });
    let service = http::router(app).into_make_service_with_connect_info::<SocketAddr>();
    if let Err(e) = axum::serve(listener, service).with_graceful_shutdown(shutdown_signal()).await {
        eprintln!("Server error: {e}");
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::SUCCESS
}
