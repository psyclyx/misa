//! Only explicitly registered fixture executables can act as provider commands.
const std = @import("std");
const process = @import("misa_process");

fn requireFixture(allocator: std.mem.Allocator, environ: *const std.process.Environ.Map, spec: process.Spec) !void {
    const root = environ.get("MISA_FIXTURE_ROOT") orelse return error.FixtureEnvironmentMissing;
    const source = environ.get("MISA_FIXTURE_PROCESSES") orelse return error.UnmatchedProviderProcessFixture;
    const executable = spec.argv[0].string;
    if (!std.fs.path.isAbsolute(executable)) return error.UnmatchedProviderProcessFixture;
    const resolved = try std.fs.path.resolve(allocator, &.{executable});
    defer allocator.free(resolved);
    if (!std.mem.startsWith(u8, resolved, root) or resolved.len <= root.len or resolved[root.len] != std.fs.path.sep)
        return error.FixturePathOutsideRoot;
    const registered = try std.json.parseFromSlice([]const []const u8, allocator, source, .{});
    defer registered.deinit();
    for (registered.value) |path| {
        if (std.mem.eql(u8, resolved, path)) return;
    }
    return error.UnmatchedProviderProcessFixture;
}

pub fn runWithActivity(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, spec: process.Spec, activity: ?process.ActivitySink) !process.Result {
    try requireFixture(allocator, environ, spec);
    var owned = spec;
    owned.environment = environ;
    return process.runWithActivity(allocator, io, owned, activity);
}

pub fn runJsonLines(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, spec: process.Spec, sink: process.StreamSink) !process.StreamResult {
    try requireFixture(allocator, environ, spec);
    var owned = spec;
    owned.environment = environ;
    return process.runJsonLines(allocator, io, owned, sink);
}
