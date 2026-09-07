//! LuaJIT lifetime and the checked Lua/data boundary.
const std = @import("std");
const c = @cImport({
    @cInclude("lua.h");
    @cInclude("lauxlib.h");
    @cInclude("lualib.h");
});

const framework = @embedFile("framework.fnl");
const state_updates = @embedFile("state.fnl");
const fennel = @embedFile("vendor/fennel.lua");
pub const max_nesting_depth: usize = 128;

const Extension = struct { ref: c_int, path: []u8 };

pub const TerminalInfo = struct {
    interactive: bool,
    images: bool = false,
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
    fennel_dofile_ref: c_int = c.LUA_NOREF,
    extensions: std.ArrayList(Extension) = .empty,
    terminal_info: ?TerminalInfo = null,
    error_buffer: [2048]u8 = undefined,
    error_len: usize = 0,

    pub fn init(allocator: std.mem.Allocator, config: std.json.Value, argv: anytype) !Runtime {
        const state = c.luaL_newstate() orelse return error.LuaInitializationFailed;
        var self: Runtime = .{ .state = state, .allocator = allocator };
        errdefer self.deinit();
        c.luaL_openlibs(state);

        try self.initializeFennel();
        self.assertStack(0);
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(state, -1, "json_null");
        c.lua_remove(state, -2);
        self.json_null_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);
        try self.setContext(config, argv);
        return self;
    }

    /// Bootstrap the bundled compiler, register its require searcher, and
    /// evaluate the framework with source maps retained for Fennel tracebacks.
    fn initializeFennel(self: *Runtime) !void {
        const state = self.state;
        if (c.luaL_loadbuffer(state, fennel.ptr, fennel.len, "@fennel.lua") != 0) {
            self.failLua("loading bundled Fennel compiler");
            return error.LuaInitializationFailed;
        }
        // The compiler needs IO/debug even after policy removes those globals.
        // Its private environment captures the libraries; _G still denotes the
        // policy environment used to compile and execute extension code.
        c.lua_createtable(state, 0, 64);
        c.lua_pushnil(state);
        while (c.lua_next(state, c.LUA_GLOBALSINDEX) != 0) {
            c.lua_pushvalue(state, -2);
            c.lua_pushvalue(state, -2);
            c.lua_rawset(state, 2);
            self.pop(1);
        }
        _ = c.lua_setfenv(state, 1);
        if (c.lua_pcall(state, 0, 1, 0) != 0) {
            self.failLua("initializing bundled Fennel compiler");
            return error.LuaInitializationFailed;
        }
        // Keep the compiler available through the conventional require API.
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "package");
        c.lua_getfield(state, -1, "loaded");
        c.lua_pushvalue(state, 1);
        c.lua_setfield(state, -2, "fennel");
        self.pop(2);
        c.lua_getfield(state, 1, "traceback");
        self.traceback_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "debug");
        self.pushTraceback();
        c.lua_setfield(state, -2, "traceback");
        self.pop(1);
        c.lua_getfield(state, 1, "dofile");
        self.fennel_dofile_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);
        self.pushTraceback();
        c.lua_getfield(state, 1, "eval");
        _ = c.lua_pushlstring(state, state_updates.ptr, state_updates.len);
        c.lua_createtable(state, 0, 1);
        _ = c.lua_pushstring(state, "state.fnl");
        c.lua_setfield(state, -2, "filename");
        if (c.lua_pcall(state, 2, 1, 2) != 0) {
            self.failLua("initializing state updates");
            return error.LuaInitializationFailed;
        }
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "package");
        c.lua_getfield(state, -1, "loaded");
        c.lua_pushvalue(state, 3);
        c.lua_setfield(state, -2, "misa.runtime.state");
        self.pop(3);
        c.lua_getfield(state, 1, "eval");
        _ = c.lua_pushlstring(state, framework.ptr, framework.len);
        c.lua_createtable(state, 0, 1);
        _ = c.lua_pushstring(state, "framework.fnl");
        c.lua_setfield(state, -2, "filename");
        if (c.lua_pcall(state, 2, 0, 2) != 0) {
            self.failLua("initializing misa API");
            return error.LuaInitializationFailed;
        }
        c.lua_getfield(state, 1, "install");
        if (c.lua_pcall(state, 0, 0, 2) != 0) {
            self.failLua("installing Fennel module searcher");
            return error.LuaInitializationFailed;
        }
        self.pop(2);
    }

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        for (self.extensions.items) |extension| self.allocator.free(extension.path);
        self.extensions.deinit(self.allocator);
    }

    pub fn loadExtension(self: *Runtime, path: []const u8) !void {
        self.assertStack(0);
        if (std.mem.indexOfScalar(u8, path, 0) != null) return error.ExtensionLoadFailed;
        const path_z = try self.allocator.dupeZ(u8, path);
        defer self.allocator.free(path_z);

        self.pushTraceback();
        const status = if (std.mem.endsWith(u8, path, ".fnl")) blk: {
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.fennel_dofile_ref);
            _ = c.lua_pushlstring(self.state, path.ptr, path.len);
            break :blk c.lua_pcall(self.state, 1, 1, 1);
        } else blk: {
            const loaded = c.luaL_loadfile(self.state, path_z.ptr);
            break :blk if (loaded != 0) loaded else c.lua_pcall(self.state, 0, 1, 1);
        };
        if (status != 0) {
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

    /// Process identity for extensions that spawn another instance of the
    /// current application, preserving its explicitly selected configuration.
    pub fn setHostInfo(self: *Runtime, executable: []const u8, config_path: []const u8) void {
        c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
        c.lua_createtable(self.state, 0, 2);
        _ = c.lua_pushlstring(self.state, executable.ptr, executable.len);
        c.lua_setfield(self.state, -2, "executable");
        _ = c.lua_pushlstring(self.state, config_path.ptr, config_path.len);
        c.lua_setfield(self.state, -2, "config_path");
        c.lua_setfield(self.state, -2, "host");
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
        c.lua_pushboolean(self.state, @intFromBool(info.images));
        c.lua_setfield(self.state, -2, "images");
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
            c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
            c.lua_getfield(self.state, -1, "_setup");
            c.lua_remove(self.state, -2);
            self.pushTraceback();
            c.lua_insert(self.state, -2);
            const error_handler = c.lua_gettop(self.state) - 1;
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, extension.ref);
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
            if (c.lua_pcall(self.state, 2, 0, error_handler) != 0) {
                self.setError("extension {d} ({s}) setup: {s}", .{ index + 1, extension.path, self.stackError() });
                c.lua_settop(self.state, 0);
                return error.ExtensionRunFailed;
            }
            self.pop(1);
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

test "bundled Fennel loads extensions after framework restrictions" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    try runtime.loadExtension("src/lua_runtime/fixtures/extension.fnl");
    try runtime.setup();
}

test "Fennel loading reports source location and restores stack after failure" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    try std.testing.expectError(error.ExtensionLoadFailed, runtime.loadExtension("src/lua_runtime/fixtures/failure.fnl"));
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "failure.fnl:2") != null);
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "Fennel fixture failed") != null);
    try std.testing.expectError(error.ExtensionLoadFailed, runtime.loadExtension("src/lua_runtime/fixtures/missing.fnl"));
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "missing.fnl") != null);
    try runtime.loadExtension("src/lua_runtime/fixtures/extension.fnl");
    try runtime.setup();
}

test "setup effects expand in order and remain outside event dispatch" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    const source =
        \\local producer = {setup = function()
        \\  local result = {fx = {{type = "register/service", name = "deferred", value = 7}}}
        \\  assert(misa.deferred == nil)
        \\  return result
        \\end}
        \\local declaration = producer.setup()
        \\assert(misa.deferred == nil)
        \\misa._setup_effects(declaration)
        \\misa._setup({setup = function()
        \\  assert(misa.deferred == 7)
        \\  return {fx = {{type = "register/service", name = "ordered", value = 8}}}
        \\end}, {})
        \\misa._setup({setup = function()
        \\  assert(misa.ordered == 8)
        \\end}, {})
        \\assert(misa.reg_event == nil)
        \\misa._setup({setup = function(context)
        \\  assert(context.marker == true)
        \\  return {fx = {
        \\    {type = "register/setup-effect", name = "register/fixture-expand", handler = function(effect)
        \\      return {fx = {{type = "register/service", name = "fixture.value", value = effect.value}}}
        \\    end},
        \\    {type = "register/fixture-expand", value = 42},
        \\    {type = "register/action", value = {id = "fixture", label = "Fixture", event = {type = "fixture"}}},
        \\    {type = "register/fx", name = "fixture/translate", handler = function()
        \\      return {type = "register/fixture-expand", value = 0}
        \\    end},
        \\    {type = "register/event", name = "fixture", handler = function(db)
        \\      return {db = db, fx = {{type = "fixture/translate"}}}
        \\    end}
        \\  }}
        \\end}, {marker = true})
        \\assert(misa.fixture.value == 42)
        \\assert(misa.action("fixture").binding.action == "fixture")
        \\assert(misa.has_setup_effect("register/fixture-expand"))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "unknown"}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {[2] = {type = "unknown"}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "register/service", name = "fixture.value", value = 0}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "register/fx", name = "register/fixture-expand", handler = function() end}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "register/fx", name = "register/future", handler = function() end}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "register/setup-effect", name = "fixture/translate", handler = function() end}}}))
        \\local namespace_ok, namespace_error = pcall(misa._setup_effects, {fx = {{type = "register/setup-effect", name = "dispatch", handler = function() end}}})
        \\assert(not namespace_ok and tostring(namespace_error):match("register/ namespace"), tostring(namespace_error))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "register/setup-effect", name = "register/", handler = function() end}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {}, db = {}}))
        \\assert(not pcall(misa._setup_effects, {fx = {oops = {}}}))
        \\assert(not pcall(misa._setup_effects, {fx = {{type = "register/service", name = "invalid..path", value = 1}}}))
        \\misa._setup_effects({fx = {{type = "register/setup-effect", name = "register/fixture-recurse", handler = function()
        \\  return {fx = {{type = "register/fixture-recurse"}}}
        \\end}}})
        \\local recursive_ok, recursive_error = pcall(misa._setup_effects, {fx = {{type = "register/fixture-recurse"}}})
        \\assert(not recursive_ok and tostring(recursive_error):match("too deep"), tostring(recursive_error))
        \\misa._seal({config = {}, argv = {}})
        \\assert(not pcall(misa._setup_effects, {fx = {}}))
        \\local ok, err = pcall(misa._dispatch, {type = "fixture"}, {}, {monotonic_ms = 0, wall_ms = 0})
        \\assert(not ok and tostring(err):match("setup effects cannot run during event dispatch"), tostring(err))
    ;
    try std.testing.expectEqual(@as(c_int, 0), c.luaL_loadbuffer(runtime.state, source.ptr, source.len, "@setup-effects-test.lua"));
    if (c.lua_pcall(runtime.state, 0, 0, 0) != 0) {
        std.debug.print("setup effects test: {s}\n", .{runtime.stackError()});
        return error.SetupEffectsTestFailed;
    }
    runtime.assertStack(0);
}
