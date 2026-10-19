import type { Services } from "../api";
import {
  formatBytes,
  headline,
  imageKindOf,
  imagesOffText,
  imagesSummary,
  llmLoadedText,
  llmStoppedHint,
  llmSummary,
  modelName,
  promptsPaused,
  selectedMode,
  statusOf,
  switchQuestion,
  switchingText,
} from "../services";

const QWEN = "ddalcu/Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit";

const services = (
  extra: {
    mode?: Services["mode"];
    models?: string[];
    comfy?: boolean;
    llmRunning?: boolean;
    switching?: Services["switching"];
  } = {},
): Services => ({
  mode: extra.mode ?? ((extra.models ?? []).length > 0 ? "llm" : "generate"),
  modes: ["generate", "edit", "llm"],
  switching: extra.switching ?? null,
  error: null,
  notice: null,
  images: {
    name: "ComfyUI",
    app: "Comfy Desktop",
    running: extra.comfy ?? true,
    memory: 63_500_000_000,
  },
  llm: {
    name: "MLX-Serve",
    app: "MLX-Serve",
    running: extra.llmRunning ?? true,
    memory: 107_000_000_000,
    models: extra.models ?? [],
  },
});

describe("status", () => {
  it("follows what holds the memory", () => {
    expect(statusOf(services())).toBe("generate");
    expect(statusOf(services({ mode: "edit" }))).toBe("edit");
    expect(statusOf(services({ models: [QWEN] }))).toBe("llm");
    expect(statusOf(services({ comfy: false }))).toBe("off");
    expect(
      statusOf(
        services({
          models: [QWEN],
          switching: { to: "edit", step: "Unloading…", since: 0 },
        }),
      ),
    ).toBe("switching");
  });

  it("shows where a switch is headed as chosen", () => {
    expect(selectedMode(services({ mode: "edit" }))).toBe("edit");
    expect(selectedMode(services({ models: [QWEN] }))).toBe("llm");
    expect(
      selectedMode(
        services({ switching: { to: "llm", step: "x", since: 0 } }),
      ),
    ).toBe("llm");
  });

  it("takes one kind of image job at a time", () => {
    expect(imageKindOf(null)).toBe("generate");
    expect(imageKindOf(services())).toBe("generate");
    expect(imageKindOf(services({ mode: "edit" }))).toBe("edit");
    expect(imageKindOf(services({ models: [QWEN] }))).toBeNull();
    expect(
      imageKindOf(services({ switching: { to: "edit", step: "x", since: 0 } })),
    ).toBeNull();

    expect(promptsPaused(null)).toBe(false);
    expect(promptsPaused(services())).toBe(false);
    expect(promptsPaused(services(), "edit")).toBe(true);
    expect(promptsPaused(services({ mode: "edit" }), "edit")).toBe(false);
    // ComfyUI starts when the prompt arrives, so being off pauses nothing.
    expect(promptsPaused(services({ comfy: false }))).toBe(false);
    expect(promptsPaused(services({ models: [QWEN] }))).toBe(true);
  });
});

describe("labels", () => {
  it("says what each side is doing", () => {
    expect(headline(services())).toBe("Ready to generate");
    expect(headline(services({ mode: "edit" }))).toBe("Ready to edit");
    expect(headline(services({ models: [QWEN] }))).toBe(
      "LLM on · images paused",
    );
    expect(
      headline(services({ switching: { to: "llm", step: "x", since: 0 } })),
    ).toBe("Switching to the LLM…");
    expect(
      headline(services({ switching: { to: "edit", step: "x", since: 0 } })),
    ).toBe("Switching to Edit…");
    expect(imagesSummary(services())).toBe("Running · 64 GB");
    expect(imagesSummary(services({ comfy: false }))).toBe("Not running");
    expect(llmSummary(services({ models: [QWEN] }))).toBe(
      "Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit · 107 GB",
    );
    expect(llmSummary(services())).toBe("No model loaded");
    expect(llmSummary(services({ llmRunning: false }))).toBe("Not running");
    expect(llmLoadedText(services({ models: [QWEN] }))).toBe(
      "The LLM (Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit · 107 GB) is loaded on your Mac, so images are paused. Choose Generate or Edit above to unload it.",
    );
  });

  it("tells an open app with its server stopped from a closed one", () => {
    const stopped = services({ llmRunning: false });
    stopped.llm.appOpen = true;
    expect(llmSummary(stopped)).toBe("Server stopped");
    expect(llmStoppedHint(stopped)).toMatch(/menu-bar icon/);
    expect(llmStoppedHint(services({ llmRunning: false }))).toBeNull();
    expect(llmStoppedHint(services())).toBeNull();

    const quiet = services({ comfy: false });
    expect(imagesOffText(quiet)).toBe("ComfyUI isn't running on your Mac.");
    quiet.images.appOpen = true;
    expect(imagesSummary(quiet)).toBe("Not answering");
    expect(imagesOffText(quiet)).toBe(
      "Comfy Desktop is open, but ComfyUI isn't answering.",
    );
  });

  it("asks before a switch only when it stops something", () => {
    expect(switchQuestion(services(), "llm")?.title).toBe("Switch to the LLM?");
    expect(switchQuestion(services(), "edit")).toBeNull();
    expect(switchQuestion(services({ mode: "edit" }), "generate")).toBeNull();
    expect(switchQuestion(services({ models: [QWEN] }), "edit")).toEqual({
      title: "Switch to Edit?",
      message:
        "MLX-Serve unloads Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit to make room, so Hermes can't answer until you switch back.",
    });
  });

  it("times the switch", () => {
    const switching = {
      to: "llm" as const,
      step: "Loading Qwen…",
      since: 1_000_000,
    };
    expect(switchingText(services({ switching }), 1_041_000)).toBe(
      "Loading Qwen… · 0:41",
    );
    expect(switchingText(services(), 0)).toBeNull();
  });

  it("formats memory and model names", () => {
    expect(formatBytes(null)).toBeNull();
    expect(formatBytes(0)).toBeNull();
    expect(formatBytes(2_400_000_000)).toBe("2.4 GB");
    expect(formatBytes(350_000_000)).toBe("350 MB");
    expect(modelName(QWEN)).toBe("Qwen3.8-Flash-Next-MLX-Serve-mixed-4-8bit");
    expect(modelName("gemma")).toBe("gemma");
  });
});
