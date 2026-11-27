import type { ForgeErr } from "./schema";

export class ApiError extends Error {
  errors: ForgeErr[];
  status: number;
  constructor(status: number, errors: ForgeErr[]) {
    super(errors.map((e) => e.message).join("; ") || `HTTP ${status}`);
    this.errors = errors;
    this.status = status;
  }
}

async function request<T>(method: "GET" | "POST", path: string, body?: unknown, signal?: AbortSignal): Promise<T> {
  const res = await fetch(path, {
    method,
    credentials: "same-origin",
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
  let data: unknown = null;
  try {
    data = await res.json();
  } catch {
    throw new ApiError(res.status, [{ code: "X3_FORGE_HTTP", message: `HTTP ${res.status} with no JSON body` }]);
  }
  if (!res.ok) {
    const errors = (data as { errors?: ForgeErr[] })?.errors ?? [{ code: "X3_FORGE_HTTP", message: `HTTP ${res.status}` }];
    throw new ApiError(res.status, errors);
  }
  return data as T;
}

export const api = {
  get: <T>(path: string, signal?: AbortSignal) => request<T>("GET", path, undefined, signal),
  post: <T>(path: string, body: unknown, signal?: AbortSignal) => request<T>("POST", path, body, signal),
};

export function errorsOf(e: unknown): ForgeErr[] {
  if (e instanceof ApiError) return e.errors;
  if (e instanceof DOMException && e.name === "AbortError") return [];
  return [{ code: "X3_FORGE_CLIENT", message: e instanceof Error ? e.message : String(e) }];
}

export function download(filename: string, data: BlobPart, type = "application/json") {
  const url = URL.createObjectURL(new Blob([data], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 2000);
}

export function randomHex(bytes: number): string {
  const b = crypto.getRandomValues(new Uint8Array(bytes));
  return Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
}
