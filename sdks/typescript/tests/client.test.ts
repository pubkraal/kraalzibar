import { describe, it, expect, vi } from "vitest";
import { createClient } from "../src/client.js";
import { KraalzibarError } from "../src/error.js";

const mockFetch = (
  status: number,
  body: unknown,
  captureRequest?: { url?: string; init?: RequestInit },
) => {
  return vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    if (captureRequest) {
      captureRequest.url = String(url);
      captureRequest.init = init;
    }
    return new Response(JSON.stringify(body), {
      status,
      statusText: status === 200 ? "OK" : "Error",
      headers: { "content-type": "application/json" },
    });
  }) as unknown as typeof globalThis.fetch;
};

describe("createClient", () => {
  it("should create client with default options", () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(200, {}),
    });

    expect(client).toBeDefined();
    expect(client.checkPermission).toBeDefined();
    expect(client.writeRelationships).toBeDefined();
    expect(client.readRelationships).toBeDefined();
    expect(client.lookupResources).toBeDefined();
    expect(client.lookupSubjects).toBeDefined();
    expect(client.writeSchema).toBeDefined();
    expect(client.readSchema).toBeDefined();
  });

  it("should throw on missing target", () => {
    expect(() => createClient({ target: "" })).toThrow("target is required");
  });
});

describe("error mapping", () => {
  it("should map 400 response to INVALID_ARGUMENT error", async () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(400, { error: "bad input" }),
    });

    await expect(
      client.checkPermission({
        resource_type: "doc",
        resource_id: "1",
        permission: "view",
        subject_type: "user",
        subject_id: "alice",
      }),
    ).rejects.toThrow(KraalzibarError);

    try {
      await client.checkPermission({
        resource_type: "doc",
        resource_id: "1",
        permission: "view",
        subject_type: "user",
        subject_id: "alice",
      });
    } catch (e) {
      expect(e).toBeInstanceOf(KraalzibarError);
      expect((e as KraalzibarError).code).toBe("INVALID_ARGUMENT");
    }
  });

  it("should map 401 response to UNAUTHENTICATED error", async () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(401, { error: "unauthorized" }),
    });

    try {
      await client.readSchema();
    } catch (e) {
      expect(e).toBeInstanceOf(KraalzibarError);
      expect((e as KraalzibarError).code).toBe("UNAUTHENTICATED");
    }
  });

  it("should map 404 response to NOT_FOUND error", async () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(404, { error: "not found" }),
    });

    try {
      await client.readSchema();
    } catch (e) {
      expect(e).toBeInstanceOf(KraalzibarError);
      expect((e as KraalzibarError).code).toBe("NOT_FOUND");
    }
  });

  it("should map 422 response to RESOURCE_EXHAUSTED error", async () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(422, { error: "max depth exceeded" }),
    });

    try {
      await client.checkPermission({
        resource_type: "doc",
        resource_id: "1",
        permission: "view",
        subject_type: "user",
        subject_id: "alice",
      });
    } catch (e) {
      expect(e).toBeInstanceOf(KraalzibarError);
      expect((e as KraalzibarError).code).toBe("RESOURCE_EXHAUSTED");
    }
  });

  it("should map 500 response to INTERNAL error", async () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(500, { error: "server error" }),
    });

    try {
      await client.readSchema();
    } catch (e) {
      expect(e).toBeInstanceOf(KraalzibarError);
      expect((e as KraalzibarError).code).toBe("INTERNAL");
    }
  });

  it("should include error message from response body", async () => {
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(400, { error: "field X is required" }),
    });

    try {
      await client.checkPermission({
        resource_type: "doc",
        resource_id: "1",
        permission: "view",
        subject_type: "user",
        subject_id: "alice",
      });
    } catch (e) {
      expect(e).toBeInstanceOf(KraalzibarError);
      expect((e as KraalzibarError).message).toBe("field X is required");
    }
  });
});

describe("REST operations", () => {
  it("should check permission via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(200, { allowed: true, checked_at: "42" }, captured),
    });

    const result = await client.checkPermission({
      resource_type: "document",
      resource_id: "readme",
      permission: "can_view",
      subject_type: "user",
      subject_id: "alice",
    });

    expect(result.allowed).toBe(true);
    expect(result.checked_at).toBe("42");
    expect(captured.url).toBe("http://localhost:8080/v1/permissions/check");
    expect(captured.init?.method).toBe("POST");
  });

  it("should write relationships via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(200, { written_at: "1" }, captured),
    });

    const result = await client.writeRelationships({
      updates: [
        {
          operation: "touch",
          resource_type: "document",
          resource_id: "readme",
          relation: "viewer",
          subject_type: "user",
          subject_id: "alice",
        },
      ],
    });

    expect(result.written_at).toBe("1");
    expect(captured.url).toBe("http://localhost:8080/v1/relationships/write");
  });

  it("should read relationships via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(
        200,
        {
          relationships: [
            {
              resource_type: "document",
              resource_id: "readme",
              relation: "viewer",
              subject_type: "user",
              subject_id: "alice",
            },
          ],
        },
        captured,
      ),
    });

    const result = await client.readRelationships({
      filter: { resource_type: "document" },
    });

    expect(result.relationships).toHaveLength(1);
    expect(captured.url).toBe("http://localhost:8080/v1/relationships/read");
  });

  it("should lookup resources via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(200, { resource_ids: ["doc1", "doc2"] }, captured),
    });

    const result = await client.lookupResources({
      resource_type: "document",
      permission: "can_view",
      subject_type: "user",
      subject_id: "alice",
    });

    expect(result.resource_ids).toEqual(["doc1", "doc2"]);
    expect(captured.url).toBe("http://localhost:8080/v1/permissions/resources");
  });

  it("should lookup subjects via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(
        200,
        {
          subjects: [
            { subject_type: "user", subject_id: "alice" },
            { subject_type: "user", subject_id: "bob" },
          ],
        },
        captured,
      ),
    });

    const result = await client.lookupSubjects({
      resource_type: "document",
      resource_id: "readme",
      permission: "can_view",
      subject_type: "user",
    });

    expect(result.subjects).toHaveLength(2);
    expect(captured.url).toBe("http://localhost:8080/v1/permissions/subjects");
  });

  it("should write schema via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(
        200,
        { breaking_changes_overridden: false },
        captured,
      ),
    });

    const result = await client.writeSchema("definition user {}");

    expect(result.breaking_changes_overridden).toBe(false);
    expect(captured.url).toBe("http://localhost:8080/v1/schema");
    expect(captured.init?.method).toBe("POST");
    const body = JSON.parse(captured.init?.body as string);
    expect(body.schema).toBe("definition user {}");
    expect(body.force).toBe(false);
  });

  it("should read schema via REST", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(200, { schema: "definition user {}" }, captured),
    });

    const result = await client.readSchema();

    expect(result).toBe("definition user {}");
    expect(captured.url).toBe("http://localhost:8080/v1/schema");
    expect(captured.init?.method).toBe("GET");
  });
});

describe("authorization header", () => {
  it("should include authorization header when api key is set", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      apiKey: "test-key-123",
      fetch: mockFetch(200, { schema: "definition user {}" }, captured),
    });

    await client.readSchema();

    const headers = captured.init?.headers as Record<string, string>;
    expect(headers["authorization"]).toBe("Bearer test-key-123");
  });

  it("should not include authorization header when no api key", async () => {
    const captured: { url?: string; init?: RequestInit } = {};
    const client = createClient({
      target: "http://localhost:8080",
      fetch: mockFetch(200, { schema: "definition user {}" }, captured),
    });

    await client.readSchema();

    const headers = captured.init?.headers as Record<string, string>;
    expect(headers["authorization"]).toBeUndefined();
  });
});
