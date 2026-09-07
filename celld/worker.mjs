// Drift-owned metadata adapter. Celld owns storage durability and owner fencing.
export const MAX_METADATA_BYTES = 8 * 1024 * 1024;
const HTTP_OK = 200;
const HTTP_COMMITTED = 204;
const HTTP_BAD_REQUEST = 400;
const HTTP_UNAUTHORIZED = 401;
const HTTP_NOT_FOUND = 404;
const HTTP_METHOD_NOT_ALLOWED = 405;
const HTTP_PRECONDITION_FAILED = 412;
const HTTP_TOO_LARGE = 413;
const HTTP_UNAVAILABLE = 503;
const SCHEMA_VERSION = 1;
const STATE_PATH = "/state";
const TABLE_SCHEMA = "CREATE TABLE IF NOT EXISTS drift_snapshot (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), revision INTEGER NOT NULL, body TEXT NOT NULL)";
const READ_STATE = "SELECT revision, body FROM drift_snapshot WHERE singleton = 1";
const CREATE_STATE = "INSERT INTO drift_snapshot(singleton, revision, body) VALUES (1, 1, ?) ON CONFLICT(singleton) DO NOTHING RETURNING revision";
const UPDATE_STATE = "UPDATE drift_snapshot SET revision = revision + 1, body = ? WHERE singleton = 1 AND revision = ? AND revision < ? RETURNING revision";

function response(status, body = null, headers = {}) {
  return new Response(body, { status, headers: { "cache-control": "no-store", ...headers } });
}

function tokenMatches(actual, expected) {
  if (typeof expected !== "string" || expected.length === 0 || typeof actual !== "string") return false;
  const wanted = `Bearer ${expected}`;
  if (actual.length !== wanted.length) return false;
  let difference = 0;
  for (let index = 0; index < wanted.length; index += 1) {
    difference |= actual.charCodeAt(index) ^ wanted.charCodeAt(index);
  }
  return difference === 0;
}

export function authorized(request, env) {
  return typeof env.DRIFT_USER === "string" && env.DRIFT_USER.length > 0
    && request.headers.get("x-drift-user") === env.DRIFT_USER
    && tokenMatches(request.headers.get("authorization"), env.DRIFT_TOKEN);
}

export function precondition(headers) {
  const create = headers.get("if-none-match");
  const update = headers.get("if-match");
  if (create === "*" && update === null) return { kind: "create" };
  if (create !== null || update === null || !/^"[1-9][0-9]*"$/.test(update)) return null;
  const revision = Number(JSON.parse(update));
  if (!Number.isSafeInteger(revision) || revision < 1) return null;
  return { kind: "update", revision };
}

export function validSnapshot(body, user) {
  try {
    const state = JSON.parse(body);
    return state !== null && typeof state === "object" && !Array.isArray(state)
      && Object.keys(state).sort().join(",") === "documents,schema,user"
      && state.schema === SCHEMA_VERSION && state.user === user
      && state.documents !== null && typeof state.documents === "object"
      && !Array.isArray(state.documents);
  } catch { return false; }
}

async function readBounded(request) {
  const reader = request.body?.getReader();
  if (!reader) return "";
  const chunks = [];
  let size = 0;
  try {
    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      size += value.byteLength;
      if (size > MAX_METADATA_BYTES) {
        await reader.cancel();
        return null;
      }
      chunks.push(value);
    }
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } finally { reader.releaseLock(); }
}

export class DriftMetadata {
  constructor(state, env) {
    this.sql = state.storage.sql;
    this.env = env;
    this.sql.exec(TABLE_SCHEMA);
  }

  async fetch(request) {
    if (!authorized(request, this.env)) return response(HTTP_UNAUTHORIZED);
    const url = new URL(request.url);
    if (url.pathname !== STATE_PATH || url.search !== "") return response(HTTP_NOT_FOUND);
    if (request.method === "GET") {
      const rows = this.sql.exec(READ_STATE).toArray();
      if (rows.length === 0) return response(HTTP_NOT_FOUND, null, { "x-drift-state": "absent" });
      const row = rows[0];
      return response(HTTP_OK, row.body, { etag: `"${row.revision}"`, "content-type": "application/json" });
    }
    if (request.method !== "PUT") return response(HTTP_METHOD_NOT_ALLOWED);
    const condition = precondition(request.headers);
    if (condition === null) return response(HTTP_BAD_REQUEST);
    let body;
    try { body = await readBounded(request); } catch { return response(HTTP_BAD_REQUEST); }
    if (body === null) return response(HTTP_TOO_LARGE);
    if (!validSnapshot(body, this.env.DRIFT_USER)) return response(HTTP_BAD_REQUEST);
    // Each CAS is one atomic SQLite statement. No read/modify/write gap exists.
    const rows = condition.kind === "create"
      ? this.sql.exec(CREATE_STATE, body).toArray()
      : this.sql.exec(UPDATE_STATE, body, condition.revision, Number.MAX_SAFE_INTEGER).toArray();
    if (rows.length === 0) return response(HTTP_PRECONDITION_FAILED);
    // Celld v0.3.0 holds the response behind its durable-write/fencing gate.
    return response(HTTP_COMMITTED, null, { etag: `"${rows[0].revision}"`, "x-drift-state": "committed" });
  }
}

export default {
  async fetch(request, env) {
    if (!authorized(request, env)) return response(HTTP_UNAUTHORIZED);
    const url = new URL(request.url);
    if (url.pathname !== STATE_PATH || url.search !== "") return response(HTTP_NOT_FOUND);
    try {
      const id = env.DRIFT_METADATA.idFromName(env.DRIFT_USER);
      return await env.DRIFT_METADATA.get(id).fetch(request);
    } catch {
      // No exception body can expose a credential or masquerade as a commit.
      return response(HTTP_UNAVAILABLE);
    }
  },
};
