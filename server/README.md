# ComfyUI prompt server

Fixed ComfyUI workflows, one input each: the prompt, plus the picture for an edit. This server takes prompts from the iPhone app ([`app/`](../app)) or its own web page and hands back the images. Every other setting (model, LoRA, size, steps, CFG, sampler, negative prompt) stays as the workflow has it. The server is a single Rust binary for your Mac. It's built for Apple Silicon and uses about 7 MB of memory.

There are two workflows:

- **Generate:** yours, exported from ComfyUI (for example Qwen-Image 2512).
- **Edit:** [`workflows/qwen_image_edit_2511_api.json`](workflows/qwen_image_edit_2511_api.json), Qwen-Image-Edit 2511 with the 4-step Lightning LoRA and an optional realism LoRA. It is built from ComfyUI's official 2511 template.

```
iPhone app / browser on your tailnet
        │  https://<mac-name>.<tailnet>.ts.net[:8443]   (tailscale serve: real HTTPS, tailnet devices only;
        │                                                 :8443 when 443 already publishes another server)
        ▼
comfyui-prompt-server on 127.0.0.1:8189       (the password on every request; accepts a prompt, nothing else)
        │                                 ╲
        ▼                                  ╲  only to unload and load the LLM, never to chat with it
ComfyUI on 127.0.0.1:8000 (ComfyUI Desktop) or :8188      MLX-Serve on 127.0.0.1:11234
```

ComfyUI and this server both listen on 127.0.0.1 only, so nothing is open on your network. Keep ComfyUI that way: with `--listen` (or Desktop's Listen Address set to 0.0.0.0), anyone who can reach it can open the full editor and change the workflow.

## Setup on the Mac

1. **Install Rust** once, from [rustup.rs](https://rustup.rs) or with `brew install rust`.
2. **Export the workflow in API format.** Open your workflow (e.g. *Qwen-Image 2512*) in ComfyUI and choose **File → Export (API)**. Save the file into this folder. Its name becomes the page title. Plain **Export** writes the editor format, which the server refuses with a hint.
3. **For editing, put the edit model files in ComfyUI's model folders.** The text encoder and VAE are the ones Qwen-Image already uses.

   | File | Folder | From |
   |---|---|---|
   | `qwen_image_edit_2511_bf16.safetensors` (41 GB) | `diffusion_models` | [Comfy-Org/Qwen-Image-Edit_ComfyUI](https://huggingface.co/Comfy-Org/Qwen-Image-Edit_ComfyUI/tree/main/split_files/diffusion_models) |
   | `Qwen-Image-Edit-2511-Lightning-4steps-V1.0-bf16.safetensors` | `loras` | [lightx2v/Qwen-Image-Edit-2511-Lightning](https://huggingface.co/lightx2v/Qwen-Image-Edit-2511-Lightning) |
   | `anything2real_2601_A_final_patched.safetensors` | `loras` | [lrzjason/Anything2Real](https://huggingface.co/lrzjason/Anything2Real) |
   | `qwen_2.5_vl_7b.safetensors`, `qwen_image_vae.safetensors` | `text_encoders`, `vae` | [Comfy-Org/Qwen-Image_ComfyUI](https://huggingface.co/Comfy-Org/Qwen-Image_ComfyUI/tree/main/split_files) |

   Without these files, editing is still offered, but each edit fails with ComfyUI's "Value not in list". `--no-edit` hides it.
4. **Tailscale** must be running and signed in on the Mac and on the iPhone. The first time, `tailscale serve` may print a link to enable HTTPS on your tailnet. Open it, approve, and start again.
5. **Start ComfyUI**, then run `./start.sh` in Terminal. The first run builds the server (about a minute) and asks you to choose the password, which is at least 10 characters (a few random words work well).

Read the startup lines once:

```
  Qwen-Image 2512
  Workflow  ./Qwen-Image 2512.json (19 nodes)
  Prompt    goes into node 238:227 "CLIP Text Encode (Positive Prompt)" (CLIPTextEncode), input 'text'
  Edit      workflows/qwen_image_edit_2511_api.json (17 nodes)
            instruction into node 10 "Edit instruction (positive)" (TextEncodeQwenImageEditPlus), input 'prompt'
            picture into node 8 "Picture to edit" (LoadImage)
            switches in the app: Realism (on)
  Seed      new random seed every run (238:230.seed)
  ComfyUI   http://127.0.0.1:8188 (answering)
  Switch    Generate ⇄ Edit ⇄ LLM (now Generate); MLX-Serve at http://127.0.0.1:11234 (no model loaded)
  Keeping   https port 443 → http://127.0.0.1:8787 (another server)
  Running   /Applications/Tailscale.app/Contents/MacOS/Tailscale serve --bg --https=8443 http://127.0.0.1:8189
  Access    devices on your tailnet, over HTTPS; password required
  Open      https://my-mac.tail1a2b3c.ts.net:8443  (from your Tailscale devices; also the app's server address)
```

- **Prompt** must name your positive prompt node. The server finds it by following the sampler's `positive` input back to its text. If it's wrong, start with `--prompt-node <id>`.
- **Edit** names the same for the edit workflow, plus the Load Image node that takes the picture.
- **Keeping** lists servers the Mac already publishes with Tailscale, such as another app's web page. Their ports are left as they are, so this server takes the next free one.
- **Open** gives the address you enter in the iPhone app, once, together with the password. A browser asks for a user name (type anything) and the password, and remembers them.

## The password

You set it once on the Mac; every device stores it once.

- **Storage:** the Mac keeps only a salted PBKDF2-SHA256 hash (600,000 rounds) in `data/auth.json`, readable only by your user.
- **Where devices keep it:** the app in the iPhone Keychain, a browser in its password store.
- **Sending:** every request carries it over Tailscale's HTTPS. There are no sessions or logins to expire.
- **Wrong guesses:** a device that sends 5 wrong passwords within 15 minutes is refused until the 15 minutes pass. Other devices are not affected.
- **Changing it:** run `./target/release/comfyui-prompt-server --set-password`. It takes effect at once, even while the server runs. Then enter the new one in the app's connection settings and in each browser.

## Using it

- Each run gets a new random seed, shown with the result, so the same prompt gives a new image. Start with `--keep-seed` to always use the workflow's saved seed.
- If the workflow's positive prompt text contains `{prompt}`, only that part is replaced. That keeps fixed text around it, e.g. `RAW photo, {prompt}, 85mm`.
- **History lives on the Mac** (`data/jobs.json`), so the app and every browser show the same list. Images stay downloadable after ComfyUI restarts. Removing a job from the history leaves its file in ComfyUI's `output` folder.
- Like every ComfyUI image, a downloaded PNG carries the full prompt and settings in its metadata.

## Editing

An edit takes a picture and an instruction, such as "make it night" or "remove the people in the back".

- **The picture** is one sent from the iPhone (`POST /api/uploads`) or an image already in the history. Either way it's copied into ComfyUI's `input/prompt-app/` folder, so the edit keeps its source even if the job it came from is removed.
- **Switches:** a LoRA node titled `Option: <name>` in a workflow becomes a switch in the app. Off sets its strength to 0, and on keeps the workflow's strength. The edit workflow has one, **Realism** (Anything2Real at 0.85). It turns illustrations, renders and stylized pictures into photographs, and works best when the instruction starts with "change the picture 1 to realistic photograph". The app offers to insert those words. Add your own `Option:` nodes to either workflow the same way.
- **Speed:** 4 steps with the Lightning LoRA. The picture is scaled to about one megapixel first, as the model expects.

## Modes: Generate, Edit and the LLM

The image model (about 60 GB in use), the edit model (about 60 GB) and a large LLM (75 GB or more) don't fit in memory together. When the edit workflow is set up, or MLX-Serve runs on the Mac too, the server keeps the Mac in one mode at a time. The app shows the modes and switches between them. The app never sends the LLM a prompt: chatting stays with whatever already uses MLX-Serve, such as Hermes.

- **One heavy model at a time.** A prompt for a mode that isn't on is refused with a message that says which mode to switch to. A switch frees the current model and waits until ComfyUI's memory is steady before the next model can load.
- **Between Generate and Edit:**
  1. Queued and running image jobs are cancelled. The app asks first.
  2. ComfyUI unloads its models and node cache.
  3. The next prompt loads the other model.
- **To Generate or Edit from the LLM:**
  1. If ComfyUI isn't running, Comfy Desktop is opened. ComfyUI loads its models with the first prompt, so it can start beside the LLM.
  2. MLX-Serve unloads every model it holds, and the server remembers which ones.
- **To the LLM** (the way to free the Mac for Hermes):
  1. If MLX-Serve's server isn't running, its app is opened. Its server holds no model until asked, so this comes first.
  2. Queued and running image jobs are cancelled. The app asks first.
  3. ComfyUI unloads its models and node cache, like its "Unload Models and Execution Cache" button.
  4. The models unloaded before are loaded again. If there are none, `--llm-model` loads.
- **A switch that can't finish changes nothing.** Each side is checked before the other lets go of its memory. If MLX-Serve can't serve, ComfyUI keeps its models, and the other way around.
- **MLX-Serve stopped from its menu-bar icon stays stopped.** MLX-Serve starts its server when the app opens. Once the server is stopped from the menu, only that menu starts it again, because reopening the app doesn't. The app then shows "Server stopped", and a switch to the LLM says to start it there.
- **Nothing is quit.** Both apps stay open with less memory in use. Other servers and their Tailscale addresses aren't touched.
- **While the LLM holds a model,** or a switch runs, new prompts are refused with a message that says why.
- **When Hermes loads the LLM by itself.** MLX-Serve loads models on demand, so a message from Hermes while Generate or Edit is on loads the LLM again. The server checks every 5 seconds. When it sees the LLM loaded while ComfyUI still holds a model, it cancels any image jobs and frees ComfyUI. The app then says so, and images stay paused until you choose Generate or Edit. Both models are in memory together only for those few seconds. MLX-Serve also refuses to load when memory is short.

`--no-switch` turns this off, and editing with it. The mode and the unloaded models are remembered in `data/services.json`.

## Start at login

Once the password is set, `./deploy.sh` builds the server and installs a LaunchAgent for it, `~/Library/LaunchAgents/local.comfyui-prompt-server.plist`. The server then starts when you log in and restarts if it stops. It runs with `--tailscale` from this folder and logs to `/tmp/comfyui-prompt-server.log`. Run `./deploy.sh` again after pulling changes. It rebuilds, restarts the server, and prints the startup lines.

- **The LLM to load** when switching to it, until a switch has unloaded one: `PROMPT_SERVER_LLM_MODEL=ddalcu/Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit ./deploy.sh`. Later runs keep it.
- **To stop it:** run `launchctl bootout gui/$(id -u)/local.comfyui-prompt-server`.
- **ComfyUI has to start on its own too.** ComfyUI Desktop can be added under System Settings → General → Login Items. A switch to Generate or Edit also opens it.

## Who can reach it

- **Default:** devices on your tailnet, through `tailscale serve`. This is Serve, not Funnel, so nothing is published to the internet, and your tailnet's access rules apply.
- **Unpublish:** `tailscale serve --https=<port> off`, with the port the startup output names. The setting survives restarts; while the server is stopped, the address simply doesn't answer.
- **Other servers on the Mac:** `--tailscale` never takes over a port that already publishes something else. It reuses the port this server had before, so the address stays the same. Otherwise it takes the first free one of 443, 8443 and 10000. `--tailscale 9443` picks the port yourself; a taken one is refused.
- **Plain LAN instead:** `--host 0.0.0.0`, without `--tailscale`. Traffic is then unencrypted HTTP, so the password travels in the clear on your network.

## Options

| Option | Variable | Default |
|---|---|---|
| `--workflow PATH` | `PROMPT_SERVER_WORKFLOW` | `workflow_api.json`, or the only `.json` file in the folder it runs in |
| `--comfy URL` | `COMFY_URL` | the first of `http://127.0.0.1:8188` and `:8000` that answers |
| `--tailscale [PORT]` | `PROMPT_SERVER_TAILSCALE` | off (`start.sh` turns it on: its earlier port, else the first free of 443, 8443, 10000) |
| `--host`, `--port` | `PROMPT_SERVER_HOST`, `PROMPT_SERVER_PORT` | `127.0.0.1`, `8189` |
| `--data-dir` | `PROMPT_SERVER_DATA` | `data` |
| `--set-password` | | asked on first start |
| `--prompt-node ID`, `--prompt-input NAME` | `PROMPT_SERVER_PROMPT_NODE`, `PROMPT_SERVER_PROMPT_INPUT` | found automatically |
| `--keep-seed` | | new random seed every run |
| `--title` | `PROMPT_SERVER_TITLE` | the workflow file name |
| `--llm-url URL` | `PROMPT_SERVER_LLM_URL` | `http://127.0.0.1:11234` (MLX-Serve) |
| `--llm-app`, `--image-app PATH` | `PROMPT_SERVER_LLM_APP`, `PROMPT_SERVER_IMAGE_APP` | `/Applications/MLX-Serve.app`, `/Applications/Comfy Desktop.app`, if present |
| `--llm-model ID` | `PROMPT_SERVER_LLM_MODEL` | none: the models the last switch unloaded |
| `--edit-workflow PATH` | `PROMPT_SERVER_EDIT_WORKFLOW` | `workflows/qwen_image_edit_2511_api.json` |
| `--no-edit` | | editing on when the edit workflow is there |
| `--no-switch` | | switching on (it also turns editing off) |

## API

Every route except `/health` needs `Authorization: Bearer <password>` (or Basic, from browsers). Paths in responses are relative to the server address.

| Route | |
|---|---|
| `GET /health` | `{"status": "ok", "service": "comfyui-prompt-server", "version"}`, no password, for connection tests |
| `GET /api/info` | the workflow title, `workflows {generate, edit}` (each `{title, options [{name, on}]}`, `edit` null when off), whether ComfyUI answers, seed mode, `maxPromptChars`, `services` (switching is on) |
| `GET /api/services` | `mode` (`generate`, `edit` or `llm`), `modes`, `switching {to, step, since}` or null, the last switch's `error`, a `notice` about something the server did by itself, and `images`/`llm`: `{name, app, running, appOpen, memory, models}` |
| `POST /api/services/switch` `{"to": "generate" \| "edit" \| "llm", "cancelJobs"}` | starts the switch; `202`. `409` while one runs, or with `activeJobs` when the switch would stop image jobs |
| `POST /api/uploads` (the picture as the body) | keeps a PNG, JPEG or WebP up to 30 MB for an edit; `201 {"upload"}` |
| `POST /api/generate` `{"prompt", "kind", "source", "options"}` | queues it; `202` with the job. `kind` is `generate` (the default) or `edit`. An edit needs `source`: `{"upload"}` or `{"job", "index"}`. `options` sets switches by name, e.g. `{"Realism": false}`. `503` while another mode is on or a switch runs |
| `GET /api/jobs?limit=30&before=<id>` | history, newest first: `{"jobs": [...], "more"}` |
| `GET /api/jobs/<id>` | one job: `kind`, `state` (`queued`, `running`, `done`, `error`, `cancelled`, `lost`), `ahead`, `progress {value, max}`, `seed`, `seconds`, `error`, `options`, `source` (an edit's picture), `images [{filename, url, preview, download}]` |
| `GET /api/jobs/<id>/source` | the picture an edit started from |
| `POST /api/jobs/<id>/cancel` | takes it out of the queue, or stops it |
| `DELETE /api/jobs/<id>` | removes it from the history |
| `GET <image url>` | the PNG; `?preview=1` a lighter WebP, `?download=1` as an attachment |

Job status is answered from memory. A background task keeps it current from ComfyUI's websocket and queue, so frequent status requests never wait on ComfyUI. Images stream straight through without being buffered.

## Troubleshooting

- **"is the editor format"**: you used **Export**. Use **File → Export (API)**.
- **"ComfyUI NOT answering"**: start ComfyUI, or pass `--comfy http://127.0.0.1:<port>`.
- **An error on a job**: that's ComfyUI's own message, e.g. a missing model or custom node.
- **"Tailscale did not publish the page"**: HTTPS isn't enabled on the tailnet yet. Open the link Tailscale printed, approve, and start again.
- **"No password is set yet"** at login: run `./start.sh` once in Terminal to choose one.
- **"HTTPS port 443 already publishes …"**: another server uses that port. Leave the port out (`--tailscale`) to take a free one.
- **"MLX-Serve is open, but its server is stopped"** or **"MLX-Serve opened, but its server didn't start"**: start the server from MLX-Serve's menu-bar icon, then switch again.

## Development

```
cargo test             # unit tests and the API against a fake ComfyUI and a fake MLX-Serve
cargo clippy --all-targets
```

The web page generates only; while Edit or the LLM is on, it shows the server's "switch first" message.

`tests/fixtures/qwen_image_2512_api.json` is ComfyUI's official Qwen-Image 2512 template, exported by ComfyUI's own frontend. It shows how a subgraph flattens into ids like `238:227`.
