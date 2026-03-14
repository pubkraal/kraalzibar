import { KraalzibarError, mapHttpStatus } from "./error.js";

export const DEFAULT_MAX_RESPONSE_SIZE = 10 * 1024 * 1024; // 10 MB

export type RestTransportOptions = {
  target: string;
  apiKey?: string;
  timeout?: number;
  maxResponseSize?: number;
};

export type RestTransport = {
  post: <TReq, TRes>(path: string, body: TReq) => Promise<TRes>;
  get: <TRes>(path: string) => Promise<TRes>;
};

export const createRestTransport = (
  options: RestTransportOptions,
  fetchImpl: typeof globalThis.fetch = globalThis.fetch,
): RestTransport => {
  const baseUrl = options.target.replace(/\/$/, "");
  const timeout = options.timeout ?? 30_000;
  const maxResponseSize = options.maxResponseSize ?? DEFAULT_MAX_RESPONSE_SIZE;

  const headers = (): Record<string, string> => {
    const h: Record<string, string> = {
      "content-type": "application/json",
    };
    if (options.apiKey) {
      h["authorization"] = `Bearer ${options.apiKey}`;
    }
    return h;
  };

  const handleResponse = async <T>(response: Response): Promise<T> => {
    const contentLength = response.headers.get("content-length");
    if (contentLength && parseInt(contentLength, 10) > maxResponseSize) {
      throw new KraalzibarError("RESOURCE_EXHAUSTED", "response too large");
    }

    const buffer = await response.arrayBuffer();
    if (buffer.byteLength > maxResponseSize) {
      throw new KraalzibarError("RESOURCE_EXHAUSTED", "response too large");
    }

    const text = new TextDecoder().decode(buffer);

    if (response.ok) {
      return JSON.parse(text) as T;
    }

    let message = response.statusText;
    try {
      const body = JSON.parse(text);
      if (typeof body === "object" && body !== null && "error" in body) {
        message = String((body as { error: unknown }).error);
      }
    } catch {
      // ignore parse errors
    }

    throw new KraalzibarError(mapHttpStatus(response.status), message);
  };

  return {
    post: async <TReq, TRes>(path: string, body: TReq): Promise<TRes> => {
      const response = await fetchImpl(`${baseUrl}${path}`, {
        method: "POST",
        headers: headers(),
        body: JSON.stringify(body),
        signal: AbortSignal.timeout(timeout),
      });
      return handleResponse<TRes>(response);
    },
    get: async <TRes>(path: string): Promise<TRes> => {
      const response = await fetchImpl(`${baseUrl}${path}`, {
        method: "GET",
        headers: headers(),
        signal: AbortSignal.timeout(timeout),
      });
      return handleResponse<TRes>(response);
    },
  };
};
