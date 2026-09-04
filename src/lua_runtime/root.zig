//! Ownership boundary for LuaJIT canonical state and event transactions.
const std = @import("std");
const c = @cImport({
    @cInclude("lua.h");
    @cInclude("lauxlib.h");
    @cInclude("lualib.h");
});

const framework = @embedFile("framework.lua");

pub const max_json_nesting_depth: usize = 128;
const Extension = struct { ref: c_int, path: []u8 };

pub const TerminalInfo = struct {
    interactive: bool,
    columns: usize,
    lines: usize,
};

pub const Runtime = struct {
    state: *c.lua_State,
    allocator: std.mem.Allocator,
    context_ref: c_int = c.LUA_NOREF,
    clone_ref: c_int = c.LUA_NOREF,
    traceback_ref: c_int = c.LUA_NOREF,
    extensions: std.ArrayList(Extension) = .empty,
    terminal_info: ?TerminalInfo = null,
    error_buffer: [2048]u8 = undefined,
    error_len: usize = 0,

    pub fn init(allocator: std.mem.Allocator, config_json: []const u8, config_value: std.json.Value, argv: anytype) !Runtime {
        const state = c.luaL_newstate() orelse return error.LuaInitializationFailed;
        var self: Runtime = .{ .state = state, .allocator = allocator };
        errdefer self.deinit();
        c.luaL_openlibs(state);
        // Keep an immutable registry reference; untrusted code never gets debug.
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
        // Capture the trusted bounded clone as a private registry bridge before
        // extension chunks can observe or replace it.
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(state, -1, "_clone");
        c.lua_pushnil(state);
        c.lua_setfield(state, -3, "_clone");
        c.lua_remove(state, -2);
        self.clone_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);
        try self.setContext(config_json, config_value, argv);
        return self;
    }
    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        for (self.extensions.items) |x| self.allocator.free(x.path);
        self.extensions.deinit(self.allocator);
    }

    pub fn loadExtension(self: *Runtime, path: []const u8) !void {
        self.assertStack(0);
        if (std.mem.indexOfScalar(u8, path, 0) != null) {
            self.setError("extension path contains NUL", .{});
            return error.ExtensionLoadFailed;
        }
        const z = try self.allocator.dupeZ(u8, path);
        defer self.allocator.free(z);
        self.pushTraceback();
        if (c.luaL_loadfile(self.state, z.ptr) != 0 or c.lua_pcall(self.state, 0, 1, 1) != 0) {
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

    /// Run setup in extension order and permanently close registration.
    pub fn setup(self: *Runtime) !void {
        try self.callSetup();
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_seal");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const eh = c.lua_gettop(self.state) - 1;
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
        if (c.lua_pcall(self.state, 1, 0, eh) != 0) {
            self.failLua("sealing registrations");
            c.lua_settop(self.state, 0);
            return error.ExtensionRunFailed;
        }
        self.pop(1);
        self.assertStack(0);
    }

    /// Install host terminal capabilities after terminal initialization and before dispatch.
    pub fn setTerminalInfo(self: *Runtime, info: TerminalInfo) void {
        self.assertStack(0);
        std.debug.assert(info.columns > 0 and info.lines > 0);
        self.terminal_info = info;
    }

    /// Dispatch one JSON event and return an owned JSON transaction envelope.
    pub fn dispatch(self: *Runtime, event_json: []const u8) ![]u8 {
        self.assertStack(0);
        var parsed = std.json.parseFromSlice(std.json.Value, self.allocator, event_json, .{}) catch {
            self.setError("invalid native event JSON", .{});
            return error.EventDispatchFailed;
        };
        defer parsed.deinit();
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_dispatch");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const eh = c.lua_gettop(self.state) - 1;
        self.pushJson(parsed.value, 0) catch {
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        };
        self.pushTerminalInfo(self.terminal_info orelse {
            self.setError("terminal coeffect was not initialized", .{});
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        });
        if (c.lua_pcall(self.state, 2, 1, eh) != 0) {
            self.setError("event dispatch: {s}", .{self.stackError()});
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        }
        var len: usize = 0;
        const ptr = c.lua_tolstring(self.state, -1, &len) orelse {
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        };
        const result = self.allocator.dupe(u8, ptr[0..len]) catch {
            c.lua_settop(self.state, 0);
            return error.OutOfMemory;
        };
        c.lua_settop(self.state, 0);
        return result;
    }
    /// Commit the state prepared by dispatch after native validation succeeds.
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
        self.assertStack(0);
    }

    pub fn lastError(self: *const Runtime) []const u8 {
        return self.error_buffer[0..self.error_len];
    }

    fn setContext(self: *Runtime, config_json: []const u8, config_value: std.json.Value, argv: anytype) !void {
        c.lua_createtable(self.state, 0, 3);
        _ = c.lua_pushlstring(self.state, config_json.ptr, config_json.len);
        c.lua_setfield(self.state, -2, "config_json");
        try self.pushJson(config_value, 0);
        c.lua_setfield(self.state, -2, "config");
        c.lua_createtable(self.state, @intCast(argv.len), 0);
        for (argv, 0..) |arg, i| {
            _ = c.lua_pushstring(self.state, arg.ptr);
            c.lua_rawseti(self.state, -2, @intCast(i + 1));
        }
        c.lua_setfield(self.state, -2, "argv");
        self.context_ref = c.luaL_ref(self.state, c.LUA_REGISTRYINDEX);
        self.assertStack(0);
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
        if (depth > max_json_nesting_depth) return error.ConfigNestingTooDeep;
        try self.ensureStack(3);
        switch (value) {
            .null => {
                c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
                c.lua_getfield(self.state, -1, "json_null");
                c.lua_remove(self.state, -2);
            },
            .bool => |v| c.lua_pushboolean(self.state, @intFromBool(v)),
            .integer => |v| c.lua_pushnumber(self.state, @floatFromInt(v)),
            .float => |v| c.lua_pushnumber(self.state, v),
            .number_string => |v| c.lua_pushnumber(self.state, std.fmt.parseFloat(f64, v) catch unreachable),
            .string => |v| {
                _ = c.lua_pushlstring(self.state, v.ptr, v.len);
            },
            .array => |v| {
                c.lua_createtable(self.state, @intCast(v.items.len), 0);
                for (v.items, 0..) |x, i| {
                    try self.pushJson(x, depth + 1);
                    c.lua_rawseti(self.state, -2, @intCast(i + 1));
                }
            },
            .object => |v| {
                c.lua_createtable(self.state, 0, @intCast(v.count()));
                var it = v.iterator();
                while (it.next()) |e| {
                    _ = c.lua_pushlstring(self.state, e.key_ptr.*.ptr, e.key_ptr.*.len);
                    try self.pushJson(e.value_ptr.*, depth + 1);
                    c.lua_rawset(self.state, -3);
                }
            },
        }
    }
    fn callSetup(self: *Runtime) !void {
        for (self.extensions.items, 0..) |x, i| {
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, x.ref);
            _ = c.lua_pushstring(self.state, "setup");
            _ = c.lua_rawget(self.state, -2);
            if (c.lua_type(self.state, -1) == c.LUA_TNIL) {
                self.pop(2);
                continue;
            }
            if (c.lua_type(self.state, -1) != c.LUA_TFUNCTION) {
                self.setError("extension {d} ({s}) field 'setup' must be a function", .{ i + 1, x.path });
                c.lua_settop(self.state, 0);
                return error.ExtensionRunFailed;
            }
            self.pushTraceback();
            c.lua_insert(self.state, -2);
            const eh = c.lua_gettop(self.state) - 1;
            self.pushContextClone() catch {
                self.setError("extension {d} ({s}) setup context clone: {s}", .{ i + 1, x.path, self.stackError() });
                c.lua_settop(self.state, 0);
                return error.ExtensionRunFailed;
            };
            if (c.lua_pcall(self.state, 1, 0, eh) != 0) {
                self.setError("extension {d} ({s}) setup: {s}", .{ i + 1, x.path, self.stackError() });
                c.lua_settop(self.state, 0);
                return error.ExtensionRunFailed;
            }
            self.pop(2);
        }
        self.assertStack(0);
    }
    /// Push a fresh setup context while leaving the caller's stack and error
    /// handler intact. Clone failures retain their traceback at stack top.
    fn pushContextClone(self: *Runtime) !void {
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.clone_ref);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const eh = c.lua_gettop(self.state) - 1;
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
        if (c.lua_pcall(self.state, 1, 1, eh) != 0) return error.ContextCloneFailed;
        c.lua_remove(self.state, eh);
    }
    fn pushTraceback(self: *Runtime) void {
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.traceback_ref);
    }
    fn ensureStack(self: *Runtime, n: c_int) !void {
        if (c.lua_checkstack(self.state, n) == 0) {
            self.setError("Lua stack exhausted", .{});
            return error.LuaStackExhausted;
        }
    }
    fn failLua(self: *Runtime, prefix: []const u8) void {
        self.setError("{s}: {s}", .{ prefix, self.stackError() });
        self.pop(1);
    }
    fn stackError(self: *Runtime) []const u8 {
        return std.mem.span(c.lua_tolstring(self.state, -1, null) orelse return "unknown Lua error");
    }
    fn setError(self: *Runtime, comptime fmt: []const u8, args: anytype) void {
        var w = std.Io.Writer.fixed(self.error_buffer[0 .. self.error_buffer.len - 1]);
        w.print(fmt, args) catch {};
        self.error_len = w.end;
    }
    fn pop(self: *Runtime, n: c_int) void {
        c.lua_settop(self.state, -n - 1);
    }
    fn assertStack(self: *Runtime, n: c_int) void {
        std.debug.assert(c.lua_gettop(self.state) == n);
    }
};
