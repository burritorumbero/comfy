import type { JobKind, Mode, Services } from "./api";
import { clock } from "./jobs";

/**
 * Where the Mac stands. `generate` and `edit`: ComfyUI answers and the LLM holds no model.
 * `llm`: the LLM holds a model, so images are paused. `off`: ComfyUI isn't running and the LLM
 * holds nothing.
 */
export type Status = "switching" | "generate" | "edit" | "llm" | "off";

const llmLoaded = (services: Services) => (services.llm.models ?? []).length > 0;

export function statusOf(services: Services): Status {
  if (services.switching) return "switching";
  if (llmLoaded(services)) return "llm";
  if (!services.images.running) return "off";
  return services.mode === "edit" ? "edit" : "generate";
}

/** The mode to show as chosen: where a switch is headed, or what the Mac is set up for. */
export function selectedMode(services: Services): Mode {
  if (services.switching) return services.switching.to;
  return llmLoaded(services) ? "llm" : services.mode;
}

/**
 * The image job the Mac takes now, or null while the LLM holds the memory or a switch runs.
 * Without switching (`services` null) the server only generates.
 */
export function imageKindOf(services: Services | null): JobKind | null {
  if (!services) return "generate";
  if (services.switching || llmLoaded(services)) return null;
  return services.mode === "edit" ? "edit" : "generate";
}

/** Prompts of this kind can't go to ComfyUI right now. */
export function promptsPaused(
  services: Services | null,
  kind: JobKind = "generate",
): boolean {
  if (!services) return false;
  return imageKindOf(services) !== kind;
}

export const modeLabel: Record<Mode, string> = {
  generate: "Generate",
  edit: "Edit",
  llm: "LLM",
};

export function formatBytes(bytes: number | null | undefined): string | null {
  if (bytes == null || bytes <= 0) return null;
  const gb = bytes / 1e9;
  if (gb >= 10) return `${Math.round(gb)} GB`;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  return `${Math.max(1, Math.round(bytes / 1e6))} MB`;
}

/** The last part of a model id: `ddalcu/Qwen3.8-Flash-Next` → `Qwen3.8-Flash-Next`. */
export function modelName(id: string): string {
  return id.split("/").pop() || id;
}

const withMemory = (text: string, bytes: number | null) =>
  [text, formatBytes(bytes)].filter(Boolean).join(" · ");

export function imagesSummary(services: Services): string {
  const { images } = services;
  if (images.running) return withMemory("Running", images.memory);
  return images.appOpen ? "Not answering" : "Not running";
}

export function llmSummary(services: Services): string {
  const { llm } = services;
  if (!llm.running) return llm.appOpen ? "Server stopped" : "Not running";
  const models = llm.models ?? [];
  if (models.length === 0) return "No model loaded";
  return withMemory(models.map(modelName).join(", "), llm.memory);
}

/** Why ComfyUI can't take prompts, when it isn't running. */
export function imagesOffText(services: Services): string {
  const app = services.images.app ?? "ComfyUI";
  return services.images.appOpen
    ? `${app} is open, but ComfyUI isn't answering.`
    : "ComfyUI isn't running on your Mac.";
}

/** Why images are paused while the LLM holds a model. */
export function llmLoadedText(services: Services): string {
  const models = (services.llm.models ?? []).map(modelName).join(", ");
  const memory = formatBytes(services.llm.memory);
  return `The LLM (${[models, memory].filter(Boolean).join(" · ")}) is loaded on your Mac, so images are paused. Choose Generate or Edit above to unload it.`;
}

/**
 * MLX-Serve's server can't be started from here once it's stopped from its menu-bar icon: reopening
 * the app doesn't start it. Shown while that is the case.
 */
export function llmStoppedHint(services: Services): string | null {
  const { llm } = services;
  if (llm.running || !llm.appOpen) return null;
  return "MLX-Serve's server is stopped. To load the LLM, start it first from MLX-Serve's menu-bar icon on your Mac.";
}

/** The short status under the title. */
export function headline(services: Services): string {
  switch (statusOf(services)) {
    case "switching": {
      const to = services.switching?.to ?? "generate";
      return to === "llm"
        ? "Switching to the LLM…"
        : `Switching to ${modeLabel[to]}…`;
    }
    case "llm":
      return "LLM on · images paused";
    case "generate":
      return "Ready to generate";
    case "edit":
      return "Ready to edit";
    case "off":
      return "ComfyUI offline";
  }
}

/** The current step and how long the switch has taken: "Loading Qwen3.8… · 0:41". */
export function switchingText(services: Services, now: number): string | null {
  const switching = services.switching;
  if (!switching) return null;
  return `${switching.step} · ${clock(Math.max(0, now - switching.since))}`;
}

/**
 * What a switch stops, to confirm first. Null when it stops nothing: moving between Generate and
 * Edit only frees ComfyUI's memory, and the server asks separately about image jobs.
 */
export function switchQuestion(
  services: Services | null,
  to: Mode,
): { title: string; message: string } | null {
  if (to === "llm") {
    return {
      title: "Switch to the LLM?",
      message:
        "ComfyUI frees its memory and MLX-Serve loads the LLM, so Hermes can answer. Images pause until you switch back.",
    };
  }
  const loaded = (services?.llm.models ?? []).map(modelName).join(", ");
  if (!loaded) return null;
  return {
    title: `Switch to ${modeLabel[to]}?`,
    message: `MLX-Serve unloads ${loaded} to make room, so Hermes can't answer until you switch back.`,
  };
}
