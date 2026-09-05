//! LuaJIT lifetime and the checked Lua/data boundary.
const std = @import("std");
const syntax = @import("misa_syntax");
const c = @cImport({
    @cInclude("lua.h");
    @cInclude("lauxlib.h");
    @cInclude("lualib.h");
});

const framework = @embedFile("framework.lua");
pub const max_nesting_depth: usize = 128;

const Extension = struct { ref: c_int, path: []u8 };

fn syntaxHighlight(state: ?*c.lua_State) callconv(.c) c_int {
    const lua_state = state orelse return 0;
    if (c.lua_type(lua_state, 1) != c.LUA_TSTRING) return c.luaL_argerror(lua_state, 1, "language must be a string");
    if (c.lua_type(lua_state, 2) != c.LUA_TSTRING) return c.luaL_argerror(lua_state, 2, "source must be a string");
    var language_len: usize = 0;
    var source_len: usize = 0;
    const language_ptr = c.lua_tolstring(lua_state, 1, &language_len) orelse return c.luaL_argerror(lua_state, 1, "language must be a string");
    const source_ptr = c.lua_tolstring(lua_state, 2, &source_len) orelse return c.luaL_argerror(lua_state, 2, "source must be a string");
    const context_ptr = c.lua_touserdata(lua_state, c.LUA_GLOBALSINDEX - 1) orelse return pushLuaError(lua_state, "syntax highlighter is unavailable");
    const highlighter: *syntax.Highlighter = @ptrCast(@alignCast(context_ptr));
    const captures = highlighter.highlight(highlighter.allocator, language_ptr[0..language_len], source_ptr[0..source_len]) catch |err| switch (err) {
        error.InvalidLanguage => return c.luaL_argerror(lua_state, 1, "language must contain 1 to 64 safe bytes"),
        error.SourceTooLarge => return c.luaL_argerror(lua_state, 2, "source exceeds 1 MiB"),
        else => return pushLuaError(lua_state, "syntax highlighting failed"),
    };
    defer highlighter.allocator.free(captures);
    c.lua_createtable(lua_state, @intCast(captures.len), 0);
    for (captures, 0..) |capture, index| {
        c.lua_createtable(lua_state, 0, 3);
        c.lua_pushnumber(lua_state, @floatFromInt(capture.start_byte));
        c.lua_setfield(lua_state, -2, "start_byte");
        c.lua_pushnumber(lua_state, @floatFromInt(capture.end_byte));
        c.lua_setfield(lua_state, -2, "end_byte");
        _ = c.lua_pushlstring(lua_state, capture.capture.ptr, capture.capture.len);
        c.lua_setfield(lua_state, -2, "capture");
        c.lua_rawseti(lua_state, -2, @intCast(index + 1));
    }
    return 1;
}

fn pushLuaError(state: *c.lua_State, message: []const u8) c_int {
    _ = c.lua_pushlstring(state, message.ptr, message.len);
    return c.lua_error(state);
}

pub const TerminalInfo = struct {
    interactive: bool,
    columns: usize,
    lines: usize,
};

/// Trusted clocks sampled by the native event loop for each Lua transaction.
/// Milliseconds remain exactly representable by LuaJIT's number type for many
/// millennia; monotonic values are deliberately unrelated to wall time.
pub const ClockInfo = struct {
    wall_ms: i64,
    monotonic_ms: i64,
};

/// Effects and view copied out of Lua into one short-lived arena.
pub const OwnedValue = struct {
    arena: std.heap.ArenaAllocator,
    value: std.json.Value,

    pub fn deinit(self: *OwnedValue) void {
        self.arena.deinit();
    }
};

pub const Transaction = struct {
    arena: std.heap.ArenaAllocator,
    effects: []const std.json.Value,
    view: std.json.Value,

    pub fn deinit(self: *Transaction) void {
        self.arena.deinit();
    }
};

pub const Runtime = struct {
    state: *c.lua_State,
    allocator: std.mem.Allocator,
    context_ref: c_int = c.LUA_NOREF,
    json_null_ref: c_int = c.LUA_NOREF,
    traceback_ref: c_int = c.LUA_NOREF,
    extensions: std.ArrayList(Extension) = .empty,
    terminal_info: ?TerminalInfo = null,
    error_buffer: [2048]u8 = undefined,
    error_len: usize = 0,
    syntax_highlighter: *syntax.Highlighter,

    pub fn init(allocator: std.mem.Allocator, config: std.json.Value, argv: anytype, grammar_dir: []const u8) !Runtime {
        const state = c.luaL_newstate() orelse return error.LuaInitializationFailed;
        const syntax_highlighter = try allocator.create(syntax.Highlighter);
        syntax_highlighter.* = syntax.Highlighter.init(allocator, grammar_dir) catch |err| {
            allocator.destroy(syntax_highlighter);
            c.lua_close(state);
            return err;
        };
        var self: Runtime = .{ .state = state, .allocator = allocator, .syntax_highlighter = syntax_highlighter };
        errdefer self.deinit();
        c.luaL_openlibs(state);

        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "debug");
        c.lua_getfield(state, -1, "traceback");
        c.lua_remove(state, -2);
        self.traceback_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);

        self.pushTraceback();
        if (c.luaL_loadbuffer(state, framework.ptr, framework.len, "@framework.lua") != 0 or c.lua_pcall(state, 0, 0, 1) != 0) {
            self.failLua("initializing misa API");
            return error.LuaInitializationFailed;
        }
        self.pop(1);
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(state, -1, "json_null");
        c.lua_remove(state, -2);
        self.json_null_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);
        self.installSyntaxApi();
        try self.setContext(config, argv);
        return self;
    }

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        self.syntax_highlighter.deinit();
        self.allocator.destroy(self.syntax_highlighter);
        for (self.extensions.items) |extension| self.allocator.free(extension.path);
        self.extensions.deinit(self.allocator);
    }

    pub fn loadExtension(self: *Runtime, path: []const u8) !void {
        self.assertStack(0);
        if (std.mem.indexOfScalar(u8, path, 0) != null) return error.ExtensionLoadFailed;
        const path_z = try self.allocator.dupeZ(u8, path);
        defer self.allocator.free(path_z);

        self.pushTraceback();
        if (c.luaL_loadfile(self.state, path_z.ptr) != 0 or c.lua_pcall(self.state, 0, 1, 1) != 0) {
            self.failLua(path);
            c.lua_settop(self.state, 0);
            return error.ExtensionLoadFailed;
        }
        c.lua_remove(self.state, 1);
        if (c.lua_type(self.state, -1) != c.LUA_TTABLE) {
            self.setError("{s}: extension must return a table", .{path});
            self.pop(1);
            return error.ExtensionLoadFailed;
        }
        _ = c.lua_pushstring(self.state, "run");
        _ = c.lua_rawget(self.state, -2);
        if (c.lua_type(self.state, -1) != c.LUA_TNIL) {
            self.setError("{s}: extension field 'run' is obsolete; register events during setup", .{path});
            self.pop(2);
            return error.ExtensionLoadFailed;
        }
        self.pop(1);
        const copy = try self.allocator.dupe(u8, path);
        errdefer self.allocator.free(copy);
        try self.extensions.append(self.allocator, .{ .ref = c.luaL_ref(self.state, c.LUA_REGISTRYINDEX), .path = copy });
        self.assertStack(0);
    }

    pub fn setup(self: *Runtime) !void {
        try self.callSetup();
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_seal");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const error_handler = c.lua_gettop(self.state) - 1;
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
        if (c.lua_pcall(self.state, 1, 0, error_handler) != 0) {
            self.failLua("sealing registrations");
            c.lua_settop(self.state, 0);
            return error.ExtensionRunFailed;
        }
        self.pop(1);
        self.assertStack(0);
        for (self.extensions.items) |extension| {
            c.luaL_unref(self.state, c.LUA_REGISTRYINDEX, extension.ref);
            self.allocator.free(extension.path);
        }
        self.extensions.deinit(self.allocator);
        self.extensions = .empty;
    }

    pub fn setTerminalInfo(self: *Runtime, info: TerminalInfo) void {
        self.assertStack(0);
        std.debug.assert(info.columns > 0 and info.lines > 0);
        self.terminal_info = info;
    }

    /// Dispatch one event and copy only its effects/view out of Lua.
    pub fn dispatch(self: *Runtime, event_json: []const u8, clock: ClockInfo) !Transaction {
        self.assertStack(0);
        var event = std.json.parseFromSlice(std.json.Value, self.allocator, event_json, .{}) catch {
            self.setError("invalid native event JSON", .{});
            return error.EventDispatchFailed;
        };
        defer event.deinit();

        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_dispatch");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const error_handler = c.lua_gettop(self.state) - 1;
        try self.pushJson(event.value, 0);
        self.pushTerminalInfo(self.terminal_info orelse return error.TerminalInfoMissing);
        self.pushClockInfo(clock);
        if (c.lua_pcall(self.state, 3, 2, error_handler) != 0) {
            self.setError("event dispatch: {s}", .{self.stackError()});
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        }

        var transaction: Transaction = .{
            .arena = std.heap.ArenaAllocator.init(self.allocator),
            .effects = undefined,
            .view = undefined,
        };
        errdefer transaction.deinit();
        var active: std.ArrayList(?*const anyopaque) = .empty;
        defer active.deinit(self.allocator);
        const arena = transaction.arena.allocator();
        const effects_value = self.readLuaValue(arena, -2, 0, &active) catch |err| {
            self.setError("invalid effects: {s}", .{@errorName(err)});
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        };
        transaction.effects = switch (effects_value) {
            .array => |array| array.items,
            else => {
                self.setError("effects must be an array", .{});
                c.lua_settop(self.state, 0);
                return error.EventDispatchFailed;
            },
        };
        transaction.view = self.readLuaValue(arena, -1, 0, &active) catch |err| {
            self.setError("invalid view: {s}", .{@errorName(err)});
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        };
        c.lua_settop(self.state, 0);
        return transaction;
    }

    pub fn mcpTools(self: *Runtime) !OwnedValue {
        return self.callMcp("_mcp_tools", null, null, null);
    }

    pub fn mcpToolEffect(self: *Runtime, name: []const u8, arguments: std.json.Value, id: []const u8) !OwnedValue {
        return self.callMcp("_mcp_tool_effect", name, arguments, id);
    }

    fn callMcp(self: *Runtime, function: []const u8, name: ?[]const u8, arguments: ?std.json.Value, id: ?[]const u8) !OwnedValue {
        self.assertStack(0);
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, function.ptr);
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const argument_count: c_int = if (name) |tool_name| blk: {
            _ = c.lua_pushlstring(self.state, tool_name.ptr, tool_name.len);
            try self.pushJson(arguments.?, 0);
            _ = c.lua_pushlstring(self.state, id.?.ptr, id.?.len);
            break :blk 3;
        } else 0;
        if (c.lua_pcall(self.state, argument_count, 1, 1) != 0) {
            self.setError("MCP policy: {s}", .{self.stackError()});
            c.lua_settop(self.state, 0);
            return error.McpPolicyFailed;
        }
        var result: OwnedValue = .{ .arena = std.heap.ArenaAllocator.init(self.allocator), .value = undefined };
        errdefer result.deinit();
        var active: std.ArrayList(?*const anyopaque) = .empty;
        defer active.deinit(self.allocator);
        result.value = self.readLuaValue(result.arena.allocator(), -1, 0, &active) catch |err| {
            self.setError("invalid MCP policy value: {s}", .{@errorName(err)});
            c.lua_settop(self.state, 0);
            return error.McpPolicyFailed;
        };
        c.lua_settop(self.state, 0);
        return result;
    }

    pub fn commitTransaction(self: *Runtime) !void {
        self.assertStack(0);
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_commit");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        if (c.lua_pcall(self.state, 0, 0, 1) != 0) {
            self.failLua("committing transaction");
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        }
        self.pop(1);
    }

    pub fn lastError(self: *const Runtime) []const u8 {
        return self.error_buffer[0..self.error_len];
    }

    fn installSyntaxApi(self: *Runtime) void {
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_createtable(self.state, 0, 1);
        c.lua_pushlightuserdata(self.state, self.syntax_highlighter);
        c.lua_pushcclosure(self.state, syntaxHighlight, 1);
        c.lua_setfield(self.state, -2, "highlight");
        c.lua_setfield(self.state, -2, "syntax");
        self.pop(1);
    }

    fn setContext(self: *Runtime, config: std.json.Value, argv: anytype) !void {
        c.lua_createtable(self.state, 0, 2);
        try self.pushJson(config, 0);
        c.lua_setfield(self.state, -2, "config");
        c.lua_createtable(self.state, @intCast(argv.len), 0);
        for (argv, 0..) |arg, index| {
            _ = c.lua_pushlstring(self.state, arg.ptr, arg.len);
            c.lua_rawseti(self.state, -2, @intCast(index + 1));
        }
        c.lua_setfield(self.state, -2, "argv");
        self.context_ref = c.luaL_ref(self.state, c.LUA_REGISTRYINDEX);
    }

    fn pushTerminalInfo(self: *Runtime, info: TerminalInfo) void {
        c.lua_createtable(self.state, 0, 3);
        c.lua_pushboolean(self.state, @intFromBool(info.interactive));
        c.lua_setfield(self.state, -2, "interactive");
        c.lua_pushnumber(self.state, @floatFromInt(info.columns));
        c.lua_setfield(self.state, -2, "columns");
        c.lua_pushnumber(self.state, @floatFromInt(info.lines));
        c.lua_setfield(self.state, -2, "lines");
    }

    fn pushClockInfo(self: *Runtime, info: ClockInfo) void {
        c.lua_createtable(self.state, 0, 2);
        c.lua_pushnumber(self.state, @floatFromInt(info.wall_ms));
        c.lua_setfield(self.state, -2, "wall_ms");
        c.lua_pushnumber(self.state, @floatFromInt(info.monotonic_ms));
        c.lua_setfield(self.state, -2, "monotonic_ms");
    }

    fn pushJson(self: *Runtime, value: std.json.Value, depth: usize) !void {
        if (depth > max_nesting_depth) return error.MaximumNestingDepth;
        try self.ensureStack(3);
        switch (value) {
            .null => {
                c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
                c.lua_getfield(self.state, -1, "json_null");
                c.lua_remove(self.state, -2);
            },
            .bool => |item| c.lua_pushboolean(self.state, @intFromBool(item)),
            .integer => |item| c.lua_pushnumber(self.state, @floatFromInt(item)),
            .float => |item| c.lua_pushnumber(self.state, item),
            .number_string => |item| c.lua_pushnumber(self.state, try std.fmt.parseFloat(f64, item)),
            .string => |item| _ = c.lua_pushlstring(self.state, item.ptr, item.len),
            .array => |array| {
                c.lua_createtable(self.state, @intCast(array.items.len), 0);
                for (array.items, 0..) |item, index| {
                    try self.pushJson(item, depth + 1);
                    c.lua_rawseti(self.state, -2, @intCast(index + 1));
                }
            },
            .object => |object| {
                c.lua_createtable(self.state, 0, @intCast(object.count()));
                var iterator = object.iterator();
                while (iterator.next()) |entry| {
                    _ = c.lua_pushlstring(self.state, entry.key_ptr.*.ptr, entry.key_ptr.*.len);
                    try self.pushJson(entry.value_ptr.*, depth + 1);
                    c.lua_rawset(self.state, -3);
                }
            },
        }
    }

    fn readLuaValue(self: *Runtime, allocator: std.mem.Allocator, index: c_int, depth: usize, active: *std.ArrayList(?*const anyopaque)) anyerror!std.json.Value {
        if (depth > max_nesting_depth) return error.MaximumNestingDepth;
        const absolute = self.absoluteIndex(index);
        return switch (c.lua_type(self.state, absolute)) {
            c.LUA_TNIL => .null,
            c.LUA_TBOOLEAN => .{ .bool = c.lua_toboolean(self.state, absolute) != 0 },
            c.LUA_TNUMBER => blk: {
                const number = c.lua_tonumber(self.state, absolute);
                if (!std.math.isFinite(number)) return error.NonFiniteNumber;
                if (@trunc(number) == number and number >= @as(f64, @floatFromInt(std.math.minInt(i64))) and number <= @as(f64, @floatFromInt(std.math.maxInt(i64))))
                    break :blk .{ .integer = @intFromFloat(number) };
                break :blk .{ .float = number };
            },
            c.LUA_TSTRING => blk: {
                var len: usize = 0;
                const ptr = c.lua_tolstring(self.state, absolute, &len) orelse return error.InvalidString;
                break :blk .{ .string = try allocator.dupe(u8, ptr[0..len]) };
            },
            c.LUA_TTABLE => if (self.isJsonNull(absolute)) .null else try self.readLuaTable(allocator, absolute, depth, active),
            else => error.UnsupportedLuaValue,
        };
    }

    fn readLuaTable(self: *Runtime, allocator: std.mem.Allocator, index: c_int, depth: usize, active: *std.ArrayList(?*const anyopaque)) anyerror!std.json.Value {
        const identity = c.lua_topointer(self.state, index);
        for (active.items) |item| if (item == identity) return error.CyclicLuaValue;
        try active.append(self.allocator, identity);
        defer _ = active.pop();

        var count: usize = 0;
        var maximum: usize = 0;
        var array = true;
        c.lua_pushnil(self.state);
        while (c.lua_next(self.state, index) != 0) {
            count += 1;
            if (c.lua_type(self.state, -2) == c.LUA_TNUMBER) {
                const key = c.lua_tonumber(self.state, -2);
                if (!std.math.isFinite(key) or key < 1 or @trunc(key) != key or key > @as(f64, @floatFromInt(std.math.maxInt(usize)))) array = false else maximum = @max(maximum, @as(usize, @intFromFloat(key)));
            } else array = false;
            self.pop(1);
        }

        if (array and maximum == count) {
            var result = std.json.Array.init(allocator);
            try result.ensureTotalCapacity(count);
            for (1..count + 1) |item_index| {
                _ = c.lua_rawgeti(self.state, index, @intCast(item_index));
                result.appendAssumeCapacity(try self.readLuaValue(allocator, -1, depth + 1, active));
                self.pop(1);
            }
            return .{ .array = result };
        }

        var result: std.json.ObjectMap = .{};
        c.lua_pushnil(self.state);
        while (c.lua_next(self.state, index) != 0) {
            if (c.lua_type(self.state, -2) != c.LUA_TSTRING) return error.InvalidObjectKey;
            var len: usize = 0;
            const key_ptr = c.lua_tolstring(self.state, -2, &len) orelse return error.InvalidObjectKey;
            const key = try allocator.dupe(u8, key_ptr[0..len]);
            try result.put(allocator, key, try self.readLuaValue(allocator, -1, depth + 1, active));
            self.pop(1);
        }
        return .{ .object = result };
    }

    fn isJsonNull(self: *Runtime, index: c_int) bool {
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.json_null_ref);
        defer self.pop(1);
        return c.lua_rawequal(self.state, index, -1) != 0;
    }

    fn callSetup(self: *Runtime) !void {
        for (self.extensions.items, 0..) |extension, index| {
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, extension.ref);
            _ = c.lua_pushstring(self.state, "setup");
            _ = c.lua_rawget(self.state, -2);
            if (c.lua_type(self.state, -1) == c.LUA_TNIL) {
                self.pop(2);
                continue;
            }
            if (c.lua_type(self.state, -1) != c.LUA_TFUNCTION) {
                self.setError("extension {d} ({s}) field 'setup' must be a function", .{ index + 1, extension.path });
                c.lua_settop(self.state, 0);
                return error.ExtensionRunFailed;
            }
            self.pushTraceback();
            c.lua_insert(self.state, -2);
            const error_handler = c.lua_gettop(self.state) - 1;
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
            if (c.lua_pcall(self.state, 1, 0, error_handler) != 0) {
                self.setError("extension {d} ({s}) setup: {s}", .{ index + 1, extension.path, self.stackError() });
                c.lua_settop(self.state, 0);
                return error.ExtensionRunFailed;
            }
            self.pop(2);
        }
    }

    fn absoluteIndex(self: *Runtime, index: c_int) c_int {
        return if (index > 0 or index <= c.LUA_REGISTRYINDEX) index else c.lua_gettop(self.state) + index + 1;
    }

    fn pushTraceback(self: *Runtime) void {
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.traceback_ref);
    }
    fn ensureStack(self: *Runtime, count: c_int) !void {
        if (c.lua_checkstack(self.state, count) == 0) return error.LuaStackExhausted;
    }
    fn failLua(self: *Runtime, prefix: []const u8) void {
        self.setError("{s}: {s}", .{ prefix, self.stackError() });
        self.pop(1);
    }
    fn stackError(self: *Runtime) []const u8 {
        return std.mem.span(c.lua_tolstring(self.state, -1, null) orelse return "unknown Lua error");
    }
    fn setError(self: *Runtime, comptime format: []const u8, args: anytype) void {
        var writer = std.Io.Writer.fixed(self.error_buffer[0 .. self.error_buffer.len - 1]);
        writer.print(format, args) catch {};
        self.error_len = writer.end;
    }
    fn pop(self: *Runtime, count: c_int) void {
        c.lua_settop(self.state, -count - 1);
    }
    fn assertStack(self: *Runtime, count: c_int) void {
        std.debug.assert(c.lua_gettop(self.state) == count);
    }
};
