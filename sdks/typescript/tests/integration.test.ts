import { describe, it, expect, beforeAll } from "vitest";
import { createClient, type KraalzibarClient } from "../src/index.js";

const TARGET = process.env.KRAALZIBAR_REST_TARGET ?? "http://localhost:8080";
const isIntegration = process.env.INTEGRATION === "true";

describe.skipIf(!isIntegration)("integration", () => {
  let client: KraalzibarClient;

  beforeAll(() => {
    client = createClient({ target: TARGET });
  });

  const writeSchema = async (schema: string) => {
    await client.writeSchema(schema, { force: true });
  };

  const touchRelationship = async (
    resourceType: string,
    resourceId: string,
    relation: string,
    subjectType: string,
    subjectId: string,
  ) => {
    await client.writeRelationships({
      updates: [
        {
          operation: "touch",
          resource_type: resourceType,
          resource_id: resourceId,
          relation,
          subject_type: subjectType,
          subject_id: subjectId,
        },
      ],
    });
  };

  const basicSchema = `definition user {}

definition document {
  relation viewer: user
  permission can_view = viewer
}`;

  it("check permission granted", async () => {
    await writeSchema(basicSchema);
    await touchRelationship("document", "readme", "viewer", "user", "alice");

    const result = await client.checkPermission({
      resource_type: "document",
      resource_id: "readme",
      permission: "can_view",
      subject_type: "user",
      subject_id: "alice",
      consistency: { type: "full" },
    });

    expect(result.allowed).toBe(true);
  });

  it("check permission denied", async () => {
    await writeSchema(basicSchema);
    await touchRelationship("document", "readme", "viewer", "user", "alice");

    const result = await client.checkPermission({
      resource_type: "document",
      resource_id: "readme",
      permission: "can_view",
      subject_type: "user",
      subject_id: "bob",
      consistency: { type: "full" },
    });

    expect(result.allowed).toBe(false);
  });

  it("write and read relationships", async () => {
    await writeSchema(basicSchema);
    await touchRelationship("document", "readme", "viewer", "user", "alice");
    await touchRelationship("document", "readme", "viewer", "user", "bob");

    const result = await client.readRelationships({
      filter: {
        resource_type: "document",
        resource_id: "readme",
        relation: "viewer",
      },
      consistency: { type: "full" },
    });

    expect(result.relationships.length).toBe(2);
  });

  it("lookup resources", async () => {
    await writeSchema(basicSchema);
    await touchRelationship("document", "doc1", "viewer", "user", "alice");
    await touchRelationship("document", "doc2", "viewer", "user", "alice");
    await touchRelationship("document", "doc3", "viewer", "user", "bob");

    const result = await client.lookupResources({
      resource_type: "document",
      permission: "can_view",
      subject_type: "user",
      subject_id: "alice",
      consistency: { type: "full" },
    });

    const ids = [...result.resource_ids].sort();
    expect(ids).toEqual(["doc1", "doc2"]);
  });

  it("lookup subjects", async () => {
    await writeSchema(`definition user {}

definition document {
  relation viewer: user
  relation owner: user
  permission can_view = viewer + owner
}`);
    await touchRelationship("document", "readme", "viewer", "user", "alice");
    await touchRelationship("document", "readme", "owner", "user", "bob");

    const result = await client.lookupSubjects({
      resource_type: "document",
      resource_id: "readme",
      permission: "can_view",
      subject_type: "user",
      consistency: { type: "full" },
    });

    const ids = result.subjects.map((s) => s.subject_id).sort();
    expect(ids).toEqual(["alice", "bob"]);
  });

  it("schema write and read", async () => {
    await writeSchema(basicSchema);

    const schema = await client.readSchema();

    expect(schema).toContain("definition user");
    expect(schema).toContain("definition document");
    expect(schema).toContain("relation viewer");
    expect(schema).toContain("permission can_view");
  });

  it("computed permission via union", async () => {
    await writeSchema(`definition user {}

definition document {
  relation viewer: user
  relation editor: user
  permission can_edit = editor
  permission can_view = viewer + can_edit
}`);
    await touchRelationship("document", "readme", "editor", "user", "alice");

    const result = await client.checkPermission({
      resource_type: "document",
      resource_id: "readme",
      permission: "can_view",
      subject_type: "user",
      subject_id: "alice",
      consistency: { type: "full" },
    });

    expect(result.allowed).toBe(true);
  });
});
