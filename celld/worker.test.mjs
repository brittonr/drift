import { test } from "node:test";
import assert from "node:assert/strict";
import { DatabaseSync } from "node:sqlite";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import worker, { DriftMetadata, MAX_METADATA_BYTES, precondition } from "./worker.mjs";

const USER = "alice";
const TOKEN = "test-only-token";
const OK = 200;
const COMMITTED = 204;
const BAD_REQUEST = 400;
const UNAUTHORIZED = 401;
const NOT_FOUND = 404;
const CONFLICT = 412;
const TOO_LARGE = 413;
const UNAVAILABLE = 503;
const env = { DRIFT_USER: USER, DRIFT_TOKEN: TOKEN };
const snapshot = (documents = {}) => JSON.stringify({ schema: 1, user: USER, documents });

function request(method = "GET", body, condition = {}) {
  return new Request("https://drift.example/state", {
    method, body,
    headers: { authorization: `Bearer ${TOKEN}`, "x-drift-user": USER, ...condition },
  });
}

function cell(path = ":memory:") {
  const db = new DatabaseSync(path);
  const state = { storage: { sql: { exec(sql, ...args) {
    const rows = db.prepare(sql).all(...args);
    return { toArray: () => rows };
  } } } };
  return { db, service: new DriftMetadata(state, env) };
}

test("conditional create and update reject existing and stale revisions", async () => {
  const { db, service } = cell();
  try {
    const absent = await service.fetch(request());
    assert.equal(absent.status, NOT_FOUND);
    assert.equal(absent.headers.get("x-drift-state"), "absent");
    const created = await service.fetch(request("PUT", snapshot(), { "if-none-match": "*" }));
    assert.equal(created.status, COMMITTED);
    assert.equal(created.headers.get("x-drift-state"), "committed");
    assert.equal((await service.fetch(request("PUT", snapshot(), { "if-none-match": "*" }))).status, CONFLICT);
    const first = await service.fetch(request());
    assert.equal(first.status, OK);
    const tag = first.headers.get("etag");
    assert.equal((await service.fetch(request("PUT", snapshot({ changed: true }), { "if-match": tag }))).status, COMMITTED);
    assert.equal((await service.fetch(request("PUT", snapshot({ stale: true }), { "if-match": tag }))).status, CONFLICT);
    assert.deepEqual(JSON.parse(await (await service.fetch(request())).text()).documents, { changed: true });
  } finally { db.close(); }
});

test("SQLite state and revision survive cell reconstruction", async () => {
  const directory = mkdtempSync(join(tmpdir(), "drift-metadata-"));
  const path = join(directory, "state.sqlite");
  try {
    const first = cell(path);
    const created = await first.service.fetch(request("PUT", snapshot(), { "if-none-match": "*" }));
    const tag = created.headers.get("etag");
    first.db.close();
    const second = cell(path);
    try {
      const restored = await second.service.fetch(request());
      assert.equal(restored.headers.get("etag"), tag);
      assert.equal(await restored.text(), snapshot());
      assert.equal((await second.service.fetch(request("PUT", snapshot(), { "if-none-match": "*" }))).status, CONFLICT);
    } finally { second.db.close(); }
  } finally { rmSync(directory, { recursive: true }); }
});

test("concurrent conditional writers produce exactly one commit", async () => {
  const { db, service } = cell();
  try {
    const replies = await Promise.all([
      service.fetch(request("PUT", snapshot({ first: true }), { "if-none-match": "*" })),
      service.fetch(request("PUT", snapshot({ second: true }), { "if-none-match": "*" })),
    ]);
    assert.deepEqual(replies.map(reply => reply.status).sort(), [COMMITTED, CONFLICT]);
  } finally { db.close(); }
});

test("rejects absent auth, wrong user, malformed state, and oversized bodies", async () => {
  const { db, service } = cell();
  try {
    const unauthenticated = new Request("https://drift.example/state");
    assert.equal((await service.fetch(unauthenticated)).status, UNAUTHORIZED);
    const wrongUser = request();
    wrongUser.headers.set("x-drift-user", "bob");
    assert.equal((await service.fetch(wrongUser)).status, UNAUTHORIZED);
    const wrongToken = request();
    wrongToken.headers.set("authorization", "Bearer wrong");
    assert.equal((await service.fetch(wrongToken)).status, UNAUTHORIZED);
    for (const body of ["not json", "{}", JSON.stringify({ schema: 1, user: "bob", documents: {} })]) {
      assert.equal((await service.fetch(request("PUT", body, { "if-none-match": "*" }))).status, BAD_REQUEST);
    }
    const oversized = "x".repeat(MAX_METADATA_BYTES + 1);
    assert.equal((await service.fetch(request("PUT", oversized, { "if-none-match": "*" }))).status, TOO_LARGE);
    assert.equal((await service.fetch(request())).status, NOT_FOUND);
  } finally { db.close(); }
});

test("fails closed on storage errors and absent deployment credentials", async () => {
  const failing = { ...env, DRIFT_METADATA: { idFromName: name => name, get: () => ({ fetch() { throw new Error("internal private detail"); } }) } };
  const reply = await worker.fetch(request("PUT", snapshot(), { "if-none-match": "*" }), failing);
  assert.equal(reply.status, UNAVAILABLE);
  assert.equal(await reply.text(), "");
  assert.equal(reply.headers.get("x-drift-state"), null);
  assert.equal((await worker.fetch(request(), { ...failing, DRIFT_TOKEN: "" })).status, UNAUTHORIZED);
});

test("rejects ambiguous, weak, malformed, and overflowing preconditions", () => {
  for (const headers of [
    {}, { "if-match": "*" }, { "if-match": 'W/"1"' }, { "if-match": '"01"' },
    { "if-match": '"1"', "if-none-match": "*" }, { "if-match": `"${BigInt(Number.MAX_SAFE_INTEGER) + 1n}"` },
  ]) assert.equal(precondition(new Headers(headers)), null);
  assert.deepEqual(precondition(new Headers({ "if-match": '"1"' })), { kind: "update", revision: 1 });
});
