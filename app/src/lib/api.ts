import { hostOf } from "./address";

export type Connection = { address: string; password: string };

export type JobState =
  "queued" | "running" | "done" | "error" | "cancelled" | "lost";
export type JobImage = {
  filename: string;
  url: string;
  preview: string;
  download: string;
};
/** A job makes a new image from a prompt, or edits a picture following one. */
export type JobKind = "generate" | "edit";
export type Job = {
  id: string;
  prompt: string;
  /** Missing on servers from before editing. */
  kind?: JobKind;
  /** Where the picture an edit started from is served. */
  source?: string;
  /** The workflow's switches, as this job set them. */
  options?: Record<string, boolean>;
  /** Unix milliseconds. */
  created: number;
  state: JobState;
  seed?: number;
  seconds?: number;
  error?: string;
  /** Jobs ComfyUI runs before this one. */
  ahead?: number;
  progress?: { value: number; max: number };
  images?: JobImage[];
};
export type JobPage = { jobs: Job[]; more: boolean };
/** A LoRA the app can switch, from a node titled "Option: <name>" in the workflow. */
export type WorkflowOption = { name: string; on: boolean };
export type WorkflowInfo = { title: string; options: WorkflowOption[] };
export type ServerInfo = {
  title: string;
  /** Missing on servers from before editing. `edit` is null when editing isn't set up. */
  workflows?: { generate: WorkflowInfo; edit: WorkflowInfo | null };
  comfy: boolean;
  progress: boolean;
  seed: "random" | "fixed";
  maxPromptChars: number;
  /** The server can switch the Mac between image generation and the LLM. */
  services?: boolean;
  version: string;
};

/**
 * What the Mac's memory is set aside for: ComfyUI with the image model, ComfyUI with the edit
 * model, or the LLM (MLX-Serve, which Hermes uses). Only one is loaded at a time.
 */
export type Mode = "generate" | "edit" | "llm";
export type ServiceState = {
  name: string;
  /** The app that runs it on the Mac. */
  app: string | null;
  /** It answers right now. */
  running: boolean;
  /** Its app is open. MLX-Serve can be open with its server stopped. */
  appOpen?: boolean;
  /** Bytes in memory, when known. */
  memory: number | null;
  /** The LLM models held in memory. */
  models?: string[];
};
export type Services = {
  /** The LLM while it holds a model, otherwise the image mode ComfyUI is set up for. */
  mode: Mode;
  /** The modes this server offers, in order. */
  modes: Mode[];
  switching: { to: Mode; step: string; since: number } | null;
  /** Why the last switch failed. */
  error: string | null;
  /** Something the server did by itself, such as freeing ComfyUI when Hermes loaded the LLM. */
  notice: string | null;
  images: ServiceState;
  llm: ServiceState;
};

/** What to edit: a picture uploaded from this iPhone, or an image in the history. */
export type EditSource = { upload: string } | { job: string; index: number };
export type GenerateRequest = {
  prompt: string;
  kind: JobKind;
  /** Required for an edit. */
  source?: EditSource;
  options?: Record<string, boolean>;
};

export const PAGE_SIZE = 30;

/** A failed call. `status` is 0 when the server could not be reached at all. */
export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    /** Set when a switch would stop this many image jobs. */
    readonly activeJobs?: number,
  ) {
    super(message);
  }
}

export function unreachableMessage(address: string): string {
  return `Can't reach ${hostOf(address)}. Check that Tailscale is connected on this iPhone and the server is running on your Mac.`;
}

/** Every request carries the password; there is no sign-in or session. */
export function authHeaders(connection: Connection): Record<string, string> {
  return { Authorization: `Bearer ${connection.password}` };
}

async function fetchWithTimeout(
  url: string,
  init: RequestInit,
  timeoutMs: number,
): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    return await fetch(url, { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

type CallOptions = {
  method?: "GET" | "POST" | "DELETE";
  body?: unknown;
  timeoutMs?: number;
};

async function call<T>(
  connection: Connection,
  path: string,
  options: CallOptions = {},
): Promise<T> {
  const headers: Record<string, string> = { ...authHeaders(connection) };
  if (options.body !== undefined) headers["Content-Type"] = "application/json";
  let response: Response;
  try {
    response = await fetchWithTimeout(
      connection.address + path,
      {
        method: options.method ?? "GET",
        headers,
        body:
          options.body === undefined ? undefined : JSON.stringify(options.body),
      },
      options.timeoutMs ?? 15000,
    );
  } catch {
    throw new ApiError(unreachableMessage(connection.address), 0);
  }
  let body: unknown = null;
  try {
    body = await response.json();
  } catch {
    // An empty or non-JSON body; the status says enough.
  }
  if (!response.ok) {
    const failure = body as { error?: string; activeJobs?: number } | null;
    throw new ApiError(
      failure?.error ?? `The server answered ${response.status}.`,
      response.status,
      failure?.activeJobs,
    );
  }
  return body as T;
}

export const api = {
  info: (c: Connection) => call<ServerInfo>(c, "/api/info"),
  jobs: (c: Connection, before?: string) =>
    call<JobPage>(
      c,
      `/api/jobs?limit=${PAGE_SIZE}${before ? `&before=${encodeURIComponent(before)}` : ""}`,
    ),
  job: (c: Connection, id: string) => call<Job>(c, `/api/jobs/${id}`),
  /** An edit of a history image takes a moment longer: the Mac copies it for ComfyUI first. */
  generate: (c: Connection, request: GenerateRequest) =>
    call<Job>(c, "/api/generate", {
      method: "POST",
      body: request,
      timeoutMs: 30000,
    }),
  cancel: (c: Connection, id: string) =>
    call<Job>(c, `/api/jobs/${id}/cancel`, { method: "POST" }),
  remove: (c: Connection, id: string) =>
    call<unknown>(c, `/api/jobs/${id}`, { method: "DELETE" }),
  services: (c: Connection) => call<Services>(c, "/api/services"),
  /** Starts the switch; it runs on the Mac and `services` shows how it goes. */
  switchTo: (c: Connection, to: Mode, cancelJobs = false) =>
    call<Services>(c, "/api/services/switch", {
      method: "POST",
      body: { to, cancelJobs },
      timeoutMs: 20000,
    }),
};

/** An image source that carries the password, for expo-image. */
export function imageSource(connection: Connection, path: string) {
  return { uri: connection.address + path, headers: authHeaders(connection) };
}

export type TestResult =
  | { ok: true; info: ServerInfo }
  | { ok: false; message: string; field?: "address" | "password" };

/**
 * Check an address and password before saving them. `/health` answers without the
 * password, so "can't reach it" and "wrong password" are told apart.
 */
export async function testConnection(
  connection: Connection,
): Promise<TestResult> {
  try {
    const health = await fetchWithTimeout(
      `${connection.address}/health`,
      {},
      8000,
    );
    const body = (await health.json().catch(() => null)) as {
      service?: string;
    } | null;
    if (!health.ok || body?.service !== "comfyui-prompt-server") {
      return {
        ok: false,
        field: "address",
        message:
          "That address answers, but it isn't the ComfyUI prompt server. Use the https://… address the server prints when it starts.",
      };
    }
  } catch {
    return {
      ok: false,
      field: "address",
      message: unreachableMessage(connection.address),
    };
  }
  try {
    return { ok: true, info: await api.info(connection) };
  } catch (e) {
    if (e instanceof ApiError && e.status === 401)
      return { ok: false, field: "password", message: "Wrong password." };
    return { ok: false, message: e instanceof Error ? e.message : String(e) };
  }
}
