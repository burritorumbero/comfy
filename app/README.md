# Prompt for iPhone

An Expo app for the [ComfyUI prompt server](../server) on your Mac. You type a prompt, or pick a picture and say how to change it. Then you watch the job, and save or share the image. The design follows [Hermex](https://github.com/uzairansaruzi/hermex): you bring your own server, and the phone is only the control surface. You connect once, the app is dark and dense, and the composer sits at the bottom.

- **Connect once.** Enter the server address and the password you chose on the Mac. The app tests the connection before saving it. It checks `/health` first, so "can't reach your Mac" isn't mistaken for "wrong password". Both are kept in the iPhone Keychain (this device only, after first unlock). There is no sign-in screen and nothing expires.
- **History lives on the Mac,** so the app and every browser show the same list. The newest page is cached on the phone, for a quick start and offline reading.
- **While a job runs:** the card shows its queue position, then the sampler's step count and a clock.
- **When it's done:** tap the image for full screen with pinch to zoom. You can save it to Photos (add-only access, so the app never sees your library) or share it. **Use this prompt again** puts a prompt back in the composer.
- **If the Mac's password changes,** a banner says so, and you enter the new one in Connection settings. The address is kept.
- **Generate · Edit · LLM:** the bar under the title sets what your Mac's memory is for. Only one heavy model is loaded at a time.
  - **Generate** makes new images from a prompt.
  - **Edit** takes a picture and an instruction such as "make it night". Tap the photo button to choose a picture from your library, or **Edit this image** (the wand) on any result. The picture stays after sending, so you can try other words on it. **Realism** switches the Anything2Real LoRA. It works best with "change the picture 1 to realistic photograph", and the app offers to insert those words.
  - **LLM** frees ComfyUI and loads the LLM in MLX-Serve, so Hermes (in Hermex) has the memory. Images pause until you choose Generate or Edit again, which unloads the LLM.
  - A switch asks before it stops something: the LLM for Hermes, or queued image jobs. If Hermes loads the LLM by itself, the Mac frees ComfyUI, and the app says so.
  - Both Mac apps stay open, and this app never chats with the LLM.
- **Privacy cover:** in the app switcher, the app shows a blank screen, so images don't show there. It's on by default.

## Before you start

- The server runs on your Mac with `--tailscale` (`./start.sh` in `server/`). Its startup output shows the address to enter, on the `Open` line, e.g. `https://my-mac.tail1a2b3c.ts.net:8443`. The port is there when the Mac's main address already serves something else, such as Hermes.
- **Tailscale** is installed on the iPhone, signed in with the same account as the Mac, and connected.
- Node.js and the Expo CLI are on the Mac, which you already have.

## Run it in Expo Go (free, quickest)

1. Install **Expo Go** from the App Store. This project uses Expo SDK 57, the SDK the App Store's Expo Go supports (57.0.9). Each Expo Go release runs only one SDK. When the App Store's Expo Go moves to SDK 58, it will refuse this project as "incompatible" until the app is upgraded with `npx expo install expo@^58.0.0 && npx expo install --fix`.
2. On the Mac, run:
   ```sh
   cd app
   ./start.sh
   ```
   The first run installs dependencies. When Tailscale is up, the script points Expo Go at the Mac's tailnet name, so the app loads anywhere your iPhone has Tailscale, not only on home Wi-Fi.
3. Scan the QR code with the iPhone camera. In the app, enter the server address and password once.

Expo Go loads the app from the Mac each time, so `./start.sh` has to be running while you use it. If macOS asks whether `node` may accept incoming connections, allow it. Inside Expo Go, the Photos prompt names Expo Go rather than Prompt.

## Open the published version in Expo Go (no dev server)

The app is published with EAS Update as `@grabbiel/comfyui-prompt-app`, so Expo Go can open it without `./start.sh`.

1. In Expo Go on the iPhone, sign in with the `grabbiel` Expo account. Expo Go only opens published projects that belong to the signed-in account or one of its organizations.
2. Open the project's [Updates page](https://expo.dev/accounts/grabbiel/projects/comfyui-prompt-app/updates) on the iPhone, pick the newest update and open it in Expo Go. You can also scan its QR code from the Mac.

To publish a new version after changing the app:

```sh
npx eas-cli@latest update --channel production --environment production --platform ios --message "What changed"
```

An update carries only JavaScript and images. If you add a library with native code or change the Expo SDK, Expo Go has to support it too.

## Install it as its own app (persistent)

To get **Prompt** on your home screen without a dev server, build it with EAS using your Expo account. Installing a build on your own iPhone this way needs a paid Apple Developer account.

```sh
cd app
npx eas-cli@latest init                          # already linked to @grabbiel/comfyui-prompt-app; a no-op now
npx eas-cli@latest device:create                 # registers your iPhone (once; open the link on the iPhone)
npx eas-cli@latest build --platform ios --profile preview
```

When the build finishes, open its link on the iPhone to install. The bundle identifier is `com.grabbiel.comfyprompt` in `app.json`; change it if you like before the first build.

Without a paid account, `npx expo run:ios --device --configuration Release` installs it from Xcode with a free Apple ID. That copy stops opening after 7 days and has to be installed again.

## Development

```sh
npm test              # unit tests: address handling, statuses, modes, history merging, the API client
npm run typecheck
npm run lint
```

- `src/app/`: screens (Expo Router). `index` is the prompt feed and composer, `connect` the connection setup, `settings` and `viewer` the rest.
- `src/lib/`: the API client (`api.ts`), the connection kept in the Keychain (`connection.tsx`), the shared history and its polling (`jobsStore.tsx`), the modes (`servicesStore.tsx`, `services.ts`), picking and uploading a picture to edit (`pictures.ts`), saving and sharing (`save.ts`).
- `src/components/`: the job card, composer, mode bar, banner and privacy cover.

Native modules are limited to those bundled in Expo Go, so the same code runs in Expo Go and in an EAS build. The photo picker runs outside the app, so choosing a picture needs no access to your library. Face ID isn't available in Expo Go, so there is no Face ID lock.
