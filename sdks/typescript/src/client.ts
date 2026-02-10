import { createRestTransport, type RestTransport } from "./rest-transport.js";
import type {
  CheckPermissionRequest,
  CheckPermissionResponse,
  LookupResourcesRequest,
  LookupResourcesResponse,
  LookupSubjectsRequest,
  LookupSubjectsResponse,
  ReadRelationshipsRequest,
  ReadRelationshipsResponse,
  ReadSchemaResponse,
  WriteRelationshipsRequest,
  WriteRelationshipsResponse,
  WriteSchemaOptions,
  WriteSchemaResponse,
} from "./types.js";

export type KraalzibarClient = {
  checkPermission: (
    request: CheckPermissionRequest,
  ) => Promise<CheckPermissionResponse>;
  writeRelationships: (
    request: WriteRelationshipsRequest,
  ) => Promise<WriteRelationshipsResponse>;
  readRelationships: (
    request: ReadRelationshipsRequest,
  ) => Promise<ReadRelationshipsResponse>;
  lookupResources: (
    request: LookupResourcesRequest,
  ) => Promise<LookupResourcesResponse>;
  lookupSubjects: (
    request: LookupSubjectsRequest,
  ) => Promise<LookupSubjectsResponse>;
  writeSchema: (
    schema: string,
    options?: WriteSchemaOptions,
  ) => Promise<WriteSchemaResponse>;
  readSchema: () => Promise<string>;
};

export type ClientOptions = {
  target: string;
  apiKey?: string;
  timeout?: number;
  fetch?: typeof globalThis.fetch;
};

export const createClient = (options: ClientOptions): KraalzibarClient => {
  if (!options.target) {
    throw new Error("target is required");
  }

  const transport: RestTransport = createRestTransport(
    {
      target: options.target,
      apiKey: options.apiKey,
      timeout: options.timeout,
    },
    options.fetch,
  );

  return {
    checkPermission: (request) =>
      transport.post<CheckPermissionRequest, CheckPermissionResponse>(
        "/v1/permissions/check",
        request,
      ),

    writeRelationships: (request) =>
      transport.post<WriteRelationshipsRequest, WriteRelationshipsResponse>(
        "/v1/relationships/write",
        request,
      ),

    readRelationships: (request) =>
      transport.post<ReadRelationshipsRequest, ReadRelationshipsResponse>(
        "/v1/relationships/read",
        request,
      ),

    lookupResources: (request) =>
      transport.post<LookupResourcesRequest, LookupResourcesResponse>(
        "/v1/permissions/resources",
        request,
      ),

    lookupSubjects: (request) =>
      transport.post<LookupSubjectsRequest, LookupSubjectsResponse>(
        "/v1/permissions/subjects",
        request,
      ),

    writeSchema: (schema, opts) =>
      transport.post<
        { schema: string; force: boolean },
        WriteSchemaResponse
      >("/v1/schema", { schema, force: opts?.force ?? false }),

    readSchema: async () => {
      const response = await transport.get<ReadSchemaResponse>("/v1/schema");
      return response.schema;
    },
  };
};
