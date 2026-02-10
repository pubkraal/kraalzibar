import { KraalzibarError, mapHttpStatus } from "./error.js";

export type RestTransportOptions = {
  target: string;
  apiKey?: string;
  timeout?: number;
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
    if (response.ok) {
      return (await response.json()) as T;
    }

    let message = response.statusText;
    try {
      const body = await response.json();
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
