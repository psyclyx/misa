//! Durable conversation log persisted in SQLite.
//!
//! Several Misa processes may write to the same database at once. The store
//! opens its connection in WAL mode so readers never block the writer,
//! `busy_timeout` absorbs ordinary lock contention, and every write runs in a
//! single `BEGIN IMMEDIATE` transaction whose sequence number is derived under
//! the write lock. When two writers still collide, the write is retried with a
//! bounded backoff instead of failing. A crash can therefore leave a committed
//! batch or nothing, never a partial batch or a duplicated sequence number.
//!
//! Four separate questions are answered here:
//!
//! * `messages` holds immutable, content-addressed transcript entries. The id
//!   is a BLAKE3 digest of `(kind, payload_version, canonical payload)`, so two
//!   transcripts that contain the same entry store it once. Because the payload
//!   is canonicalised (object keys sorted) the digest is stable no matter how
//!   the producing process ordered its keys.
//! * `branch_messages` is the ordered transcript of one conversation: a
//!   materialised `(seq, message_id)` edge list. Reading a transcript stays a
//!   flat indexed range scan, which a parent pointer per message would turn
//!   into a recursive walk. Forking copies only these ~40 byte edges, never the
//!   payloads.
//! * `conversations` is the header: timestamps, an opaque JSON `metadata`
//!   document, and the fork provenance of this branch.
//! * `provider_requests` is append-only cost accounting: exactly one row per
//!   provider request, whether or not it is ever linked to a message. A request
//!   is written when it starts and enriched when it finishes, and recording it
//!   never depends on a transcript existing.
//!
//! Cost is stored as typed integer columns, not only JSON, so it can be
//! aggregated:
//!
//! * `cost_micros` is USD scaled by 1e6. Never a float: summing money in
//!   binary floating point accumulates error, and a local database can hold
//!   millions of rows.
//! * `cost_kind` distinguishes the three states the cost layer already
//!   produces: `reported` (the provider told us), `estimated` (derived from
//!   pricing), `unknown` (no pricing available) and `pending` (not finished).
//!   Collapsing these into a single nullable number would silently report an
//!   estimate as if it were billed.
//! * `parent_request_id` groups requests a single logical turn fanned out into
//!   (subagents, retries, fallbacks) so their spend can be rolled up once.
//!
//! Spend and context are different questions and the schema keeps them apart.
//! `provider_requests.conversation_id` records the branch that *issued* a
//! request, so summing a branch's spend never double counts a fork. A message
//! copied into a fork carries the `request_id` that produced it in that branch,
//! so the cost of a branch's *context* is the sum over the requests that
//! actually made up its transcript.

const std = @import("std");
const c = @cImport(@cInclude("sqlite3.h"));

pub const max_conversation_id_bytes: usize = 128;
pub const max_kind_bytes: usize = 64;
pub const max_request_id_bytes: usize = 256;
pub const max_label_bytes: usize = 256;
pub const max_entry_bytes: usize = 1024 * 1024;
pub const max_batch_bytes: usize = 4 * 1024 * 1024;
pub const max_entries_per_append: usize = 256;
pub const max_load_limit: usize = 1024;
pub const max_requests_per_load: usize = 1024;
pub const max_document_bytes: usize = 64 * 1024;
pub const max_document_depth: usize = 32;
pub const max_list_limit: usize = 256;
pub const busy_timeout_ms: c_int = 5000;
pub const max_write_attempts: usize = 8;

pub const schema_version: c_int = 3;
const retry_backoff_ms: usize = 25;

/// Payload revision understood by this binary. A payload written with a newer
/// version must be refused rather than reinterpreted, because `payload` is
/// deliberately opaque to the store: no schema migration can rewrite bytes the
/// store does not understand.
pub const max_payload_version: u32 = 1;

pub const cost_kinds = [_][]const u8{ "pending", "reported", "estimated", "unknown" };

/// One transcript entry. `data` is any finite JSON value; it is canonicalized
/// and serialized to text at the storage boundary so the schema never depends
/// on transcript shape. `request_id` links the entry to the provider request
/// that produced it.
pub const Entry = struct {
    kind: []const u8,
    payload_version: u32 = 1,
    data: std.json.Value,
    request_id: ?[]const u8 = null,
};

/// One provider request, recorded for cost accounting. `conversation` is the
/// branch that issued it, not a requirement: a request is captured even when it
/// has no conversation and no linked message.
pub const Request = struct {
    id: []const u8,
    conversation: ?[]const u8 = null,
    message_id: ?[]const u8 = null,
    provider_id: ?[]const u8 = null,
    parent_request_id: ?[]const u8 = null,
    provider: []const u8,
    model: []const u8,
    status: []const u8,
    cost_kind: []const u8 = "pending",
    cost_micros: ?i64 = null,
    cost_currency: []const u8 = "USD",
    input_tokens: ?i64 = null,
    output_tokens: ?i64 = null,
    cache_read_tokens: ?i64 = null,
    cache_write_tokens: ?i64 = null,
    ttft_ms: ?i64 = null,
    finished_at_ms: ?i64 = null,
    settings: ?std.json.Value = null,
    usage: ?std.json.Value = null,
    cost: ?std.json.Value = null,
    metadata: ?std.json.Value = null,
};

/// A validated transcript append, shared by the native effect and the store.
pub const Append = struct {
    conversation: []const u8,
    entries: []const Entry,
    metadata: ?std.json.Value = null,
    request: ?Request = null,
    at_ms: i64,
};

/// A validated fork request: copy `from`'s first `at_seq` entries into a new
/// conversation. Forking is only legal at a message boundary, so a fork can
/// never split a turn.
pub const Fork = struct {
    conversation: []const u8,
    from: []const u8,
    at_seq: i64,
    at_ms: i64,
    metadata: ?std.json.Value = null,
};

/// A validated transcript append request as produced by the native effect layer.
pub const Spec = struct {
    conversation: []const u8,
    entries: []const Entry,
    metadata: ?std.json.Value = null,
    completion: []const u8,
    id: []const u8,
};

pub const Record = struct {
    seq: i64,
    at_ms: i64,
    message_id: []const u8,
    kind: []const u8,
    payload_version: u32,
    payload: []const u8,
    request_id: ?[]const u8,
};

pub const RequestRecord = struct {
    id: []const u8,
    conversation: ?[]const u8,
    message_id: ?[]const u8,
    provider_id: ?[]const u8,
    parent_request_id: ?[]const u8,
    provider: []const u8,
    model: []const u8,
    status: []const u8,
    cost_kind: []const u8,
    cost_micros: ?i64,
    cost_currency: []const u8,
    input_tokens: ?i64,
    output_tokens: ?i64,
    cache_read_tokens: ?i64,
    cache_write_tokens: ?i64,
    ttft_ms: ?i64,
    finished_at_ms: ?i64,
    settings: ?[]const u8,
    usage: ?[]const u8,
    cost: ?[]const u8,
    metadata: ?[]const u8,
    created_at_ms: i64,
    updated_at_ms: i64,
};

/// One row of the conversation index, without its transcript.
pub const Summary = struct {
    id: []const u8,
    created_at: i64,
    updated_at: i64,
    metadata: []const u8,
    forked_from_id: ?[]const u8,
    forked_from_seq: ?i64,
    entry_count: i64,
};

/// A reopened conversation. `conversation` borrows the caller's argument; every
/// other slice is owned by the allocator passed to `load`. `more_entries` and
/// `more_requests` report whether the page was truncated.
pub const Snapshot = struct {
    conversation: []const u8,
    metadata: []const u8,
    forked_from_id: ?[]const u8,
    forked_from_seq: ?i64,
    entries: []Record,
    requests: []RequestRecord,
    more_entries: bool,
    more_requests: bool,
};

/// A stored binary object (image bytes, and similar). Content addressed, so
/// pasting the same screenshot twice costs one row.
pub const Blob = struct {
    hash: []const u8,
    mime: []const u8,
    byte_count: i64,
    bytes: []const u8,
};

/// SQLite's `SQLITE_TRANSIENT`: copy bound text, so callers keep ownership.
const transient = @as(c.sqlite3_destructor_type, @ptrFromInt(std.math.maxInt(usize)));

const tables = "CREATE TABLE IF NOT EXISTS conversations (" ++
    "id TEXT PRIMARY KEY, " ++
    "created_at INTEGER NOT NULL, " ++
    "updated_at INTEGER NOT NULL, " ++
    "metadata TEXT NOT NULL DEFAULT '{}', " ++
    "forked_from_id TEXT REFERENCES conversations(id) ON DELETE SET NULL, " ++
    "forked_from_seq INTEGER) WITHOUT ROWID;" ++
    "CREATE TABLE IF NOT EXISTS messages (" ++
    "id TEXT PRIMARY KEY, " ++
    "kind TEXT NOT NULL, " ++
    "payload_version INTEGER NOT NULL, " ++
    "payload TEXT NOT NULL, " ++
    "created_at_ms INTEGER NOT NULL) WITHOUT ROWID;" ++
    "CREATE TABLE IF NOT EXISTS branch_messages (" ++
    "conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE, " ++
    "seq INTEGER NOT NULL, " ++
    "message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE, " ++
    "at_ms INTEGER NOT NULL, " ++
    "request_id TEXT, " ++
    "PRIMARY KEY (conversation_id, seq)) WITHOUT ROWID;" ++
    "CREATE TABLE IF NOT EXISTS provider_requests (" ++
    "id TEXT PRIMARY KEY, " ++
    "conversation_id TEXT REFERENCES conversations(id) ON DELETE SET NULL, " ++
    "message_id TEXT REFERENCES messages(id) ON DELETE SET NULL, " ++
    "provider_id TEXT, " ++
    "parent_request_id TEXT REFERENCES provider_requests(id) ON DELETE SET NULL, " ++
    "created_at_ms INTEGER NOT NULL, " ++
    "updated_at_ms INTEGER NOT NULL, " ++
    "finished_at_ms INTEGER, " ++
    "provider TEXT NOT NULL, " ++
    "model TEXT NOT NULL, " ++
    "status TEXT NOT NULL, " ++
    "cost_kind TEXT NOT NULL DEFAULT 'pending', " ++
    "cost_micros INTEGER, " ++
    "cost_currency TEXT NOT NULL DEFAULT 'USD', " ++
    "input_tokens INTEGER, " ++
    "output_tokens INTEGER, " ++
    "cache_read_tokens INTEGER, " ++
    "cache_write_tokens INTEGER, " ++
    "ttft_ms INTEGER, " ++
    "settings TEXT, " ++
    "usage TEXT, " ++
    "cost TEXT, " ++
    "metadata TEXT) WITHOUT ROWID;" ++
    "CREATE TABLE IF NOT EXISTS blobs (" ++
    "hash TEXT PRIMARY KEY, " ++
    "mime TEXT NOT NULL, " ++
    "byte_count INTEGER NOT NULL, " ++
    "bytes BLOB NOT NULL) WITHOUT ROWID;";

/// Indexes are separate from tables because an upgrade must add missing
/// *columns* before an index can reference them: a version 1 database has no
/// `provider_requests` at all, and a version 2 one has no `parent_request_id`.
const indexes = "CREATE INDEX IF NOT EXISTS conversations_by_updated ON conversations(updated_at);" ++
    "CREATE INDEX IF NOT EXISTS branch_messages_by_message ON branch_messages(message_id);" ++
    "CREATE INDEX IF NOT EXISTS branch_messages_by_time ON branch_messages(conversation_id, at_ms);" ++
    "CREATE INDEX IF NOT EXISTS branch_messages_by_request ON branch_messages(request_id);" ++
    "CREATE INDEX IF NOT EXISTS provider_requests_by_conversation " ++
    "ON provider_requests(conversation_id, created_at_ms);" ++
    "CREATE INDEX IF NOT EXISTS provider_requests_by_model " ++
    "ON provider_requests(provider, model, created_at_ms);" ++
    "CREATE INDEX IF NOT EXISTS provider_requests_by_parent " ++
    "ON provider_requests(parent_request_id);";

/// Columns added to `provider_requests` after it was first introduced. Adding
/// them one at a time keeps the step idempotent whether the table was created
/// by version 1 (absent), version 2 (untyped) or this version (complete).
const request_columns = [_]struct { name: []const u8, definition: []const u8 }{
    .{ .name = "message_id", .definition = "TEXT" },
    .{ .name = "provider_id", .definition = "TEXT" },
    .{ .name = "parent_request_id", .definition = "TEXT" },
    .{ .name = "finished_at_ms", .definition = "INTEGER" },
    .{ .name = "cost_kind", .definition = "TEXT NOT NULL DEFAULT 'pending'" },
    .{ .name = "cost_micros", .definition = "INTEGER" },
    .{ .name = "cost_currency", .definition = "TEXT NOT NULL DEFAULT 'USD'" },
    .{ .name = "input_tokens", .definition = "INTEGER" },
    .{ .name = "output_tokens", .definition = "INTEGER" },
    .{ .name = "cache_read_tokens", .definition = "INTEGER" },
    .{ .name = "cache_write_tokens", .definition = "INTEGER" },
    .{ .name = "ttft_ms", .definition = "INTEGER" },
};

pub const Store = struct {
    allocator: std.mem.Allocator,
    io: std.Io,
    path: [:0]u8,
    db: *c.sqlite3,

    pub fn open(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map) !Store {
        const path = try databasePath(allocator, environ);
        errdefer allocator.free(path);
        const protect_parent = environ.get("MISA_CONVERSATION_DB") == null;
        try prepareDirectory(io, path, protect_parent);

        var handle: ?*c.sqlite3 = null;
        const flags = c.SQLITE_OPEN_READWRITE | c.SQLITE_OPEN_CREATE | c.SQLITE_OPEN_NOMUTEX;
        if (c.sqlite3_open_v2(path.ptr, &handle, flags, null) != c.SQLITE_OK) {
            if (handle) |db| _ = c.sqlite3_close_v2(db);
            return error.DatabaseOpenFailed;
        }
        const db = handle.?;
        errdefer _ = c.sqlite3_close_v2(db);

        var store: Store = .{
            .allocator = allocator,
            .io = io,
            .path = path,
            .db = db,
        };
        if (c.sqlite3_busy_timeout(db, busy_timeout_ms) != c.SQLITE_OK) return error.DatabaseError;
        // These are per-connection and never take a database lock. WAL is
        // persistent database state, established once by the creating
        // transaction in `migrate` instead of on every open.
        try store.exec("PRAGMA synchronous=NORMAL;");
        try store.exec("PRAGMA foreign_keys=ON;");
        try store.migrate();
        return store;
    }

    pub fn deinit(self: *Store) void {
        _ = c.sqlite3_close_v2(self.db);
        self.allocator.free(self.path);
    }

    /// Append one ordered batch to a conversation, creating it if needed.
    /// Returns the sequence number of the last entry written.
    pub fn append(self: *Store, request_append: Append) !i64 {
        try validateConversationId(request_append.conversation);
        if (request_append.entries.len == 0 or request_append.entries.len > max_entries_per_append) return error.InvalidBatch;
        for (request_append.entries) |entry| {
            try validateKind(entry.kind);
            try validatePayloadVersion(entry.payload_version);
            if (entry.request_id) |id| try validateRequestId(id);
            try validateDocument(entry.data);
        }
        if (request_append.metadata) |metadata| {
            if (metadata != .object) return error.InvalidMetadata;
            try validateDocument(metadata);
        }
        if (request_append.request) |request| try validateRequest(request);

        var attempt: usize = 0;
        while (attempt < max_write_attempts) : (attempt += 1) {
            if (self.appendOnce(request_append)) |last| {
                return last;
            } else |err| {
                // busy_timeout already waited; a remaining busy result means
                // another writer held the lock across the timeout boundary.
                if (err != error.DatabaseBusy) return err;
                std.Io.sleep(self.io, .fromMilliseconds(retry_backoff_ms), .awake) catch {};
            }
        }
        return error.DatabaseBusy;
    }

    /// Record or enrich one provider request without touching the transcript.
    /// Every provider request is captured here even when it has no conversation
    /// and no linked message; `conversation` only groups it for accounting.
    pub fn recordRequest(self: *Store, request: Request, now_ms: i64) !void {
        try validateRequest(request);

        var attempt: usize = 0;
        while (attempt < max_write_attempts) : (attempt += 1) {
            if (self.recordRequestOnce(request, now_ms)) |_| {
                return;
            } else |err| {
                if (err != error.DatabaseBusy) return err;
                std.Io.sleep(self.io, .fromMilliseconds(retry_backoff_ms), .awake) catch {};
            }
        }
        return error.DatabaseBusy;
    }

    /// Branch a conversation at a message boundary. The new conversation gets
    /// its own header and its own copy of the ordered edges up to `at_seq`;
    /// message payloads stay shared. Returns the last sequence copied.
    pub fn fork(self: *Store, request_fork: Fork) !i64 {
        try validateConversationId(request_fork.conversation);
        try validateConversationId(request_fork.from);
        if (std.mem.eql(u8, request_fork.conversation, request_fork.from)) return error.InvalidFork;
        if (request_fork.at_seq <= 0) return error.InvalidFork;
        if (request_fork.metadata) |metadata| {
            if (metadata != .object) return error.InvalidMetadata;
            try validateDocument(metadata);
        }

        var attempt: usize = 0;
        while (attempt < max_write_attempts) : (attempt += 1) {
            if (self.forkOnce(request_fork)) |last| {
                return last;
            } else |err| {
                if (err != error.DatabaseBusy) return err;
                std.Io.sleep(self.io, .fromMilliseconds(retry_backoff_ms), .awake) catch {};
            }
        }
        return error.DatabaseBusy;
    }

    /// Index a conversation's transcript, newest first. This is what a resume
    /// picker lists.
    pub fn list(self: *Store, allocator: std.mem.Allocator, limit: usize) ![]Summary {
        const bounded = @min(limit, max_list_limit);
        var summaries: std.ArrayList(Summary) = .empty;
        errdefer freeSummaries(allocator, summaries.items);
        const stmt = try self.prepare(
            "SELECT c.id, c.created_at, c.updated_at, c.metadata, c.forked_from_id, c.forked_from_seq, " ++
                "(SELECT COUNT(*) FROM branch_messages m WHERE m.conversation_id = c.id) " ++
                "FROM conversations c ORDER BY c.updated_at DESC, c.id LIMIT ?1;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindInt(stmt, 1, @intCast(bounded));
        while (true) {
            const code = c.sqlite3_step(stmt);
            if (code == c.SQLITE_DONE) break;
            if (code != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
            const id_text = c.sqlite3_column_text(stmt, 0) orelse return error.DatabaseError;
            const metadata_text = c.sqlite3_column_text(stmt, 3) orelse return error.DatabaseError;
            const id = try allocator.dupe(u8, id_text[0..@intCast(c.sqlite3_column_bytes(stmt, 0))]);
            errdefer allocator.free(id);
            const metadata = try allocator.dupe(u8, metadata_text[0..@intCast(c.sqlite3_column_bytes(stmt, 3))]);
            errdefer allocator.free(metadata);
            const forked_from = try columnText(allocator, stmt, 4);
            errdefer if (forked_from) |value| allocator.free(value);
            try summaries.append(allocator, .{
                .id = id,
                .created_at = c.sqlite3_column_int64(stmt, 1),
                .updated_at = c.sqlite3_column_int64(stmt, 2),
                .metadata = metadata,
                .forked_from_id = forked_from,
                .forked_from_seq = if (c.sqlite3_column_type(stmt, 5) == c.SQLITE_NULL)
                    null
                else
                    c.sqlite3_column_int64(stmt, 5),
                .entry_count = c.sqlite3_column_int64(stmt, 6),
            });
        }
        return summaries.toOwnedSlice(allocator);
    }

    /// Reopen a conversation: header metadata, a page of transcript entries in
    /// ascending order, and the provider requests grouped under it.
    pub fn load(
        self: *Store,
        allocator: std.mem.Allocator,
        conversation: []const u8,
        after_seq: i64,
        entry_limit: usize,
        request_limit: usize,
    ) !Snapshot {
        try validateConversationId(conversation);
        var header = try self.loadHeader(allocator, conversation);
        errdefer header.deinit(allocator);

        const bounded_entries = @min(entry_limit, max_load_limit);
        const bounded_requests = @min(request_limit, max_requests_per_load);
        var more_entries = false;
        var entries = try self.loadEntries(allocator, conversation, after_seq, bounded_entries, &more_entries);
        errdefer freeRecords(allocator, entries.items);
        var more_requests = false;
        var requests = try self.loadRequests(allocator, conversation, bounded_requests, &more_requests);
        errdefer freeRequestRecords(allocator, requests.items);
        const owned_entries = try entries.toOwnedSlice(allocator);
        errdefer freeRecords(allocator, owned_entries);
        const owned_requests = try requests.toOwnedSlice(allocator);
        return .{
            .conversation = conversation,
            .metadata = header.metadata,
            .forked_from_id = header.forked_from_id,
            .forked_from_seq = header.forked_from_seq,
            .entries = owned_entries,
            .requests = owned_requests,
            .more_entries = more_entries,
            .more_requests = more_requests,
        };
    }

    /// Store bytes under their content hash and return the hash. Storing the
    /// same bytes twice is a no-op.
    pub fn putBlob(self: *Store, allocator: std.mem.Allocator, mime: []const u8, bytes: []const u8) ![]u8 {
        try validateLabel(mime);
        if (bytes.len == 0 or bytes.len > 64 * 1024 * 1024) return error.InvalidBlob;
        const hash = try blobHash(allocator, bytes);
        errdefer allocator.free(hash);

        var attempt: usize = 0;
        while (attempt < max_write_attempts) : (attempt += 1) {
            if (self.putBlobOnce(hash, mime, bytes)) |_| {
                return hash;
            } else |err| {
                if (err != error.DatabaseBusy) return err;
                std.Io.sleep(self.io, .fromMilliseconds(retry_backoff_ms), .awake) catch {};
            }
        }
        return error.DatabaseBusy;
    }

    pub fn getBlob(self: *Store, allocator: std.mem.Allocator, hash: []const u8) !?Blob {
        try validateLabel(hash);
        const stmt = try self.prepare("SELECT mime, byte_count, bytes FROM blobs WHERE hash = ?1;");
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, hash);
        const code = c.sqlite3_step(stmt);
        if (code == c.SQLITE_DONE) return null;
        if (code != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
        const mime_text = c.sqlite3_column_text(stmt, 0) orelse return error.DatabaseError;
        const blob_bytes = c.sqlite3_column_blob(stmt, 2) orelse return error.DatabaseError;
        const blob_len: usize = @intCast(c.sqlite3_column_bytes(stmt, 2));
        return .{
            .hash = try allocator.dupe(u8, hash),
            .mime = try allocator.dupe(u8, mime_text[0..@intCast(c.sqlite3_column_bytes(stmt, 0))]),
            .byte_count = c.sqlite3_column_int64(stmt, 1),
            .bytes = try allocator.dupe(u8, @as([*]const u8, @ptrCast(blob_bytes))[0..blob_len]),
        };
    }

    const Header = struct {
        metadata: []u8,
        forked_from_id: ?[]u8,
        forked_from_seq: ?i64,

        fn deinit(self: Header, allocator: std.mem.Allocator) void {
            allocator.free(self.metadata);
            if (self.forked_from_id) |value| allocator.free(value);
        }
    };

    fn loadHeader(self: *Store, allocator: std.mem.Allocator, conversation: []const u8) !Header {
        const stmt = try self.prepare("SELECT metadata, forked_from_id, forked_from_seq FROM conversations WHERE id = ?1;");
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        const code = c.sqlite3_step(stmt);
        if (code == c.SQLITE_DONE) return error.ConversationNotFound;
        if (code != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
        const text = c.sqlite3_column_text(stmt, 0) orelse return error.DatabaseError;
        return .{
            .metadata = try allocator.dupe(u8, text[0..@intCast(c.sqlite3_column_bytes(stmt, 0))]),
            .forked_from_id = try columnText(allocator, stmt, 1),
            .forked_from_seq = if (c.sqlite3_column_type(stmt, 2) == c.SQLITE_NULL)
                null
            else
                c.sqlite3_column_int64(stmt, 2),
        };
    }

    fn loadEntries(
        self: *Store,
        allocator: std.mem.Allocator,
        conversation: []const u8,
        after_seq: i64,
        limit: usize,
        more: *bool,
    ) !std.ArrayList(Record) {
        // Fetch one extra row so `more` is exact rather than a guess.
        var records: std.ArrayList(Record) = .empty;
        errdefer freeRecords(allocator, records.items);
        const stmt = try self.prepare(
            "SELECT m.seq, m.at_ms, m.message_id, s.kind, s.payload_version, s.payload, m.request_id " ++
                "FROM branch_messages m JOIN messages s ON s.id = m.message_id " ++
                "WHERE m.conversation_id = ?1 AND m.seq > ?2 ORDER BY m.seq LIMIT ?3;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        try bindInt(stmt, 2, after_seq);
        try bindInt(stmt, 3, @intCast(limit + 1));
        while (true) {
            const code = c.sqlite3_step(stmt);
            if (code == c.SQLITE_DONE) break;
            if (code != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
            if (records.items.len == limit) {
                more.* = true;
                break;
            }
            const message_id = try columnRequiredText(allocator, stmt, 2);
            errdefer allocator.free(message_id);
            const kind = try columnRequiredText(allocator, stmt, 3);
            errdefer allocator.free(kind);
            const payload = try columnRequiredText(allocator, stmt, 5);
            errdefer allocator.free(payload);
            const request_id = try columnText(allocator, stmt, 6);
            errdefer if (request_id) |id| allocator.free(id);
            try records.append(allocator, .{
                .seq = c.sqlite3_column_int64(stmt, 0),
                .at_ms = c.sqlite3_column_int64(stmt, 1),
                .message_id = message_id,
                .kind = kind,
                .payload_version = @intCast(c.sqlite3_column_int64(stmt, 4)),
                .payload = payload,
                .request_id = request_id,
            });
        }
        return records;
    }

    fn loadRequests(
        self: *Store,
        allocator: std.mem.Allocator,
        conversation: []const u8,
        limit: usize,
        more: *bool,
    ) !std.ArrayList(RequestRecord) {
        var records: std.ArrayList(RequestRecord) = .empty;
        errdefer freeRequestRecords(allocator, records.items);
        const stmt = try self.prepare(
            "SELECT id, conversation_id, message_id, provider_id, parent_request_id, provider, model, status, " ++
                "cost_kind, cost_micros, cost_currency, input_tokens, output_tokens, cache_read_tokens, " ++
                "cache_write_tokens, ttft_ms, finished_at_ms, settings, usage, cost, metadata, created_at_ms, updated_at_ms " ++
                "FROM provider_requests WHERE conversation_id = ?1 ORDER BY created_at_ms, id LIMIT ?2;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        try bindInt(stmt, 2, @intCast(limit + 1));
        while (true) {
            const code = c.sqlite3_step(stmt);
            if (code == c.SQLITE_DONE) break;
            if (code != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
            if (records.items.len == limit) {
                more.* = true;
                break;
            }
            const id = try columnRequiredText(allocator, stmt, 0);
            errdefer allocator.free(id);
            const provider = try columnRequiredText(allocator, stmt, 5);
            errdefer allocator.free(provider);
            const model = try columnRequiredText(allocator, stmt, 6);
            errdefer allocator.free(model);
            const status = try columnRequiredText(allocator, stmt, 7);
            errdefer allocator.free(status);
            const cost_kind = try columnRequiredText(allocator, stmt, 8);
            errdefer allocator.free(cost_kind);
            const cost_currency = try columnRequiredText(allocator, stmt, 10);
            errdefer allocator.free(cost_currency);
            try records.append(allocator, .{
                .id = id,
                .conversation = try columnText(allocator, stmt, 1),
                .message_id = try columnText(allocator, stmt, 2),
                .provider_id = try columnText(allocator, stmt, 3),
                .parent_request_id = try columnText(allocator, stmt, 4),
                .provider = provider,
                .model = model,
                .status = status,
                .cost_kind = cost_kind,
                .cost_micros = columnOptionalInt(stmt, 9),
                .cost_currency = cost_currency,
                .input_tokens = columnOptionalInt(stmt, 11),
                .output_tokens = columnOptionalInt(stmt, 12),
                .cache_read_tokens = columnOptionalInt(stmt, 13),
                .cache_write_tokens = columnOptionalInt(stmt, 14),
                .ttft_ms = columnOptionalInt(stmt, 15),
                .finished_at_ms = columnOptionalInt(stmt, 16),
                .settings = try columnText(allocator, stmt, 17),
                .usage = try columnText(allocator, stmt, 18),
                .cost = try columnText(allocator, stmt, 19),
                .metadata = try columnText(allocator, stmt, 20),
                .created_at_ms = c.sqlite3_column_int64(stmt, 21),
                .updated_at_ms = c.sqlite3_column_int64(stmt, 22),
            });
        }
        return records;
    }

    fn appendOnce(self: *Store, request_append: Append) !i64 {
        try self.exec("BEGIN IMMEDIATE;");
        errdefer self.rollbackQuietly();

        try self.upsertConversation(request_append.conversation, request_append.at_ms);
        if (request_append.metadata) |metadata| try self.mergeMetadata(request_append.conversation, metadata);
        if (request_append.request) |request| try self.writeRequest(request_append.conversation, request, request_append.at_ms, null);

        var seq: i64 = 0;
        {
            const stmt = try self.prepare("SELECT COALESCE(MAX(seq), 0) FROM branch_messages WHERE conversation_id = ?1;");
            defer _ = c.sqlite3_finalize(stmt);
            try bindText(stmt, 1, request_append.conversation);
            if (c.sqlite3_step(stmt) != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
            seq = c.sqlite3_column_int64(stmt, 0);
        }

        var batch_bytes: usize = 0;
        for (request_append.entries) |entry| {
            const message = try self.encodeMessage(entry, request_append.at_ms);
            defer message.deinit(self.allocator);
            batch_bytes += message.payload.len;
            if (batch_bytes > max_batch_bytes) return error.EntryTooLarge;
            seq += 1;
            try self.insertMessage(message);
            try self.insertEdge(request_append.conversation, seq, message.id, request_append.at_ms, entry.request_id);
            // Name the message a request produced, so cost can be attributed to
            // content as well as to the branch that issued it.
            if (entry.request_id) |request_id| {
                try self.linkRequestMessage(request_id, message.id);
            }
        }

        try self.exec("COMMIT;");
        return seq;
    }

    fn forkOnce(self: *Store, request_fork: Fork) !i64 {
        try self.exec("BEGIN IMMEDIATE;");
        errdefer self.rollbackQuietly();

        if (!try self.conversationExists(request_fork.from)) return error.ConversationNotFound;
        if (try self.conversationExists(request_fork.conversation)) return error.ConversationExists;

        var available: i64 = 0;
        {
            const stmt = try self.prepare("SELECT COALESCE(MAX(seq), 0) FROM branch_messages WHERE conversation_id = ?1;");
            defer _ = c.sqlite3_finalize(stmt);
            try bindText(stmt, 1, request_fork.from);
            if (c.sqlite3_step(stmt) != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
            available = c.sqlite3_column_int64(stmt, 0);
        }
        if (request_fork.at_seq > available) return error.InvalidFork;

        const metadata = if (request_fork.metadata) |document|
            try self.encodeDocument(document)
        else
            null;
        defer if (metadata) |text| self.allocator.free(text);

        {
            const stmt = try self.prepare(
                "INSERT INTO conversations(id, created_at, updated_at, metadata, forked_from_id, forked_from_seq) " ++
                    "VALUES(?1, ?2, ?2, COALESCE(?3, '{}'), ?4, ?5);",
            );
            defer _ = c.sqlite3_finalize(stmt);
            try bindText(stmt, 1, request_fork.conversation);
            try bindInt(stmt, 2, request_fork.at_ms);
            try bindNullableText(stmt, 3, metadata);
            try bindText(stmt, 4, request_fork.from);
            try bindInt(stmt, 5, request_fork.at_seq);
            if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
        }

        {
            const stmt = try self.prepare(
                "INSERT INTO branch_messages(conversation_id, seq, message_id, at_ms, request_id) " ++
                    "SELECT ?1, seq, message_id, at_ms, request_id FROM branch_messages " ++
                    "WHERE conversation_id = ?2 AND seq <= ?3;",
            );
            defer _ = c.sqlite3_finalize(stmt);
            try bindText(stmt, 1, request_fork.conversation);
            try bindText(stmt, 2, request_fork.from);
            try bindInt(stmt, 3, request_fork.at_seq);
            if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
        }

        try self.exec("COMMIT;");
        return request_fork.at_seq;
    }

    fn conversationExists(self: *Store, conversation: []const u8) !bool {
        const stmt = try self.prepare("SELECT 1 FROM conversations WHERE id = ?1;");
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        const code = c.sqlite3_step(stmt);
        if (code == c.SQLITE_ROW) return true;
        if (code == c.SQLITE_DONE) return false;
        return errorFromCode(c.sqlite3_errcode(self.db));
    }

    fn putBlobOnce(self: *Store, hash: []const u8, mime: []const u8, bytes: []const u8) !void {
        try self.exec("BEGIN IMMEDIATE;");
        errdefer self.rollbackQuietly();
        const stmt = try self.prepare(
            "INSERT INTO blobs(hash, mime, byte_count, bytes) VALUES(?1, ?2, ?3, ?4) " ++
                "ON CONFLICT(hash) DO NOTHING;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, hash);
        try bindText(stmt, 2, mime);
        try bindInt(stmt, 3, @intCast(bytes.len));
        if (c.sqlite3_bind_blob(stmt, 4, bytes.ptr, @intCast(bytes.len), transient) != c.SQLITE_OK) {
            return error.DatabaseError;
        }
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
        try self.exec("COMMIT;");
    }

    const Message = struct {
        id: []u8,
        kind: []const u8,
        payload_version: u32,
        payload: []u8,
        at_ms: i64,

        fn deinit(self: Message, allocator: std.mem.Allocator) void {
            allocator.free(self.id);
            allocator.free(self.payload);
        }
    };

    /// Canonicalize an entry's payload, hash it with its kind and payload
    /// version, and return the addressable message.
    fn encodeMessage(self: *Store, entry: Entry, at_ms: i64) !Message {
        const payload = try self.encodeDocument(entry.data);
        errdefer self.allocator.free(payload);
        if (payload.len > max_entry_bytes) return error.EntryTooLarge;
        const id = try messageId(self.allocator, entry.kind, entry.payload_version, payload);
        return .{ .id = id, .kind = entry.kind, .payload_version = entry.payload_version, .payload = payload, .at_ms = at_ms };
    }

    fn insertMessage(self: *Store, message: Message) !void {
        const stmt = try self.prepare(
            "INSERT INTO messages(id, kind, payload_version, payload, created_at_ms) VALUES(?1, ?2, ?3, ?4, ?5) " ++
                "ON CONFLICT(id) DO NOTHING;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, message.id);
        try bindText(stmt, 2, message.kind);
        try bindInt(stmt, 3, @intCast(message.payload_version));
        try bindText(stmt, 4, message.payload);
        try bindInt(stmt, 5, message.at_ms);
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
    }

    fn insertEdge(self: *Store, conversation: []const u8, seq: i64, message_id: []const u8, at_ms: i64, request_id: ?[]const u8) !void {
        const stmt = try self.prepare(
            "INSERT INTO branch_messages(conversation_id, seq, message_id, at_ms, request_id) VALUES(?1, ?2, ?3, ?4, ?5);",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        try bindInt(stmt, 2, seq);
        try bindText(stmt, 3, message_id);
        try bindInt(stmt, 4, at_ms);
        try bindNullableText(stmt, 5, request_id);
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
    }

    fn linkRequestMessage(self: *Store, request_id: []const u8, message_id: []const u8) !void {
        const stmt = try self.prepare(
            "UPDATE provider_requests SET message_id = ?2 WHERE id = ?1 AND message_id IS NULL;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, request_id);
        try bindText(stmt, 2, message_id);
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
    }

    fn recordRequestOnce(self: *Store, request: Request, now_ms: i64) !void {
        try self.exec("BEGIN IMMEDIATE;");
        errdefer self.rollbackQuietly();
        if (request.conversation) |conversation| try self.upsertConversation(conversation, now_ms);
        try self.writeRequest(null, request, now_ms, now_ms);
        try self.exec("COMMIT;");
    }

    /// Insert a request, or enrich the existing row. Omitted fields keep their
    /// earlier value so a start-of-request write and an end-of-request write
    /// compose without either call having to repeat the other's data.
    fn writeRequest(self: *Store, default_conversation: ?[]const u8, request: Request, created_ms: i64, now_ms: ?i64) !void {
        const settings = try self.encodeOptional(request.settings);
        defer if (settings) |text| self.allocator.free(text);
        const usage = try self.encodeOptional(request.usage);
        defer if (usage) |text| self.allocator.free(text);
        const cost = try self.encodeOptional(request.cost);
        defer if (cost) |text| self.allocator.free(text);
        const metadata = try self.encodeOptional(request.metadata);
        defer if (metadata) |text| self.allocator.free(text);

        const stmt = try self.prepare(
            "INSERT INTO provider_requests(id, conversation_id, message_id, provider_id, parent_request_id, created_at_ms, " ++
                "updated_at_ms, finished_at_ms, provider, model, status, cost_kind, cost_micros, cost_currency, " ++
                "input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, ttft_ms, settings, usage, cost, metadata) " ++
                "VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23) " ++
                "ON CONFLICT(id) DO UPDATE SET " ++
                "conversation_id=COALESCE(provider_requests.conversation_id, excluded.conversation_id), " ++
                "message_id=COALESCE(provider_requests.message_id, excluded.message_id), " ++
                "provider_id=COALESCE(excluded.provider_id, provider_requests.provider_id), " ++
                "parent_request_id=COALESCE(excluded.parent_request_id, provider_requests.parent_request_id), " ++
                "updated_at_ms=excluded.updated_at_ms, " ++
                "finished_at_ms=COALESCE(excluded.finished_at_ms, provider_requests.finished_at_ms), " ++
                "provider=excluded.provider, " ++
                "model=excluded.model, " ++
                "status=excluded.status, " ++
                // A pending write must not erase a settled cost kind, and a
                // later estimate must not erase a provider-reported figure.
                "cost_kind=CASE WHEN provider_requests.cost_micros IS NOT NULL AND provider_requests.cost_kind <> 'pending' " ++
                "AND excluded.cost_kind = 'pending' THEN provider_requests.cost_kind ELSE excluded.cost_kind END, " ++
                "cost_micros=COALESCE(excluded.cost_micros, provider_requests.cost_micros), " ++
                "cost_currency=excluded.cost_currency, " ++
                "input_tokens=COALESCE(excluded.input_tokens, provider_requests.input_tokens), " ++
                "output_tokens=COALESCE(excluded.output_tokens, provider_requests.output_tokens), " ++
                "cache_read_tokens=COALESCE(excluded.cache_read_tokens, provider_requests.cache_read_tokens), " ++
                "cache_write_tokens=COALESCE(excluded.cache_write_tokens, provider_requests.cache_write_tokens), " ++
                "ttft_ms=COALESCE(excluded.ttft_ms, provider_requests.ttft_ms), " ++
                "settings=COALESCE(excluded.settings, provider_requests.settings), " ++
                "usage=COALESCE(excluded.usage, provider_requests.usage), " ++
                "cost=COALESCE(excluded.cost, provider_requests.cost), " ++
                "metadata=COALESCE(excluded.metadata, provider_requests.metadata);",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, request.id);
        try bindNullableText(stmt, 2, request.conversation orelse default_conversation);
        try bindNullableText(stmt, 3, request.message_id);
        try bindNullableText(stmt, 4, request.provider_id);
        try bindNullableText(stmt, 5, request.parent_request_id);
        try bindInt(stmt, 6, created_ms);
        try bindInt(stmt, 7, now_ms orelse created_ms);
        try bindNullableInt(stmt, 8, request.finished_at_ms);
        try bindText(stmt, 9, request.provider);
        try bindText(stmt, 10, request.model);
        try bindText(stmt, 11, request.status);
        try bindText(stmt, 12, request.cost_kind);
        try bindNullableInt(stmt, 13, request.cost_micros);
        try bindText(stmt, 14, request.cost_currency);
        try bindNullableInt(stmt, 15, request.input_tokens);
        try bindNullableInt(stmt, 16, request.output_tokens);
        try bindNullableInt(stmt, 17, request.cache_read_tokens);
        try bindNullableInt(stmt, 18, request.cache_write_tokens);
        try bindNullableInt(stmt, 19, request.ttft_ms);
        try bindNullableText(stmt, 20, settings);
        try bindNullableText(stmt, 21, usage);
        try bindNullableText(stmt, 22, cost);
        try bindNullableText(stmt, 23, metadata);
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
    }

    fn upsertConversation(self: *Store, conversation: []const u8, at_ms: i64) !void {
        const stmt = try self.prepare(
            "INSERT INTO conversations(id, created_at, updated_at) VALUES(?1, ?2, ?2) " ++
                "ON CONFLICT(id) DO UPDATE SET updated_at=excluded.updated_at;",
        );
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        try bindInt(stmt, 2, at_ms);
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
    }

    /// Shallow-merge one JSON object into the conversation header. Later keys
    /// win; JSON null is stored as a value rather than deleting the key.
    fn mergeMetadata(self: *Store, conversation: []const u8, patch: std.json.Value) !void {
        const a = self.allocator;
        const header = try self.loadHeader(a, conversation);
        defer header.deinit(a);
        var parsed = std.json.parseFromSlice(std.json.Value, a, header.metadata, .{ .allocate = .alloc_always }) catch return error.InvalidMetadata;
        defer parsed.deinit();
        if (parsed.value != .object) return error.InvalidMetadata;
        const target = &parsed.value.object;
        var iterator = patch.object.iterator();
        while (iterator.next()) |entry| {
            const key = try parsed.arena.allocator().dupe(u8, entry.key_ptr.*);
            try target.put(parsed.arena.allocator(), key, try cloneJson(parsed.arena.allocator(), entry.value_ptr.*));
        }
        const merged = try std.json.Stringify.valueAlloc(a, parsed.value, .{});
        defer a.free(merged);
        if (merged.len > max_document_bytes) return error.MetadataTooLarge;

        const stmt = try self.prepare("UPDATE conversations SET metadata = ?2 WHERE id = ?1;");
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, conversation);
        try bindText(stmt, 2, merged);
        if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
    }

    /// Serialize one finite JSON value in canonical form: object keys sorted by
    /// byte order, no insignificant whitespace. Content addressing depends on
    /// this, because the producing process does not control key order.
    fn encodeDocument(self: *Store, value: std.json.Value) ![]u8 {
        var out: std.ArrayList(u8) = .empty;
        errdefer out.deinit(self.allocator);
        try writeCanonical(&out, self.allocator, value);
        if (out.items.len > max_document_bytes) return error.DocumentTooLarge;
        return out.toOwnedSlice(self.allocator);
    }

    fn encodeOptional(self: *Store, value: ?std.json.Value) !?[]u8 {
        const present = value orelse return null;
        return try self.encodeDocument(present);
    }

    fn rollbackQuietly(self: *Store) void {
        self.exec("ROLLBACK;") catch {};
    }

    fn exec(self: *Store, sql: [:0]const u8) !void {
        if (c.sqlite3_exec(self.db, sql, null, null, null) != c.SQLITE_OK) {
            return errorFromCode(c.sqlite3_errcode(self.db));
        }
    }

    fn prepare(self: *Store, sql: [:0]const u8) !*c.sqlite3_stmt {
        var statement: ?*c.sqlite3_stmt = null;
        if (c.sqlite3_prepare_v2(self.db, sql, -1, &statement, null) != c.SQLITE_OK) {
            return errorFromCode(c.sqlite3_errcode(self.db));
        }
        return statement.?;
    }

    fn readUserVersion(self: *Store) !c_int {
        const stmt = try self.prepare("PRAGMA user_version;");
        defer _ = c.sqlite3_finalize(stmt);
        if (c.sqlite3_step(stmt) != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
        return c.sqlite3_column_int(stmt, 0);
    }

    /// `PRAGMA journal_mode=WAL` takes an exclusive lock and does not invoke
    /// the busy handler, so concurrent creators retry it explicitly.
    fn enableWal(self: *Store) !void {
        var attempt: usize = 0;
        while (attempt < max_write_attempts) : (attempt += 1) {
            if (self.exec("PRAGMA journal_mode=WAL;")) |_| {
                return;
            } else |err| {
                if (err != error.DatabaseBusy) return err;
                std.Io.sleep(self.io, .fromMilliseconds(retry_backoff_ms), .awake) catch {};
            }
        }
        return error.DatabaseBusy;
    }

    fn tableExists(self: *Store, name: []const u8) !bool {
        const stmt = try self.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1;");
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, name);
        const code = c.sqlite3_step(stmt);
        if (code == c.SQLITE_ROW) return true;
        if (code == c.SQLITE_DONE) return false;
        return errorFromCode(c.sqlite3_errcode(self.db));
    }

    /// Convert pre-version-3 rows into content-addressed messages. The digest
    /// cannot be computed in SQLite, so the copy runs here, inside the caller's
    /// transaction.
    fn convertLegacyEntries(self: *Store) !void {
        if (!try self.tableExists("conversation_entries")) return;
        const has_request = try self.columnExists("conversation_entries", "request_id");

        var rows: std.ArrayList(LegacyRow) = .empty;
        defer {
            for (rows.items) |row| row.deinit(self.allocator);
            rows.deinit(self.allocator);
        }
        {
            const sql: [:0]const u8 = if (has_request)
                "SELECT conversation_id, seq, at_ms, kind, payload, request_id FROM conversation_entries ORDER BY conversation_id, seq;"
            else
                "SELECT conversation_id, seq, at_ms, kind, payload, NULL FROM conversation_entries ORDER BY conversation_id, seq;";
            const stmt = try self.prepare(sql);
            defer _ = c.sqlite3_finalize(stmt);
            while (true) {
                const code = c.sqlite3_step(stmt);
                if (code == c.SQLITE_DONE) break;
                if (code != c.SQLITE_ROW) return errorFromCode(c.sqlite3_errcode(self.db));
                try rows.append(self.allocator, .{
                    .conversation = try columnRequiredText(self.allocator, stmt, 0),
                    .seq = c.sqlite3_column_int64(stmt, 1),
                    .at_ms = c.sqlite3_column_int64(stmt, 2),
                    .kind = try columnRequiredText(self.allocator, stmt, 3),
                    .payload = try columnRequiredText(self.allocator, stmt, 4),
                    .request_id = try columnText(self.allocator, stmt, 5),
                });
            }
        }

        for (rows.items) |row| {
            // Canonicalize where possible so a legacy payload hashes to the
            // same id a fresh write would produce.
            const canonical = try canonicalizeText(self.allocator, row.payload);
            defer self.allocator.free(canonical);
            const id = try messageId(self.allocator, row.kind, 1, canonical);
            defer self.allocator.free(id);
            const message: Message = .{ .id = id, .kind = row.kind, .payload_version = 1, .payload = canonical, .at_ms = row.at_ms };
            try self.insertMessage(message);
            const stmt = try self.prepare(
                "INSERT OR REPLACE INTO branch_messages(conversation_id, seq, message_id, at_ms, request_id) VALUES(?1, ?2, ?3, ?4, ?5);",
            );
            defer _ = c.sqlite3_finalize(stmt);
            try bindText(stmt, 1, row.conversation);
            try bindInt(stmt, 2, row.seq);
            try bindText(stmt, 3, id);
            try bindInt(stmt, 4, row.at_ms);
            try bindNullableText(stmt, 5, row.request_id);
            if (c.sqlite3_step(stmt) != c.SQLITE_DONE) return errorFromCode(c.sqlite3_errcode(self.db));
        }

        try self.exec("DROP TABLE conversation_entries;");
    }

    fn columnExists(self: *Store, table: []const u8, column: []const u8) !bool {
        var buffer: [256]u8 = undefined;
        const sql = try std.fmt.bufPrintZ(&buffer, "SELECT 1 FROM pragma_table_info('{s}') WHERE name = ?1;", .{table});
        const stmt = try self.prepare(sql);
        defer _ = c.sqlite3_finalize(stmt);
        try bindText(stmt, 1, column);
        const code = c.sqlite3_step(stmt);
        if (code == c.SQLITE_ROW) return true;
        if (code == c.SQLITE_DONE) return false;
        return errorFromCode(c.sqlite3_errcode(self.db));
    }

    /// A version match is not enough: a database written by a different binary
    /// at the same version must fail loudly instead of silently missing
    /// columns. Probing costs one prepare per schema object.
    fn probe(self: *Store, sql: [:0]const u8) !void {
        var statement: ?*c.sqlite3_stmt = null;
        const code = c.sqlite3_prepare_v2(self.db, sql, -1, &statement, null);
        if (statement) |stmt| _ = c.sqlite3_finalize(stmt);
        if (code != c.SQLITE_OK) return error.UnsupportedSchemaVersion;
    }

    fn verifyShape(self: *Store) !void {
        try self.probe("SELECT metadata, forked_from_id, forked_from_seq FROM conversations LIMIT 1;");
        try self.probe("SELECT id, kind, payload_version, payload FROM messages LIMIT 1;");
        try self.probe("SELECT conversation_id, seq, message_id, at_ms, request_id FROM branch_messages LIMIT 1;");
        try self.probe("SELECT cost_kind, cost_micros, cost_currency, parent_request_id, message_id FROM provider_requests LIMIT 1;");
        try self.probe("SELECT hash, mime, byte_count, bytes FROM blobs LIMIT 1;");
    }

    fn migrate(self: *Store) !void {
        const version = try self.readUserVersion();
        if (version > schema_version) return error.UnsupportedSchemaVersion;
        if (version == schema_version) return self.verifyShape();

        // WAL is persistent database state. Establish it once before the
        // creating transaction; the pragma does not honour `busy_timeout`, so
        // concurrent first-open races retry explicitly. Losing processes then
        // find the version already set and skip the idempotent DDL.
        try self.enableWal();
        try self.exec("BEGIN IMMEDIATE;");
        errdefer self.rollbackQuietly();
        const current = try self.readUserVersion();
        if (current != 0) {
            // Tables first, then missing columns, then indexes: an index can
            // only reference a column that already exists. Every step is
            // idempotent, so a database left mid-upgrade by an older binary
            // resumes cleanly instead of failing on a duplicate column.
            try self.exec(tables);
            try self.ensureColumn("conversations", "metadata", "TEXT NOT NULL DEFAULT '{}'");
            try self.ensureColumn("conversations", "forked_from_id", "TEXT");
            try self.ensureColumn("conversations", "forked_from_seq", "INTEGER");
            if (try self.tableExists("conversation_entries")) {
                try self.ensureColumn("conversation_entries", "request_id", "TEXT");
            }
            for (request_columns) |column| {
                try self.ensureColumn("provider_requests", column.name, column.definition);
            }
            try self.exec(indexes);
            try self.convertLegacyEntries();
        } else {
            try self.exec(tables);
            try self.exec(indexes);
        }
        var buffer: [32]u8 = undefined;
        const set_version = try std.fmt.bufPrintZ(&buffer, "PRAGMA user_version={d};", .{schema_version});
        try self.exec(set_version);
        try self.exec("COMMIT;");
        try self.verifyShape();
    }

    /// Add a column only when it is absent. Version 1 predates `metadata` and
    /// `provider_requests` entirely, version 2 has no typed cost columns, and
    /// this version creates them all up front, so the same step has to be a
    /// no-op in all three cases.
    fn ensureColumn(self: *Store, table: []const u8, column: []const u8, definition: []const u8) !void {
        if (try self.columnExists(table, column)) return;
        var buffer: [256]u8 = undefined;
        const sql = try std.fmt.bufPrintZ(&buffer, "ALTER TABLE {s} ADD COLUMN {s} {s};", .{ table, column, definition });
        try self.exec(sql);
    }
};

const LegacyRow = struct {
    conversation: []u8,
    seq: i64,
    at_ms: i64,
    kind: []u8,
    payload: []u8,
    request_id: ?[]u8,

    fn deinit(self: LegacyRow, allocator: std.mem.Allocator) void {
        allocator.free(self.conversation);
        allocator.free(self.kind);
        allocator.free(self.payload);
        if (self.request_id) |value| allocator.free(value);
    }
};

pub fn freeRecords(allocator: std.mem.Allocator, records: []Record) void {
    for (records) |record| {
        allocator.free(record.message_id);
        allocator.free(record.kind);
        allocator.free(record.payload);
        if (record.request_id) |id| allocator.free(id);
    }
    allocator.free(records);
}

pub fn freeRequestRecords(allocator: std.mem.Allocator, records: []RequestRecord) void {
    for (records) |record| {
        allocator.free(record.id);
        allocator.free(record.provider);
        allocator.free(record.model);
        allocator.free(record.status);
        allocator.free(record.cost_kind);
        allocator.free(record.cost_currency);
        for ([_]?[]const u8{ record.conversation, record.message_id, record.provider_id, record.parent_request_id, record.settings, record.usage, record.cost, record.metadata }) |field| {
            if (field) |text| allocator.free(text);
        }
    }
    allocator.free(records);
}

pub fn freeSummaries(allocator: std.mem.Allocator, summaries: []Summary) void {
    for (summaries) |summary| {
        allocator.free(summary.id);
        allocator.free(summary.metadata);
        if (summary.forked_from_id) |value| allocator.free(value);
    }
    allocator.free(summaries);
}

pub fn freeBlob(allocator: std.mem.Allocator, blob: Blob) void {
    allocator.free(blob.hash);
    allocator.free(blob.mime);
    allocator.free(blob.bytes);
}

pub fn freeSnapshot(allocator: std.mem.Allocator, snapshot: Snapshot) void {
    allocator.free(snapshot.metadata);
    if (snapshot.forked_from_id) |value| allocator.free(value);
    freeRecords(allocator, snapshot.entries);
    freeRequestRecords(allocator, snapshot.requests);
}

pub fn validateConversationId(id: []const u8) !void {
    if (id.len == 0 or id.len > max_conversation_id_bytes) return error.InvalidConversationId;
    for (id) |byte| if (byte < 0x21 or byte > 0x7e) return error.InvalidConversationId;
}

pub fn validateKind(kind: []const u8) !void {
    if (kind.len == 0 or kind.len > max_kind_bytes) return error.InvalidEntryKind;
    for (kind) |byte| if (byte < 0x21 or byte > 0x7e) return error.InvalidEntryKind;
}

pub fn validatePayloadVersion(version: u32) !void {
    if (version == 0 or version > max_payload_version) return error.UnsupportedPayloadVersion;
}

pub fn validateRequestId(id: []const u8) !void {
    if (id.len == 0 or id.len > max_request_id_bytes) return error.InvalidRequestId;
    for (id) |byte| if (byte < 0x21 or byte > 0x7e) return error.InvalidRequestId;
}

pub fn validateLabel(label: []const u8) !void {
    if (label.len == 0 or label.len > max_label_bytes) return error.InvalidRequestLabel;
    for (label) |byte| if (byte < 0x21 or byte > 0x7e) return error.InvalidRequestLabel;
}

pub fn validateCostKind(kind: []const u8) !void {
    for (cost_kinds) |candidate| if (std.mem.eql(u8, candidate, kind)) return;
    return error.InvalidCostKind;
}

pub fn validateRequest(request: Request) !void {
    try validateRequestId(request.id);
    if (request.conversation) |conversation| try validateConversationId(conversation);
    if (request.message_id) |message_id| try validateLabel(message_id);
    if (request.provider_id) |provider_id| try validateLabel(provider_id);
    if (request.parent_request_id) |parent| try validateRequestId(parent);
    if (std.mem.eql(u8, request.id, request.parent_request_id orelse "")) return error.InvalidRequestId;
    try validateLabel(request.provider);
    try validateLabel(request.model);
    try validateLabel(request.status);
    try validateLabel(request.cost_currency);
    try validateCostKind(request.cost_kind);
    for ([_]?i64{ request.input_tokens, request.output_tokens, request.cache_read_tokens, request.cache_write_tokens, request.ttft_ms }) |count| {
        if (count) |value| if (value < 0) return error.InvalidRequestCount;
    }
    if (request.cost_micros) |value| if (value < 0) return error.InvalidRequestCount;
    if (!std.mem.eql(u8, request.cost_kind, "pending") and !std.mem.eql(u8, request.cost_kind, "unknown") and request.cost_micros == null) {
        return error.InvalidRequestCost;
    }
    for ([_]?std.json.Value{ request.settings, request.usage, request.cost, request.metadata }) |field| {
        if (field) |value| try validateDocument(value);
    }
}

pub fn validateDocument(value: std.json.Value) !void {
    try validateValue(value, 0);
}

fn validateValue(value: std.json.Value, depth: usize) !void {
    if (depth > max_document_depth) return error.DocumentTooDeep;
    switch (value) {
        .array => |array| for (array.items) |item| try validateValue(item, depth + 1),
        .object => |object| {
            var iterator = object.iterator();
            while (iterator.next()) |entry| {
                if (entry.key_ptr.*.len > max_document_bytes) return error.DocumentTooLarge;
                try validateValue(entry.value_ptr.*, depth + 1);
            }
        },
        .string, .number_string => |string| if (string.len > max_document_bytes) return error.DocumentTooLarge,
        .float => |number| if (!std.math.isFinite(number)) return error.InvalidDocument,
        else => {},
    }
}

/// The address of one message: BLAKE3 over the kind, the payload version and
/// the canonical payload. Length-prefixing the parts keeps the digest
/// unambiguous when a kind or payload contains the separator byte.
pub fn messageId(allocator: std.mem.Allocator, kind: []const u8, payload_version: u32, payload: []const u8) ![]u8 {
    var hasher = std.crypto.hash.Blake3.init(.{});
    var length: [8]u8 = undefined;
    std.mem.writeInt(u64, &length, kind.len, .little);
    hasher.update(&length);
    hasher.update(kind);
    std.mem.writeInt(u32, length[0..4], payload_version, .little);
    hasher.update(length[0..4]);
    std.mem.writeInt(u64, &length, payload.len, .little);
    hasher.update(&length);
    hasher.update(payload);
    var digest: [32]u8 = undefined;
    hasher.final(&digest);
    const hex = std.fmt.bytesToHex(digest, .lower);
    return allocator.dupe(u8, &hex);
}

pub fn blobHash(allocator: std.mem.Allocator, bytes: []const u8) ![]u8 {
    var digest: [32]u8 = undefined;
    std.crypto.hash.Blake3.hash(bytes, &digest, .{});
    const hex = std.fmt.bytesToHex(digest, .lower);
    return allocator.dupe(u8, &hex);
}

/// Parse and re-serialize a payload in canonical form. Unparseable legacy text
/// is kept verbatim so an upgrade never loses a transcript.
fn canonicalizeText(allocator: std.mem.Allocator, text: []const u8) ![]u8 {
    var parsed = std.json.parseFromSlice(std.json.Value, allocator, text, .{ .allocate = .alloc_always }) catch {
        return allocator.dupe(u8, text);
    };
    defer parsed.deinit();
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    try writeCanonical(&out, allocator, parsed.value);
    return out.toOwnedSlice(allocator);
}

fn writeJsonString(out: *std.ArrayList(u8), allocator: std.mem.Allocator, value: []const u8) !void {
    try out.append(allocator, '"');
    for (value) |byte| switch (byte) {
        '"' => try out.appendSlice(allocator, "\\\""),
        '\\' => try out.appendSlice(allocator, "\\\\"),
        0x08 => try out.appendSlice(allocator, "\\b"),
        0x0c => try out.appendSlice(allocator, "\\f"),
        '\n' => try out.appendSlice(allocator, "\\n"),
        '\r' => try out.appendSlice(allocator, "\\r"),
        '\t' => try out.appendSlice(allocator, "\\t"),
        else => if (byte < 0x20) {
            try out.print(allocator, "\\u{x:0>4}", .{byte});
        } else try out.append(allocator, byte),
    };
    try out.append(allocator, '"');
}

/// Serialize a finite JSON value with object keys in byte order. Two values
/// that differ only in key order produce identical text, which is what makes
/// the content address stable.
fn writeCanonical(out: *std.ArrayList(u8), allocator: std.mem.Allocator, value: std.json.Value) !void {
    switch (value) {
        .null => try out.appendSlice(allocator, "null"),
        .bool => |flag| try out.appendSlice(allocator, if (flag) "true" else "false"),
        .integer => |number| try out.print(allocator, "{d}", .{number}),
        .float => |number| try out.print(allocator, "{d}", .{number}),
        .number_string => |text| try out.appendSlice(allocator, text),
        .string => |string| try writeJsonString(out, allocator, string),
        .array => |array| {
            try out.append(allocator, '[');
            for (array.items, 0..) |item, index| {
                if (index != 0) try out.append(allocator, ',');
                try writeCanonical(out, allocator, item);
            }
            try out.append(allocator, ']');
        },
        .object => |object| {
            const Row = struct { key: []const u8, value: std.json.Value };
            const rows = try allocator.alloc(Row, object.count());
            defer allocator.free(rows);
            var iterator = object.iterator();
            var count: usize = 0;
            while (iterator.next()) |entry| : (count += 1) {
                rows[count] = .{ .key = entry.key_ptr.*, .value = entry.value_ptr.* };
            }
            std.mem.sort(Row, rows, {}, struct {
                fn lessThan(_: void, left: Row, right: Row) bool {
                    return std.mem.order(u8, left.key, right.key) == .lt;
                }
            }.lessThan);
            try out.append(allocator, '{');
            for (rows, 0..) |row, index| {
                if (index != 0) try out.append(allocator, ',');
                try writeJsonString(out, allocator, row.key);
                try out.append(allocator, ':');
                try writeCanonical(out, allocator, row.value);
            }
            try out.append(allocator, '}');
        },
    }
}

pub fn databasePath(allocator: std.mem.Allocator, environ: *const std.process.Environ.Map) ![:0]u8 {
    if (environ.get("MISA_CONVERSATION_DB")) |path| {
        if (path.len == 0 or std.mem.indexOfScalar(u8, path, 0) != null) return error.InvalidDatabasePath;
        return allocator.dupeZ(u8, path);
    }
    const root = if (environ.get("XDG_STATE_HOME")) |value|
        try std.fs.path.join(allocator, &.{ value, "misa" })
    else if (environ.get("HOME")) |home|
        try std.fs.path.join(allocator, &.{ home, ".local", "state", "misa" })
    else
        return error.DatabasePathUnavailable;
    defer allocator.free(root);
    return std.fs.path.joinZ(allocator, &.{ root, "conversations.sqlite3" });
}

fn prepareDirectory(io: std.Io, path: []const u8, protect: bool) !void {
    const directory_path = std.fs.path.dirname(path) orelse ".";
    try std.Io.Dir.cwd().createDirPath(io, directory_path);
    // `.iterate` avoids the `O_PATH` descriptor, which cannot carry a mode.
    var directory = try std.Io.Dir.cwd().openDir(io, directory_path, .{ .iterate = true, .follow_symlinks = false });
    defer directory.close(io);
    if (protect) try directory.setPermissions(io, @enumFromInt(0o700));
}

fn bindText(stmt: *c.sqlite3_stmt, index: c_int, value: []const u8) !void {
    if (c.sqlite3_bind_text(stmt, index, value.ptr, @intCast(value.len), transient) != c.SQLITE_OK) {
        return error.DatabaseError;
    }
}

fn bindNullableText(stmt: *c.sqlite3_stmt, index: c_int, value: ?[]const u8) !void {
    const present = value orelse {
        if (c.sqlite3_bind_null(stmt, index) != c.SQLITE_OK) return error.DatabaseError;
        return;
    };
    try bindText(stmt, index, present);
}

fn bindInt(stmt: *c.sqlite3_stmt, index: c_int, value: i64) !void {
    if (c.sqlite3_bind_int64(stmt, index, value) != c.SQLITE_OK) return error.DatabaseError;
}

fn bindNullableInt(stmt: *c.sqlite3_stmt, index: c_int, value: ?i64) !void {
    const present = value orelse {
        if (c.sqlite3_bind_null(stmt, index) != c.SQLITE_OK) return error.DatabaseError;
        return;
    };
    try bindInt(stmt, index, present);
}

fn columnText(allocator: std.mem.Allocator, stmt: *c.sqlite3_stmt, index: c_int) !?[]u8 {
    const text = c.sqlite3_column_text(stmt, index) orelse return null;
    const copy: []u8 = try allocator.dupe(u8, text[0..@intCast(c.sqlite3_column_bytes(stmt, index))]);
    return copy;
}

fn columnRequiredText(allocator: std.mem.Allocator, stmt: *c.sqlite3_stmt, index: c_int) ![]u8 {
    return (try columnText(allocator, stmt, index)) orelse error.DatabaseError;
}

fn columnOptionalInt(stmt: *c.sqlite3_stmt, index: c_int) ?i64 {
    if (c.sqlite3_column_type(stmt, index) == c.SQLITE_NULL) return null;
    return c.sqlite3_column_int64(stmt, index);
}

fn cloneJson(allocator: std.mem.Allocator, value: std.json.Value) !std.json.Value {
    const encoded = try std.json.Stringify.valueAlloc(allocator, value, .{});
    defer allocator.free(encoded);
    const parsed = try std.json.parseFromSlice(std.json.Value, allocator, encoded, .{ .allocate = .alloc_always });
    return parsed.value;
}

fn errorFromCode(code: c_int) anyerror {
    return switch (code) {
        c.SQLITE_BUSY, c.SQLITE_LOCKED => error.DatabaseBusy,
        c.SQLITE_FULL => error.DatabaseFull,
        c.SQLITE_CONSTRAINT => error.DatabaseConstraint,
        else => error.DatabaseError,
    };
}

fn testEnviron(allocator: std.mem.Allocator, sub_path: []const u8) !struct {
    map: std.process.Environ.Map,
    path: []u8,
} {
    const path = try std.fmt.allocPrint(allocator, ".zig-cache/tmp/{s}/conversations.sqlite3", .{sub_path});
    errdefer allocator.free(path);
    var map = std.process.Environ.Map.init(allocator);
    errdefer map.deinit();
    try map.put("MISA_CONVERSATION_DB", path);
    return .{ .map = map, .path = path };
}

/// Parse a small JSON document for a test. The caller owns the arena backing
/// the result, so nothing leaks between tests.
fn jsonValue(allocator: std.mem.Allocator, text: []const u8) !std.json.Value {
    return std.json.parseFromSliceLeaky(std.json.Value, allocator, text, .{ .allocate = .alloc_always });
}

test "database paths follow override and XDG precedence" {
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("HOME", "/home/test");
    var path = try databasePath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/home/test/.local/state/misa/conversations.sqlite3", path);
    std.testing.allocator.free(path);
    try environ.put("XDG_STATE_HOME", "/state");
    path = try databasePath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/state/misa/conversations.sqlite3", path);
    std.testing.allocator.free(path);
    try environ.put("MISA_CONVERSATION_DB", "/tmp/log.sqlite3");
    path = try databasePath(std.testing.allocator, &environ);
    try std.testing.expectEqualStrings("/tmp/log.sqlite3", path);
    std.testing.allocator.free(path);
}

test "the default database directory is private to the user" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    const root = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/state", .{temporary.sub_path});
    defer std.testing.allocator.free(root);
    try environ.put("XDG_STATE_HOME", root);

    var store = try Store.open(std.testing.allocator, std.testing.io, &environ);
    defer store.deinit();
    const directory = try std.fs.path.join(std.testing.allocator, &.{ root, "misa" });
    defer std.testing.allocator.free(directory);
    const stat = try std.Io.Dir.cwd().statFile(std.testing.io, directory, .{});
    try std.testing.expectEqual(@as(u32, 0o700), stat.permissions.toMode() & 0o777);
}

/// Read one integer column. Tests use this to inspect rows the public API does
/// not expose, such as the message dedupe table.
fn scalar(store: *Store, sql: [:0]const u8) !i64 {
    const stmt = try store.prepare(sql);
    defer _ = c.sqlite3_finalize(stmt);
    if (c.sqlite3_step(stmt) != c.SQLITE_ROW) return error.DatabaseError;
    return c.sqlite3_column_int64(stmt, 0);
}

test "append assigns contiguous sequences and load reads them back" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }

    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();
    const entries = [_]Entry{
        .{ .kind = "message", .data = .{ .object = .{} } },
        .{ .kind = "tool_call", .data = .{ .string = "ls" } },
    };
    try std.testing.expectEqual(@as(i64, 2), try store.append(.{ .conversation = "session-a", .entries = &entries, .at_ms = 1000 }));
    try std.testing.expectEqual(@as(i64, 3), try store.append(.{ .conversation = "session-a", .entries = entries[0..1], .at_ms = 2000 }));
    try std.testing.expectEqual(@as(i64, 1), try store.append(.{ .conversation = "session-b", .entries = entries[0..1], .at_ms = 3000 }));

    const snapshot = try store.load(std.testing.allocator, "session-a", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, snapshot);
    try std.testing.expectEqual(@as(usize, 3), snapshot.entries.len);
    try std.testing.expectEqual(@as(i64, 1), snapshot.entries[0].seq);
    try std.testing.expectEqual(@as(i64, 1000), snapshot.entries[0].at_ms);
    try std.testing.expectEqual(@as(u32, 1), snapshot.entries[0].payload_version);
    try std.testing.expectEqualStrings("tool_call", snapshot.entries[1].kind);
    try std.testing.expectEqualStrings("\"ls\"", snapshot.entries[1].payload);
    try std.testing.expectEqual(@as(i64, 3), snapshot.entries[2].seq);
    try std.testing.expectEqualStrings("{}", snapshot.metadata);
    try std.testing.expect(snapshot.forked_from_id == null);
    try std.testing.expect(!snapshot.more_entries);

    const tail = try store.load(std.testing.allocator, "session-a", 2, 16, 16);
    defer freeSnapshot(std.testing.allocator, tail);
    try std.testing.expectEqual(@as(usize, 1), tail.entries.len);
    try std.testing.expectEqual(@as(i64, 3), tail.entries[0].seq);

    try std.testing.expectError(error.ConversationNotFound, store.load(std.testing.allocator, "missing", 0, 16, 16));
}

test "load reports a truncated page for entries and requests" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();

    const entries = [_]Entry{.{ .kind = "message", .data = .{ .integer = 1 } }};
    for (0..3) |index| try std.testing.expectEqual(@as(i64, @intCast(index + 1)), try store.append(.{ .conversation = "paged", .entries = &entries, .at_ms = 1 }));
    for (0..3) |index| {
        var id_buffer: [16]u8 = undefined;
        const request_id = try std.fmt.bufPrint(&id_buffer, "r{d}", .{index});
        try store.recordRequest(.{ .id = request_id, .conversation = "paged", .provider = "p", .model = "m", .status = "ok" }, 1);
    }

    const first = try store.load(std.testing.allocator, "paged", 0, 2, 2);
    defer freeSnapshot(std.testing.allocator, first);
    try std.testing.expectEqual(@as(usize, 2), first.entries.len);
    try std.testing.expect(first.more_entries);
    try std.testing.expect(first.more_requests);

    const last = try store.load(std.testing.allocator, "paged", first.entries[1].seq, 2, 2);
    defer freeSnapshot(std.testing.allocator, last);
    try std.testing.expectEqual(@as(usize, 1), last.entries.len);
    try std.testing.expect(!last.more_entries);
}

test "metadata merges into the conversation header" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const entries = [_]Entry{.{ .kind = "message", .data = .{ .object = .{} } }};

    _ = try store.append(.{ .conversation = "meta", .entries = &entries, .at_ms = 1, .metadata = try jsonValue(arena.allocator(), "{\"title\":\"first\",\"cwd\":\"/work\"}") });
    _ = try store.append(.{ .conversation = "meta", .entries = &entries, .at_ms = 2, .metadata = try jsonValue(arena.allocator(), "{\"title\":\"second\"}") });

    const snapshot = try store.load(std.testing.allocator, "meta", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, snapshot);
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, snapshot.metadata, .{});
    defer parsed.deinit();
    try std.testing.expectEqualStrings("second", parsed.value.object.get("title").?.string);
    try std.testing.expectEqualStrings("/work", parsed.value.object.get("cwd").?.string);
}

test "provider requests keep settled cost and record parentage" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();

    // A request with no conversation and no message is still durable.
    try store.recordRequest(.{ .id = "orphan", .provider = "openai", .model = "gpt", .status = "ok" }, 10);

    // Settle a cost, then enrich with usage only: the reported figure and its
    // kind survive, because a later pending write must not erase them.
    try store.recordRequest(.{ .id = "req-1", .conversation = "chat", .provider = "anthropic", .model = "opus", .status = "pending" }, 20);
    try store.recordRequest(.{ .id = "req-1", .conversation = "chat", .provider = "anthropic", .model = "opus", .status = "ok", .cost_kind = "reported", .cost_micros = 10_000, .input_tokens = 10, .provider_id = "msg_1" }, 30);
    try store.recordRequest(.{ .id = "req-1", .provider = "anthropic", .model = "opus", .status = "ok", .output_tokens = 20 }, 40);

    // A child request (subagent or retry) groups under its parent.
    try store.recordRequest(.{ .id = "req-child", .conversation = "chat", .parent_request_id = "req-1", .provider = "anthropic", .model = "haiku", .status = "ok", .cost_kind = "estimated", .cost_micros = 2500 }, 50);

    const snapshot = try store.load(std.testing.allocator, "chat", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, snapshot);
    try std.testing.expectEqual(@as(usize, 2), snapshot.requests.len);
    const parent = snapshot.requests[0];
    try std.testing.expectEqualStrings("req-1", parent.id);
    try std.testing.expectEqualStrings("reported", parent.cost_kind);
    try std.testing.expectEqual(@as(?i64, 10_000), parent.cost_micros);
    try std.testing.expectEqual(@as(?i64, 10), parent.input_tokens);
    try std.testing.expectEqual(@as(?i64, 20), parent.output_tokens);
    try std.testing.expectEqualStrings("msg_1", parent.provider_id.?);
    try std.testing.expectEqual(@as(i64, 20), parent.created_at_ms);
    try std.testing.expectEqual(@as(i64, 40), parent.updated_at_ms);
    const child = snapshot.requests[1];
    try std.testing.expectEqualStrings("req-1", child.parent_request_id.?);
    try std.testing.expectEqualStrings("estimated", child.cost_kind);
    try std.testing.expectEqual(@as(?i64, 2500), child.cost_micros);
}

test "identical entries share one message row" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();

    // The same content in two conversations, written with the object keys in a
    // different order, must address one message.
    const first = [_]Entry{.{ .kind = "user", .data = .{ .object = .{} } }};
    _ = try store.append(.{ .conversation = "one", .entries = &first, .at_ms = 1 });
    const second = [_]Entry{.{ .kind = "user", .data = .{ .object = .{} } }};
    _ = try store.append(.{ .conversation = "two", .entries = &second, .at_ms = 2 });
    try std.testing.expectEqual(@as(i64, 1), try scalar(&store, "SELECT COUNT(*) FROM messages;"));
    try std.testing.expectEqual(@as(i64, 2), try scalar(&store, "SELECT COUNT(*) FROM branch_messages;"));
    // Object key order does not change the address. The arena owns the parsed
    // documents so the testing allocator sees no leak.
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const ordered = [_]Entry{.{ .kind = "user", .data = try jsonValue(arena.allocator(), "{\"a\":1,\"b\":2}") }};
    _ = try store.append(.{ .conversation = "one", .entries = &ordered, .at_ms = 3 });
    const reordered = [_]Entry{.{ .kind = "user", .data = try jsonValue(arena.allocator(), "{\"b\":2,\"a\":1}") }};
    _ = try store.append(.{ .conversation = "two", .entries = &reordered, .at_ms = 4 });
    try std.testing.expectEqual(@as(i64, 2), try scalar(&store, "SELECT COUNT(*) FROM messages;"));
}

test "forking copies edges, shares payloads, and records provenance" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();

    const entries = [_]Entry{
        .{ .kind = "user", .data = .{ .string = "one" } },
        .{ .kind = "assistant", .data = .{ .string = "two" } },
        .{ .kind = "user", .data = .{ .string = "three" } },
    };
    _ = try store.append(.{ .conversation = "origin", .entries = &entries, .at_ms = 10 });

    try std.testing.expectEqual(@as(i64, 2), try store.fork(.{ .conversation = "branch", .from = "origin", .at_seq = 2, .at_ms = 20 }));

    // Three messages are shared by both transcripts; no payload was copied.
    try std.testing.expectEqual(@as(i64, 3), try scalar(&store, "SELECT COUNT(*) FROM messages;"));
    try std.testing.expectEqual(@as(i64, 5), try scalar(&store, "SELECT COUNT(*) FROM branch_messages;"));

    const snapshot = try store.load(std.testing.allocator, "branch", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, snapshot);
    try std.testing.expectEqual(@as(usize, 2), snapshot.entries.len);
    try std.testing.expectEqualStrings("origin", snapshot.forked_from_id.?);
    try std.testing.expectEqual(@as(?i64, 2), snapshot.forked_from_seq);
    try std.testing.expectEqualStrings("\"two\"", snapshot.entries[1].payload);

    // The two branches share the same message addresses.
    const origin = try store.load(std.testing.allocator, "origin", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, origin);
    try std.testing.expectEqualStrings(origin.entries[0].message_id, snapshot.entries[0].message_id);

    // Writing to a fork does not touch its parent, and neither is subordinate:
    // both are ordinary conversations in the index.
    _ = try store.append(.{ .conversation = "branch", .entries = entries[0..1], .at_ms = 30 });
    try std.testing.expectEqual(@as(i64, 3), try scalar(&store, "SELECT COUNT(*) FROM branch_messages WHERE conversation_id = 'origin';"));
    try std.testing.expectEqual(@as(i64, 3), try scalar(&store, "SELECT COUNT(*) FROM branch_messages WHERE conversation_id = 'branch';"));

    try std.testing.expectError(error.ConversationExists, store.fork(.{ .conversation = "branch", .from = "origin", .at_seq = 1, .at_ms = 40 }));
    try std.testing.expectError(error.ConversationNotFound, store.fork(.{ .conversation = "x", .from = "missing", .at_seq = 1, .at_ms = 40 }));
    try std.testing.expectError(error.InvalidFork, store.fork(.{ .conversation = "y", .from = "origin", .at_seq = 99, .at_ms = 40 }));
    try std.testing.expectError(error.InvalidFork, store.fork(.{ .conversation = "origin", .from = "origin", .at_seq = 1, .at_ms = 40 }));
}

test "list reports recency, size, and fork origin" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();

    const entries = [_]Entry{.{ .kind = "user", .data = .{ .string = "hello" } }};
    _ = try store.append(.{ .conversation = "old", .entries = &entries, .at_ms = 10 });
    _ = try store.append(.{ .conversation = "old", .entries = &entries, .at_ms = 10 });
    _ = try store.append(.{ .conversation = "new", .entries = &entries, .at_ms = 50 });
    _ = try store.fork(.{ .conversation = "spawn", .from = "new", .at_seq = 1, .at_ms = 60 });
    try store.recordRequest(.{ .id = "r", .conversation = "old", .provider = "p", .model = "m", .status = "ok" }, 10);

    const summaries = try store.list(std.testing.allocator, 16);
    defer freeSummaries(std.testing.allocator, summaries);
    try std.testing.expectEqual(@as(usize, 3), summaries.len);
    try std.testing.expectEqualStrings("spawn", summaries[0].id);
    try std.testing.expectEqual(@as(i64, 1), summaries[0].entry_count);
    try std.testing.expectEqualStrings("new", summaries[0].forked_from_id.?);
    try std.testing.expectEqual(@as(?i64, 1), summaries[0].forked_from_seq);
    try std.testing.expectEqualStrings("new", summaries[1].id);
    try std.testing.expectEqualStrings("old", summaries[2].id);
    try std.testing.expectEqual(@as(i64, 2), summaries[2].entry_count);
}

test "blobs are stored once and read back by content hash" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    var fixture = try testEnviron(std.testing.allocator, &temporary.sub_path);
    defer {
        fixture.map.deinit();
        std.testing.allocator.free(fixture.path);
    }
    var store = try Store.open(std.testing.allocator, std.testing.io, &fixture.map);
    defer store.deinit();

    const first = try store.putBlob(std.testing.allocator, "image/png", "PNGDATA");
    defer std.testing.allocator.free(first);
    const second = try store.putBlob(std.testing.allocator, "image/png", "PNGDATA");
    defer std.testing.allocator.free(second);
    try std.testing.expectEqualStrings(first, second);
    try std.testing.expectEqual(@as(i64, 1), try scalar(&store, "SELECT COUNT(*) FROM blobs;"));

    const blob = (try store.getBlob(std.testing.allocator, first)).?;
    defer freeBlob(std.testing.allocator, blob);
    try std.testing.expectEqualStrings("image/png", blob.mime);
    try std.testing.expectEqual(@as(i64, 7), blob.byte_count);
    try std.testing.expectEqualStrings("PNGDATA", blob.bytes);
    try std.testing.expect((try store.getBlob(std.testing.allocator, "0000000000000000000000000000000000000000000000000000000000000000")) == null);
}

test "invalid identifiers, kinds, payloads, requests, and empty batches are rejected" {
    try std.testing.expectError(error.InvalidConversationId, validateConversationId(""));
    try std.testing.expectError(error.InvalidConversationId, validateConversationId("has space"));
    try std.testing.expectError(error.InvalidConversationId, validateConversationId("has\u{0}nul"));
    try std.testing.expectError(error.InvalidEntryKind, validateKind(""));
    try std.testing.expectError(error.InvalidEntryKind, validateKind("bad\nkind"));
    try std.testing.expectError(error.UnsupportedPayloadVersion, validatePayloadVersion(0));
    try std.testing.expectError(error.UnsupportedPayloadVersion, validatePayloadVersion(max_payload_version + 1));
    try std.testing.expectError(error.InvalidRequestId, validateRequestId(""));
    try std.testing.expectError(error.InvalidRequestLabel, validateLabel("bad\tlabel"));
    try std.testing.expectError(error.InvalidCostKind, validateCostKind("guessed"));
    try std.testing.expectError(error.InvalidRequestLabel, validateRequest(.{ .id = "req", .provider = "", .model = "m", .status = "ok" }));
    // A settled kind without a figure cannot be aggregated honestly.
    try std.testing.expectError(error.InvalidRequestCost, validateRequest(.{ .id = "req", .provider = "p", .model = "m", .status = "ok", .cost_kind = "reported" }));
    try std.testing.expectError(error.InvalidRequestCount, validateRequest(.{ .id = "req", .provider = "p", .model = "m", .status = "ok", .cost_kind = "reported", .cost_micros = 1, .input_tokens = -1 }));
    try std.testing.expectError(error.InvalidRequestId, validateRequest(.{ .id = "req", .parent_request_id = "req", .provider = "p", .model = "m", .status = "ok" }));
    // Nesting beyond the depth bound is rejected rather than recursed into.
    const depth = max_document_depth + 2;
    var buffer: [2 * (max_document_depth + 2)]u8 = undefined;
    var index: usize = 0;
    for (0..depth) |_| {
        buffer[index] = '[';
        index += 1;
    }
    for (0..depth) |_| {
        buffer[index] = ']';
        index += 1;
    }
    var parsed = try std.json.parseFromSlice(std.json.Value, std.testing.allocator, buffer[0..index], .{ .allocate = .alloc_always });
    defer parsed.deinit();
    try std.testing.expectError(error.DocumentTooDeep, validateDocument(parsed.value));
}

test "legacy databases upgrade to content-addressed transcripts" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrintSentinel(std.testing.allocator, ".zig-cache/tmp/{s}/conversations.sqlite3", .{temporary.sub_path}, 0);
    defer std.testing.allocator.free(path);

    // Version 1: no metadata column, no provider_requests, untyped entries.
    var handle: ?*c.sqlite3 = null;
    try std.testing.expectEqual(c.SQLITE_OK, c.sqlite3_open_v2(path.ptr, &handle, c.SQLITE_OPEN_READWRITE | c.SQLITE_OPEN_CREATE, null));
    const db = handle.?;
    const legacy = "CREATE TABLE conversations (id TEXT PRIMARY KEY, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL) WITHOUT ROWID;" ++
        "CREATE TABLE conversation_entries (conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE, " ++
        "seq INTEGER NOT NULL, at_ms INTEGER NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL, " ++
        "PRIMARY KEY (conversation_id, seq)) WITHOUT ROWID;" ++
        "INSERT INTO conversations(id, created_at, updated_at) VALUES('old', 1, 1);" ++
        "INSERT INTO conversation_entries(conversation_id, seq, at_ms, kind, payload) VALUES('old', 1, 1, 'message', '{\"kept\":true}');" ++
        "INSERT INTO conversation_entries(conversation_id, seq, at_ms, kind, payload) VALUES('old', 2, 2, 'message', '{\"a\":1,\"b\":2}');" ++
        "PRAGMA user_version=1;";
    try std.testing.expectEqual(c.SQLITE_OK, c.sqlite3_exec(db, legacy, null, null, null));
    _ = c.sqlite3_close_v2(db);

    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_CONVERSATION_DB", path);
    var store = try Store.open(std.testing.allocator, std.testing.io, &environ);
    defer store.deinit();

    const snapshot = try store.load(std.testing.allocator, "old", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, snapshot);
    try std.testing.expectEqual(@as(usize, 2), snapshot.entries.len);
    try std.testing.expectEqualStrings("{\"kept\":true}", snapshot.entries[0].payload);
    // The conversion canonicalises so a legacy payload hashes like a fresh one.
    try std.testing.expectEqualStrings("{\"a\":1,\"b\":2}", snapshot.entries[1].payload);
    try std.testing.expectEqualStrings("{}", snapshot.metadata);
    try std.testing.expect(snapshot.entries[0].request_id == null);
    try std.testing.expect(!try store.tableExists("conversation_entries"));

    // The upgraded database accepts every new write path, and the legacy entry
    // now dedupes against an identical fresh one.
    try store.recordRequest(.{ .id = "after-upgrade", .conversation = "old", .provider = "p", .model = "m", .status = "ok" }, 5);
    var arena = std.heap.ArenaAllocator.init(std.testing.allocator);
    defer arena.deinit();
    const entries = [_]Entry{.{ .kind = "message", .data = try jsonValue(arena.allocator(), "{\"a\":1,\"b\":2}") }};
    _ = try store.append(.{ .conversation = "old", .entries = &entries, .at_ms = 6, .metadata = try jsonValue(arena.allocator(), "{\"upgraded\":true}") });
    const reopened = try store.load(std.testing.allocator, "old", 0, 16, 16);
    defer freeSnapshot(std.testing.allocator, reopened);
    try std.testing.expectEqual(@as(usize, 3), reopened.entries.len);
    try std.testing.expectEqualStrings("{\"upgraded\":true}", reopened.metadata);
    try std.testing.expectEqualStrings("after-upgrade", reopened.requests[0].id);
    try std.testing.expectEqual(@as(i64, 2), try scalar(&store, "SELECT COUNT(*) FROM messages;"));
}

test "concurrent writers share one conversation without lost or duplicate entries" {
    const workers = 3;
    const appends_per_worker = 6;
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const path = try std.fmt.allocPrint(std.testing.allocator, ".zig-cache/tmp/{s}/conversations.sqlite3", .{temporary.sub_path});
    defer std.testing.allocator.free(path);

    var failure = std.atomic.Value(bool).init(false);
    var threads: [workers]std.Thread = undefined;
    var contexts: [workers]Worker = undefined;
    for (&contexts, &threads, 0..) |*context, *thread, worker| {
        context.* = .{ .path = path, .worker = worker, .count = appends_per_worker, .failed = &failure };
        thread.* = try std.Thread.spawn(.{}, Worker.run, .{context});
    }
    for (&threads) |thread| thread.join();

    try std.testing.expect(!failure.load(.acquire));

    var environ = std.process.Environ.Map.init(std.testing.allocator);
    defer environ.deinit();
    try environ.put("MISA_CONVERSATION_DB", path);
    var store = try Store.open(std.testing.allocator, std.testing.io, &environ);
    defer store.deinit();
    const snapshot = try store.load(std.testing.allocator, "shared", 0, workers * appends_per_worker + 8, workers * appends_per_worker + 8);
    defer freeSnapshot(std.testing.allocator, snapshot);
    try std.testing.expectEqual(@as(usize, workers * appends_per_worker), snapshot.entries.len);
    for (snapshot.entries, 1..) |record, expected| {
        try std.testing.expectEqual(@as(i64, @intCast(expected)), record.seq);
    }
    // Every request written by every worker survived; ids are unique per
    // (worker, index), so the count is exact.
    try std.testing.expectEqual(@as(usize, workers * appends_per_worker), snapshot.requests.len);
}

const Worker = struct {
    path: []const u8,
    worker: usize,
    count: usize,
    failed: *std.atomic.Value(bool),

    fn run(self: *Worker) void {
        self.write() catch self.failed.store(true, .release);
    }

    fn write(self: *Worker) !void {
        var environ = std.process.Environ.Map.init(std.testing.allocator);
        defer environ.deinit();
        try environ.put("MISA_CONVERSATION_DB", self.path);
        var store = try Store.open(std.testing.allocator, std.testing.io, &environ);
        defer store.deinit();
        for (0..self.count) |index| {
            const entries = [_]Entry{.{ .kind = "tick", .data = .{ .integer = @intCast(self.worker * 1000 + index) } }};
            var id_buffer: [32]u8 = undefined;
            const request_id = try std.fmt.bufPrint(&id_buffer, "w{d}-{d}", .{ self.worker, index });
            _ = try store.append(.{
                .conversation = "shared",
                .entries = &entries,
                .at_ms = 1000 + @as(i64, @intCast(index)),
                .request = .{ .id = request_id, .provider = "p", .model = "m", .status = "ok" },
            });
        }
    }
};
