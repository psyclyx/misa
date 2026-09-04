//! LuaJIT lifetime and the checked Lua/data boundary.
const std = @import("std");
const c = @cImport({
    @cInclude("lua.h");
    @cInclude("lauxlib.h");
    @cInclude("lualib.h");
});

const framework = @embedFile("framework.lua");
pub const max_nesting_depth: usize = 128;

const Extension = struct { ref: c_int, path: []u8 };

pub const TerminalInfo = struct {
    interactive: bool,
    columns: usize,
    lines: usize,
};

/// Effects and view copied out of Lua into one short-lived arena.
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

    pub fn init(allocator: std.mem.Allocator, config: std.json.Value, argv: anytype) !Runtime {
        const state = c.luaL_newstate() orelse return error.LuaInitializationFailed;
        var self: Runtime = .{ .state = state, .allocator = allocator };
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
        try self.setContext(config, argv);
        return self;
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
    }

    pub fn setTerminalInfo(self: *Runtime, info: TerminalInfo) void {
        self.assertStack(0);
        std.debug.assert(info.columns > 0 and info.lines > 0);
        self.terminal_info = info;
    }

    /// Dispatch one event and copy only its effects/view out of Lua.
    pub fn dispatch(self: *Runtime, event_json: []const u8) !Transaction {
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
        if (c.lua_pcall(self.state, 2, 2, error_handler) != 0) {
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

    fn setContext(self: *Runtime, config: std.json.Value, argv: anytype) !void {
        c.lua_createtable(self.state, 0, 2);
        try self.pushJson(config, 0);
        c.lua_setfield(self.state, -2, "config");
        c.lua_createtable(self.state, @intCast(argv.len), 0);
        for (argv, 0..) |arg, index| {
            _ = c.lua_pushstring(self.state, arg.ptr);
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
