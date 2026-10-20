# comfy

Prompt: generate and edit images with ComfyUI on your Mac, from your iPhone, over Tailscale.

```
iPhone: Prompt app (Expo)  ──  https://<mac>.<tailnet>.ts.net:8443  ──▶  server/ on the Mac  ──▶  ComfyUI
                                                                                      ╲
                                                                                       ▶  MLX-Serve (only to free or load the LLM)
```

- **[`server/`](server)** is a small Rust server for your Mac. It holds fixed ComfyUI workflows and keeps the shared history. It also switches the Mac between three modes, so only one large model is in memory at a time:
  - **Generate** makes images from a prompt. It uses your workflow, for example Qwen-Image 2512.
  - **Edit** changes a picture following an instruction. It uses Qwen-Image-Edit 2511 with the 4-step Lightning LoRA and an optional realism LoRA.
  - **LLM** frees ComfyUI so MLX-Serve can hold the LLM, for Hermes.
- **[`app/`](app)** is the iPhone app. It runs in Expo Go, or as its own app built with EAS. You type a prompt, or pick a picture and say what to change, then save or share the result.

Each folder's README covers setup. In short:

1. Run `server/start.sh` on the Mac once. It builds the server and asks for the password.
2. Run `server/deploy.sh` to keep the server running at login.
3. Open the app in Expo Go and enter the address the server printed, plus the password.

## CI/CD

| Workflow | When | What |
|---|---|---|
| [CI](.github/workflows/ci.yml) | every push and pull request | **Server** (macOS): `cargo fmt --check`, `clippy -D warnings`, `cargo test`. **App**: typecheck, lint, tests, `expo-doctor`, and an iOS bundle. |
| CI → **Publish the app** | a push to `main` that changes `app/`, after both checks pass | `eas update` to the `production` channel, so Expo Go and installed builds get the new app. |
| [Release](.github/workflows/release.yml) | a tag `v*` matching `server/Cargo.toml` | Tests and builds the server for macOS on Apple Silicon, and attaches it to a GitHub release. |
| [Dependabot](.github/dependabot.yml) | weekly | Grouped updates for Cargo, npm and the actions. Expo SDK packages are left to `npx expo install --fix`. |

**Publishing needs an Expo token.** Create one at [expo.dev → Access tokens](https://expo.dev/settings/access-tokens) for the account that owns the project. Add it as the repository secret `EXPO_TOKEN`, under Settings → Secrets and variables → Actions. Until it's there, the publish job skips with a notice.

**The server deploys on the Mac itself.** GitHub can't reach it, and this repository is public, so it has no self-hosted runner. After merging, pull and run `server/deploy.sh`. It rebuilds the server and restarts it.

To make a release, bump `version` in `server/Cargo.toml`, merge, then run:

```sh
git tag v2.1.0
git push origin v2.1.0
```

## Development

```sh
cd server && cargo test && cargo clippy --all-targets
cd app && npm ci && npm test && npm run typecheck && npm run lint
```

The exported workflow, the password hash and the job history stay on the Mac (`server/.gitignore`).
