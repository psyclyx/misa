const std = @import("std");
const options = @import("integration_options");
pub const io = std.testing.io;

/// Owns a temporary application environment and every allocation for one case.
pub const Harness = struct {
    arena: *std.heap.ArenaAllocator,
    temporary: std.testing.TmpDir,
    directory: []const u8,
    binary: []const u8,
    environ: std.process.Environ.Map,
    provider_executables: std.ArrayList([]const u8) = .empty,

    pub fn init() !Harness {
        const arena = try std.testing.allocator.create(std.heap.ArenaAllocator);
        errdefer std.testing.allocator.destroy(arena);
        arena.* = std.heap.ArenaAllocator.init(std.testing.allocator);
        errdefer arena.deinit();
        const gpa = arena.allocator();
        var temporary = std.testing.tmpDir(.{});
        errdefer temporary.cleanup();
        const directory = try temporary.dir.realPathFileAlloc(io, ".", gpa);
        var environ = std.process.Environ.Map.init(gpa);
        var host = try std.testing.environ.createMap(gpa);
        defer host.deinit();
        // Fixtures own their environment. Only tool discovery, locale, and the
        // explicitly supplied grammar dependency come from the build process.
        for ([_][]const u8{ "PATH", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "TERM", "MISA_TREE_SITTER_DIR" }) |key| {
            if (host.get(key)) |value| try environ.put(key, value);
        }
        try environ.put("HOME", directory);
        try environ.put("MISA_FIXTURE_ROOT", directory);
        try environ.put("CODEX_HOME", try std.fs.path.join(gpa, &.{ directory, "codex" }));
        try environ.put("CLAUDE_CONFIG_DIR", try std.fs.path.join(gpa, &.{ directory, "claude-config" }));
        try environ.put("XDG_CACHE_HOME", try std.fs.path.join(gpa, &.{ directory, "cache" }));
        try environ.put("XDG_DATA_HOME", try std.fs.path.join(gpa, &.{ directory, "data" }));
        // Exercise the installed generated catalog by default. Custom fixture
        // paths remain literal Fennel/Lua; source overrides have explicit tests.
        try environ.put("MISA_AUTH_FILE", try std.fs.path.join(gpa, &.{ directory, "auth.json" }));
        try environ.put("MISA_STATE_FILE", try std.fs.path.join(gpa, &.{ directory, "application-state.json" }));
        try environ.put("XDG_STATE_HOME", directory);
        try environ.put("XDG_CONFIG_HOME", directory);
        return .{ .arena = arena, .temporary = temporary, .directory = directory, .binary = try std.fs.path.resolve(gpa, &.{ options.source_root, options.binary }), .environ = environ };
    }

    pub fn deinit(self: *Harness) void {
        self.temporary.cleanup();
        self.arena.deinit();
        std.testing.allocator.destroy(self.arena);
    }

    pub fn allocator(self: *Harness) std.mem.Allocator {
        return self.arena.allocator();
    }

    pub fn path(self: *Harness, name: []const u8) ![]const u8 {
        return std.fs.path.join(self.allocator(), &.{ self.directory, name });
    }

    /// Expand only explicit fixture placeholders; this never evaluates shell text.
    pub fn expand(self: *Harness, value: []const u8) ![]const u8 {
        var result = value;
        const replacements = [_][2][]const u8{
            .{ "@ROOT@", options.source_root },
            .{ "@WORK@", self.directory },
            .{ "@BIN@", self.binary },
        };
        for (replacements) |replacement| result = try std.mem.replaceOwned(u8, self.allocator(), result, replacement[0], replacement[1]);
        return result;
    }

    pub fn write(self: *Harness, name: []const u8, value: []const u8) !void {
        try self.temporary.dir.writeFile(io, .{ .sub_path = name, .data = try self.expand(value) });
    }

    pub fn executable(self: *Harness, name: []const u8, value: []const u8) !void {
        try self.temporary.dir.writeFile(io, .{ .sub_path = name, .data = try self.expand(value), .flags = .{ .permissions = .fromMode(0o700) } });
        try self.provider_executables.append(self.allocator(), try self.path(name));
        try self.environ.put("MISA_FIXTURE_PROCESSES", try std.json.Stringify.valueAlloc(self.allocator(), self.provider_executables.items, .{}));
    }

    pub fn claudeFixture(self: *Harness, name: []const u8) !void {
        try self.environ.put("MISA_FIXTURE_CLAUDE", try self.path(name));
    }

    pub fn read(self: *Harness, name: []const u8) ![]const u8 {
        return self.temporary.dir.readFileAlloc(io, name, self.allocator(), .limited(1024 * 1024));
    }

    pub fn config(self: *Harness, value: []const u8) !void {
        try self.write("config.fnl", value);
        try self.environ.put("MISA_CONFIG", try self.path("config.fnl"));
    }

    pub const Invocation = struct {
        args: []const []const u8 = &.{},
        input: []const u8 = "",
        binary: ?[]const u8 = null,
        input_delay_ms: u32 = 0,
        cwd: ?[]const u8 = null,
        timeout_ms: u32 = 15000,
    };

    /// The deadline includes stdin delivery, output capture, and child termination.
    pub fn run(self: *Harness, invocation: Invocation) !std.process.RunResult {
        const Completion = union(enum) { process: anyerror!std.process.RunResult, deadline: std.Io.Cancelable!void };
        var buffer: [2]Completion = undefined;
        var select = std.Io.Select(Completion).init(io, &buffer);
        defer select.cancelDiscard();
        try select.concurrent(.deadline, waitDeadline, .{invocation.timeout_ms});
        try select.concurrent(.process, runProcess, .{ self, invocation });
        return switch (try select.await()) {
            .process => |result| result,
            .deadline => error.Timeout,
        };
    }

    fn waitDeadline(milliseconds: u32) std.Io.Cancelable!void {
        try std.Io.sleep(io, .fromMilliseconds(milliseconds), .awake);
    }

    fn runProcess(self: *Harness, invocation: Invocation) anyerror!std.process.RunResult {
        var argv: std.ArrayList([]const u8) = .empty;
        try argv.append(self.allocator(), invocation.binary orelse self.binary);
        for (invocation.args) |arg| try argv.append(self.allocator(), try self.expand(arg));
        var child = try std.process.spawn(io, .{
            .argv = argv.items,
            .environ_map = &self.environ,
            .cwd = .{ .path = invocation.cwd orelse self.directory },
            .stdin = .pipe,
            .stdout = .pipe,
            .stderr = .pipe,
        });
        defer child.kill(io);
        var group: std.Io.Group = .init;
        defer group.cancel(io);
        const input = child.stdin.?;
        child.stdin = null;
        group.async(io, pumpInput, .{ input, invocation.input, invocation.input_delay_ms });
        var buffer: std.Io.File.MultiReader.Buffer(2) = undefined;
        var reader: std.Io.File.MultiReader = undefined;
        reader.init(self.allocator(), io, buffer.toStreams(), &.{ child.stdout.?, child.stderr.? });
        defer reader.deinit();
        while (reader.fill(4096, .none)) |_| {
            if (reader.reader(0).buffered().len > 4 * 1024 * 1024 or reader.reader(1).buffered().len > 4 * 1024 * 1024) return error.StreamTooLong;
        } else |err| switch (err) {
            error.EndOfStream => {},
            else => return err,
        }
        try reader.checkAnyError();
        try group.await(io);
        return .{ .term = try child.wait(io), .stdout = try reader.toOwnedSlice(0), .stderr = try reader.toOwnedSlice(1) };
    }

    fn pumpInput(file: std.Io.File, input: []const u8, delay_ms: u32) void {
        defer file.close(io);
        if (delay_ms != 0) std.Io.sleep(io, .fromMilliseconds(delay_ms), .awake) catch return;
        file.writeStreamingAll(io, input) catch {};
    }

    pub fn expect(self: *Harness, invocation: Invocation, expected: []const u8) !void {
        const result = try self.run(invocation);
        try success(result);
        try std.testing.expectEqualStrings(expected, result.stdout);
    }
};

pub fn success(result: std.process.RunResult) !void {
    if (result.term != .exited or result.term.exited != 0) {
        std.debug.print("child failed ({any})\nstdout:\n{s}\nstderr:\n{s}\n", .{ result.term, result.stdout, result.stderr });
        return error.ChildFailed;
    }
}

pub fn contains(haystack: []const u8, needle: []const u8) !void {
    if (std.mem.indexOf(u8, haystack, needle) == null) {
        std.debug.print("missing {s} in:\n{s}\n", .{ needle, haystack });
        return error.MissingOutput;
    }
}
