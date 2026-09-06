//! Owned native syntax service. Workers serialize use of its reusable parser cache.
const std = @import("std");
const syntax = @import("root.zig");

pub const Spec = struct {
    id: []const u8,
    language: []const u8,
    source: []const u8,
    completion: []const u8,
    timeout_ms: u32 = 1000,

    pub fn parse(object: std.json.ObjectMap) !Spec {
        const spec: Spec = .{
            .id = try field(object, "id", true),
            .language = try field(object, "language", true),
            .source = try field(object, "source", false),
            .completion = try field(object, "completion", true),
            .timeout_ms = if (object.get("timeout_ms")) |value| blk: {
                if (value != .integer or value.integer < 1 or value.integer > 60000) return error.InvalidEffect;
                break :blk @intCast(value.integer);
            } else 1000,
        };
        if (spec.language.len > syntax.max_language_bytes or spec.source.len > syntax.max_source_bytes or !std.unicode.utf8ValidateSlice(spec.source)) return error.InvalidEffect;
        return spec;
    }
    fn field(object: std.json.ObjectMap, key: []const u8, nonempty: bool) ![]const u8 {
        const value = object.get(key) orelse return error.InvalidEffect;
        if (value != .string or (nonempty and value.string.len == 0)) return error.InvalidEffect;
        return value.string;
    }
};

pub const Service = struct {
    allocator: std.mem.Allocator,
    grammar_dir: []u8,
    mutex: std.Io.Mutex = .init,
    highlighter: ?syntax.Highlighter = null,

    pub fn init(allocator: std.mem.Allocator, grammar_dir: []const u8) !Service {
        return .{ .allocator = allocator, .grammar_dir = try allocator.dupe(u8, grammar_dir) };
    }
    /// Call after all tasks using the service have joined.
    pub fn deinit(self: *Service) void {
        if (self.highlighter) |*highlighter| highlighter.deinit();
        self.allocator.free(self.grammar_dir);
    }
    pub fn run(self: *Service, allocator: std.mem.Allocator, io: std.Io, spec: Spec) ![]syntax.Capture {
        try self.mutex.lock(io);
        defer self.mutex.unlock(io);
        if (self.highlighter == null) self.highlighter = try .init(self.allocator, self.grammar_dir);
        return self.highlighter.?.highlightCancelable(allocator, io, spec.language, spec.source, spec.timeout_ms);
    }
};

test "syntax work waiting for the parser cache is cancelable" {
    const io = std.testing.io;
    var service = try Service.init(std.testing.allocator, "");
    defer service.deinit();
    try service.mutex.lock(io);
    defer service.mutex.unlock(io);
    const Job = struct {
        service: *Service,
        started: std.Io.Event = .unset,
        canceled: bool = false,
        fn run(self: *@This(), context: std.Io) std.Io.Cancelable!void {
            self.started.set(context);
            const captures = self.service.run(std.testing.allocator, context, .{ .id = "test", .completion = "done", .source = "", .language = "python" }) catch |err| {
                self.canceled = err == error.Canceled;
                return;
            };
            std.testing.allocator.free(captures);
        }
    };
    var job: Job = .{ .service = &service };
    var group: std.Io.Group = .init;
    try group.concurrent(io, Job.run, .{ &job, io });
    job.started.waitUncancelable(io);
    group.cancel(io);
    try std.testing.expect(job.canceled);
    try std.testing.expect(service.highlighter == null);
}
