//! Minimal MCP stdio bridge over Lua-registered tools.
const std = @import("std");
const lua = @import("misa_lua_runtime");
const file = @import("misa_file");
const process = @import("misa_process");

const max_line = 1024 * 1024;

pub fn run(allocator: std.mem.Allocator, io: std.Io, runtime: *lua.Runtime) !void {
    runtime.setTerminalInfo(.{ .interactive = false, .columns = 80, .lines = 24 });
    var sequence: usize = 0;
    while (try readLine(allocator, io)) |line| {
        defer allocator.free(line);
        if (line.len == 0) continue;
        var request = std.json.parseFromSlice(std.json.Value, allocator, line, .{ .allocate = .alloc_always }) catch {
            try writeProtocolError(allocator, io, .null, -32700, "Parse error");
            continue;
        };
        defer request.deinit();
        const object = switch (request.value) {
            .object => |value| value,
            else => {
                try writeProtocolError(allocator, io, .null, -32600, "Invalid Request");
                continue;
            },
        };
        const method = stringField(object, "method") orelse {
            try writeProtocolError(allocator, io, object.get("id") orelse .null, -32600, "Invalid Request");
            continue;
        };
        const id = object.get("id");
        if (std.mem.eql(u8, method, "notifications/initialized") or std.mem.eql(u8, method, "notifications/cancelled")) continue;
        if (id == null) continue;
        if (std.mem.eql(u8, method, "initialize")) {
            try writeInitialize(allocator, io, object, id.?);
        } else if (std.mem.eql(u8, method, "ping")) {
            try writeResultPrefix(allocator, io, id.?);
            try std.Io.File.stdout().writeStreamingAll(io, "{}}\n");
        } else if (std.mem.eql(u8, method, "tools/list")) {
            try writeTools(allocator, io, runtime, id.?);
        } else if (std.mem.eql(u8, method, "tools/call")) {
            sequence += 1;
            try callTool(allocator, io, runtime, object, id.?, sequence);
        } else {
            try writeProtocolError(allocator, io, id.?, -32601, "Method not found");
        }
    }
}

fn callTool(allocator: std.mem.Allocator, io: std.Io, runtime: *lua.Runtime, request: std.json.ObjectMap, id: std.json.Value, sequence: usize) !void {
    const params = objectField(request, "params") orelse return writeProtocolError(allocator, io, id, -32602, "Invalid params");
    const name = stringField(params, "name") orelse return writeProtocolError(allocator, io, id, -32602, "Invalid params");
    const arguments = params.get("arguments") orelse std.json.Value{ .object = .{} };
    if (arguments != .object) return writeProtocolError(allocator, io, id, -32602, "Invalid params");
    const call_id = try std.fmt.allocPrint(allocator, "mcp-{d}", .{sequence});
    defer allocator.free(call_id);
    var translated = runtime.mcpToolEffect(name, arguments, call_id, .{
        .wall_ms = std.Io.Timestamp.now(io, .real).toMilliseconds(),
        .monotonic_ms = std.Io.Timestamp.now(io, .awake).toMilliseconds(),
    }) catch {
        return writeToolResult(allocator, io, id, runtime.lastError(), true);
    };
    defer translated.deinit();
    const effect = switch (translated.value) {
        .object => |value| value,
        else => return writeToolResult(allocator, io, id, "tool translator returned invalid effect", true),
    };
    const kind = stringField(effect, "type") orelse return writeToolResult(allocator, io, id, "tool effect has no type", true);
    if (std.mem.eql(u8, kind, "dispatch")) {
        const event = objectField(effect, "event") orelse return writeToolResult(allocator, io, id, "dispatch effect has no event", true);
        if (!std.mem.eql(u8, stringField(event, "type") orelse "", "tool/result"))
            return writeToolResult(allocator, io, id, "tool dispatch did not produce a result", true);
        const text = stringField(event, "text") orelse "";
        const is_error = switch (event.get("is_error") orelse std.json.Value{ .bool = false }) {
            .bool => |value| value,
            else => true,
        };
        return writeToolResult(allocator, io, id, text, is_error);
    }
    if (std.mem.startsWith(u8, kind, "file/")) {
        const spec = file.Spec.parse(kind, effect) catch |err| return writeToolResult(allocator, io, id, @errorName(err), true);
        const text = file.run(allocator, io, spec) catch |err| return writeToolResult(allocator, io, id, @errorName(err), true);
        defer allocator.free(text);
        return writeToolResult(allocator, io, id, text, false);
    }
    if (std.mem.eql(u8, kind, "process/run")) {
        const spec = process.Spec.parse(effect) catch |err| return writeToolResult(allocator, io, id, @errorName(err), true);
        const result = process.run(allocator, io, spec) catch |err| return writeToolResult(allocator, io, id, @errorName(err), true);
        defer result.deinit(allocator);
        var text: std.ArrayList(u8) = .empty;
        defer text.deinit(allocator);
        try text.appendSlice(allocator, result.stdout);
        if (result.stderr.len > 0) {
            if (text.items.len > 0 and text.items[text.items.len - 1] != '\n') try text.append(allocator, '\n');
            try text.appendSlice(allocator, result.stderr);
        }
        if (text.items.len == 0) try text.appendSlice(allocator, if (result.status == 0) "command completed with no output" else "command failed with no output");
        return writeToolResult(allocator, io, id, text.items, result.status != 0);
    }
    return writeToolResult(allocator, io, id, "tool effect is not available through the MCP bridge", true);
}

fn writeInitialize(allocator: std.mem.Allocator, io: std.Io, request: std.json.ObjectMap, id: std.json.Value) !void {
    const params = objectField(request, "params");
    const version = if (params) |value| stringField(value, "protocolVersion") orelse "2024-11-05" else "2024-11-05";
    const response = try std.json.Stringify.valueAlloc(allocator, .{
        .jsonrpc = "2.0",
        .id = id,
        .result = .{
            .protocolVersion = version,
            // An empty tuple serializes as []; MCP capability declarations
            // are objects, and strict clients reject an array here.
            .capabilities = .{ .tools = struct {}{} },
            .serverInfo = .{ .name = "misa", .version = "0.1.0" },
        },
    }, .{});
    defer allocator.free(response);
    try std.Io.File.stdout().writeStreamingAll(io, response);
    try std.Io.File.stdout().writeStreamingAll(io, "\n");
}

fn writeTools(allocator: std.mem.Allocator, io: std.Io, runtime: *lua.Runtime, id: std.json.Value) !void {
    var owned = try runtime.mcpTools();
    defer owned.deinit();
    const tools = switch (owned.value) {
        .array => |value| value.items,
        else => return error.InvalidToolRegistry,
    };
    try writeResultPrefix(allocator, io, id);
    try std.Io.File.stdout().writeStreamingAll(io, "{\"tools\":[");
    for (tools, 0..) |tool, index| {
        const object = switch (tool) {
            .object => |value| value,
            else => return error.InvalidToolRegistry,
        };
        const item = .{
            .name = stringField(object, "name") orelse return error.InvalidToolRegistry,
            .description = stringField(object, "description") orelse return error.InvalidToolRegistry,
            .inputSchema = object.get("input_schema") orelse return error.InvalidToolRegistry,
        };
        const json = try std.json.Stringify.valueAlloc(allocator, item, .{});
        defer allocator.free(json);
        if (index > 0) try std.Io.File.stdout().writeStreamingAll(io, ",");
        try std.Io.File.stdout().writeStreamingAll(io, json);
    }
    try std.Io.File.stdout().writeStreamingAll(io, "]}}\n");
}

fn writeToolResult(allocator: std.mem.Allocator, io: std.Io, id: std.json.Value, text: []const u8, is_error: bool) !void {
    const result = try std.json.Stringify.valueAlloc(allocator, .{
        .jsonrpc = "2.0",
        .id = id,
        .result = .{
            .content = &.{.{ .type = "text", .text = text }},
            .isError = is_error,
        },
    }, .{});
    defer allocator.free(result);
    try std.Io.File.stdout().writeStreamingAll(io, result);
    try std.Io.File.stdout().writeStreamingAll(io, "\n");
}

fn writeResultPrefix(allocator: std.mem.Allocator, io: std.Io, id: std.json.Value) !void {
    const encoded = try std.json.Stringify.valueAlloc(allocator, id, .{});
    defer allocator.free(encoded);
    try std.Io.File.stdout().writeStreamingAll(io, "{\"jsonrpc\":\"2.0\",\"id\":");
    try std.Io.File.stdout().writeStreamingAll(io, encoded);
    try std.Io.File.stdout().writeStreamingAll(io, ",\"result\":");
}

fn writeProtocolError(allocator: std.mem.Allocator, io: std.Io, id: std.json.Value, code: i32, message: []const u8) !void {
    const response = try std.json.Stringify.valueAlloc(allocator, .{
        .jsonrpc = "2.0",
        .id = id,
        .@"error" = .{ .code = code, .message = message },
    }, .{});
    defer allocator.free(response);
    try std.Io.File.stdout().writeStreamingAll(io, response);
    try std.Io.File.stdout().writeStreamingAll(io, "\n");
}

fn readLine(allocator: std.mem.Allocator, io: std.Io) !?[]u8 {
    var line: std.ArrayList(u8) = .empty;
    errdefer line.deinit(allocator);
    var byte: [1]u8 = undefined;
    var saw_input = false;
    while (true) {
        const count = std.Io.File.stdin().readStreaming(io, &.{&byte}) catch |err| switch (err) {
            error.EndOfStream => break,
            else => return err,
        };
        if (count == 0) break;
        saw_input = true;
        if (byte[0] == '\n') break;
        if (byte[0] != '\r') try line.append(allocator, byte[0]);
        if (line.items.len > max_line) return error.RequestTooLarge;
    }
    if (!saw_input) {
        line.deinit(allocator);
        return null;
    }
    const owned = try line.toOwnedSlice(allocator);
    return owned;
}

fn stringField(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    return switch (object.get(name) orelse return null) {
        .string => |value| value,
        else => null,
    };
}

fn objectField(object: std.json.ObjectMap, name: []const u8) ?std.json.ObjectMap {
    return switch (object.get(name) orelse return null) {
        .object => |value| value,
        else => null,
    };
}
