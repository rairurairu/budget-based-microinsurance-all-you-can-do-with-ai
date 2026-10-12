// The typed client for contracts/openapi.json. Types are generated (`just gen`, never edited by
// hand: src/generated); this file only turns a path and method into a typed fetch. Path
// parameters are percent-encoded (some ids contain `/`). The bearer token goes in the
// Authorization header, never in a URL, including for the payout stream, which is read with
// fetch rather than EventSource for that reason.
import type { components, paths } from "./generated/schema.ts";

export type { components, paths } from "./generated/schema.ts";
export type Schemas = components["schemas"];
export * from "./format.ts";

type Method = "get" | "post" | "put" | "patch";

/** The paths that have an operation for `M`. */
export type PathsFor<M extends Method> = {
  [P in keyof paths]: paths[P][M] extends { responses: unknown } ? P : never;
}[keyof paths];

type Op<P extends keyof paths, M extends Method> = paths[P][M];

type PathParams<O> = O extends { parameters: { path: infer X } }
  ? [X] extends [never | undefined]
    ? undefined
    : X
  : undefined;
type QueryParams<O> = O extends { parameters: { query?: infer Q } }
  ? [Q] extends [never | undefined]
    ? undefined
    : Q
  : undefined;
type RequestBody<O> = O extends { requestBody: { content: { "application/json": infer B } } }
  ? B
  : O extends { requestBody: { content: { "text/plain": infer T } } }
    ? T
    : undefined;
/** The 200 response's JSON (or text). */
export type Ok<O> = O extends { responses: { 200: { content: { "application/json": infer R } } } }
  ? R
  : O extends { responses: { 200: { content: { "text/plain": infer T } } } }
    ? T
    : undefined;

export interface CallOptions<O> {
  path?: PathParams<O>;
  query?: QueryParams<O>;
  body?: RequestBody<O>;
}

/** The API refused the request, or could not be reached. */
export class ApiError extends Error {
  readonly status: number;
  readonly body: Schemas["ErrorBody"] | null;

  constructor(status: number, body: Schemas["ErrorBody"] | null) {
    super(body?.detail ?? `HTTP ${status}`);
    this.status = status;
    this.body = body;
  }

  /** The business rule that refused the request, when one did. */
  get rule(): string | null {
    return this.body?.rule ?? null;
  }
}

export interface ClientConfig {
  /** Where the API is, such as `https://api.insurer.example` or `` for the same origin. */
  baseUrl: string;
  /** The bearer token for each call, if any. */
  token?: () => string | null | undefined;
  fetch?: typeof fetch;
}

/** `/me/claims/{claim_id}` with `{ claim_id: "a/b" }` is `/me/claims/a%2Fb`. */
export function fill(
  path: string,
  params?: Record<string, unknown>,
  query?: Record<string, unknown>,
): string {
  const filled = path.replace(/\{([^}]+)\}/g, (_, name: string) => {
    const value = params?.[name];
    if (value === undefined || value === null) {
      throw new Error(`missing path parameter ${name}`);
    }
    return encodeURIComponent(String(value));
  });
  const q = Object.entries(query ?? {})
    .filter(([, v]) => v !== undefined && v !== null && v !== "")
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`)
    .join("&");
  return q ? `${filled}?${q}` : filled;
}

export function createClient(config: ClientConfig) {
  const doFetch = config.fetch ?? ((input, init) => fetch(input, init));

  function headers(json: boolean): Record<string, string> {
    const h: Record<string, string> = { Accept: "application/json" };
    const token = config.token?.();
    if (token) {
      h.Authorization = `Bearer ${token}`;
    }
    if (json) {
      h["Content-Type"] = "application/json";
    }
    return h;
  }

  async function call<M extends Method, P extends PathsFor<M>>(
    method: M,
    path: P,
    options: CallOptions<Op<P, M>> = {},
  ): Promise<Ok<Op<P, M>>> {
    const url =
      config.baseUrl +
      fill(
        path as string,
        options.path as Record<string, unknown> | undefined,
        options.query as Record<string, unknown> | undefined,
      );
    const hasBody = options.body !== undefined;
    const text = typeof options.body === "string";
    const init: RequestInit = { method: method.toUpperCase(), headers: headers(hasBody && !text) };
    if (hasBody) {
      init.body = text ? (options.body as string) : JSON.stringify(options.body);
      if (text) {
        (init.headers as Record<string, string>)["Content-Type"] = "text/plain";
      }
    }
    const res = await doFetch(url, init);
    const raw = await res.text();
    if (!res.ok) {
      let body: Schemas["ErrorBody"] | null = null;
      try {
        body = JSON.parse(raw) as Schemas["ErrorBody"];
      } catch {
        body = null;
      }
      throw new ApiError(res.status, body);
    }
    const type = res.headers.get("content-type") ?? "";
    return (type.includes("application/json") ? JSON.parse(raw) : raw) as Ok<Op<P, M>>;
  }

  /**
   * Reads server-sent events from `path` until `signal` aborts, calling `onEvent` with each
   * event's name and parsed JSON data.
   */
  async function stream(
    path: PathsFor<"get">,
    onEvent: (event: string, data: unknown) => void,
    signal?: AbortSignal,
  ): Promise<void> {
    const init: RequestInit = { headers: { ...headers(false), Accept: "text/event-stream" } };
    if (signal) {
      init.signal = signal;
    }
    const res = await doFetch(config.baseUrl + path, init);
    if (!res.ok || !res.body) {
      throw new ApiError(res.status, null);
    }
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buffer = "";
    for (;;) {
      const { done, value } = await reader.read();
      if (done) {
        return;
      }
      buffer += decoder.decode(value, { stream: true });
      for (let end = buffer.indexOf("\n\n"); end >= 0; end = buffer.indexOf("\n\n")) {
        const block = buffer.slice(0, end);
        buffer = buffer.slice(end + 2);
        const parsed = parseEvent(block);
        if (parsed) {
          onEvent(parsed.event, parsed.data);
        }
      }
    }
  }

  return {
    get: <P extends PathsFor<"get">>(path: P, options?: CallOptions<Op<P, "get">>) =>
      call("get", path, options),
    post: <P extends PathsFor<"post">>(path: P, options?: CallOptions<Op<P, "post">>) =>
      call("post", path, options),
    put: <P extends PathsFor<"put">>(path: P, options?: CallOptions<Op<P, "put">>) =>
      call("put", path, options),
    patch: <P extends PathsFor<"patch">>(path: P, options?: CallOptions<Op<P, "patch">>) =>
      call("patch", path, options),
    stream,
  };
}

export type Client = ReturnType<typeof createClient>;

/** One server-sent event block: its `event:` name and its `data:` as JSON. */
export function parseEvent(block: string): { event: string; data: unknown } | null {
  let event = "message";
  const data: string[] = [];
  for (const line of block.split("\n")) {
    if (line.startsWith("event:")) {
      event = line.slice(6).trim();
    } else if (line.startsWith("data:")) {
      data.push(line.slice(5).trimStart());
    }
  }
  if (data.length === 0) {
    return null;
  }
  try {
    return { event, data: JSON.parse(data.join("\n")) };
  } catch {
    return null;
  }
}
