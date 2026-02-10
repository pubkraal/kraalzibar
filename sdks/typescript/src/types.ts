export type CheckPermissionRequest = {
  resource_type: string;
  resource_id: string;
  permission: string;
  subject_type: string;
  subject_id: string;
  consistency?: ConsistencyRequest;
};

export type CheckPermissionResponse = {
  allowed: boolean;
  checked_at?: string;
};

export type LookupResourcesRequest = {
  resource_type: string;
  permission: string;
  subject_type: string;
  subject_id: string;
  limit?: number;
  consistency?: ConsistencyRequest;
};

export type LookupResourcesResponse = {
  resource_ids: string[];
  looked_up_at?: string;
};

export type LookupSubjectsRequest = {
  resource_type: string;
  resource_id: string;
  permission: string;
  subject_type: string;
  consistency?: ConsistencyRequest;
};

export type LookupSubjectsResponse = {
  subjects: SubjectResponse[];
  looked_up_at?: string;
};

export type SubjectResponse = {
  subject_type: string;
  subject_id: string;
  subject_relation?: string;
};

export type WriteRelationshipsRequest = {
  updates: RelationshipUpdate[];
};

export type RelationshipUpdate = {
  operation: "touch" | "delete";
  resource_type: string;
  resource_id: string;
  relation: string;
  subject_type: string;
  subject_id: string;
  subject_relation?: string;
};

export type WriteRelationshipsResponse = {
  written_at: string;
};

export type ReadRelationshipsRequest = {
  filter?: RelationshipFilter;
  limit?: number;
  consistency?: ConsistencyRequest;
};

export type RelationshipFilter = {
  resource_type?: string;
  resource_id?: string;
  relation?: string;
  subject_type?: string;
  subject_id?: string;
};

export type ReadRelationshipsResponse = {
  relationships: RelationshipResponse[];
};

export type RelationshipResponse = {
  resource_type: string;
  resource_id: string;
  relation: string;
  subject_type: string;
  subject_id: string;
  subject_relation?: string;
};

export type WriteSchemaOptions = {
  force?: boolean;
};

export type WriteSchemaResponse = {
  breaking_changes_overridden: boolean;
};

export type ReadSchemaResponse = {
  schema: string;
};

export type ConsistencyRequest =
  | { type: "full" }
  | { type: "minimize_latency" }
  | { type: "at_least_as_fresh"; token: string }
  | { type: "at_exact_snapshot"; token: string };
