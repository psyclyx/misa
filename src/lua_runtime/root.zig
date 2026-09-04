//! Ownership boundary for the embedded system LuaJIT state.
const std = @import("std");
const c = @cImport({
    @cInclude("lua.h");
    @cInclude("lauxlib.h");
    @cInclude("lualib.h");
});

const bootstrap =
    \\local handlers = {}
    \\misa = {}
    \\function misa.register(name, handler)
    \\  assert(type(name) == 'string', 'handler name must be a string')
    \\  assert(type(handler) == 'function', 'handler must be a function')
    \\  local list = handlers[name] or {}; handlers[name] = list
    \\  list[#list + 1] = handler
    \\end
    \\function misa.call(name, ...)
    \\  local results = { n = 0 }
    \\  for _, handler in ipairs(handlers[name] or {}) do
    \\    results.n = results.n + 1
    \\    results[results.n] = handler(...)
    \\  end
    \\  return results
    \\end
;

const Extension = struct {
    ref: c_int,
    path: []u8,
};

pub const Runtime = struct {
    state: *c.lua_State,
    allocator: std.mem.Allocator,
    context_ref: c_int = c.LUA_NOREF,
    extensions: std.ArrayList(Extension) = .empty,
    error_buffer: [1024]u8 = undefined,
    error_len: usize = 0,

    pub fn init(allocator: std.mem.Allocator, config_json: []const u8, argv: anytype) !Runtime {
        const state = c.luaL_newstate() orelse return error.LuaInitializationFailed;
        var runtime: Runtime = .{ .state = state, .allocator = allocator };
        errdefer runtime.deinit();

        c.luaL_openlibs(state);
        if (c.luaL_loadstring(state, bootstrap.ptr) != 0 or c.lua_pcall(state, 0, 0, 0) != 0) {
            runtime.failLua("initializing misa API");
            return error.LuaInitializationFailed;
        }
        runtime.assertStack(0);
        runtime.setContext(config_json, argv);
        return runtime;
    }

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
        for (self.extensions.items) |extension| self.allocator.free(extension.path);
        self.extensions.deinit(self.allocator);
    }

    pub fn loadExtension(self: *Runtime, path: []const u8) !void {
        self.assertStack(0);
        const path_z = self.allocator.dupeZ(u8, path) catch {
            self.setError("out of memory while loading {s}", .{path});
            return error.ExtensionLoadFailed;
        };
        defer self.allocator.free(path_z);
        // Lua's filename API is C-string based; retain exactly the path it sees.
        const lua_path = std.mem.span(path_z.ptr);

        if (c.luaL_loadfile(self.state, path_z.ptr) != 0) {
            self.failLua(lua_path);
            return error.ExtensionLoadFailed;
        }
        if (c.lua_pcall(self.state, 0, 1, 0) != 0) {
            self.failLua(lua_path);
            return error.ExtensionLoadFailed;
        }
        if (c.lua_type(self.state, -1) != c.LUA_TTABLE) {
            self.setError("{s}: extension must return a table", .{lua_path});
            self.pop(1);
            self.assertStack(0);
            return error.ExtensionLoadFailed;
        }

        self.extensions.ensureUnusedCapacity(self.allocator, 1) catch {
            self.setError("out of memory while loading {s}", .{lua_path});
            self.pop(1);
            self.assertStack(0);
            return error.ExtensionLoadFailed;
        };
        const path_copy = self.allocator.dupe(u8, lua_path) catch {
            self.setError("out of memory while loading {s}", .{lua_path});
            self.pop(1);
            self.assertStack(0);
            return error.ExtensionLoadFailed;
        };
        const ref = c.luaL_ref(self.state, c.LUA_REGISTRYINDEX);
        self.extensions.appendAssumeCapacity(.{ .ref = ref, .path = path_copy });
        self.assertStack(0);
    }

    pub fn run(self: *Runtime) !void {
        try self.callPhase("setup");
        try self.callPhase("run");
    }

    pub fn lastError(self: *const Runtime) []const u8 {
        return self.error_buffer[0..self.error_len];
    }

    fn setContext(self: *Runtime, config_json: []const u8, argv: anytype) void {
        self.assertStack(0);
        c.lua_createtable(self.state, 0, 3);
        _ = c.lua_pushlstring(self.state, config_json.ptr, config_json.len);
        c.lua_setfield(self.state, -2, "config_json");
        c.lua_createtable(self.state, @intCast(argv.len), 0);
        for (argv, 0..) |arg, i| {
            _ = c.lua_pushstring(self.state, arg.ptr);
            c.lua_rawseti(self.state, -2, @intCast(i + 1));
        }
        c.lua_setfield(self.state, -2, "argv");
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_setfield(self.state, -2, "misa");
        self.context_ref = c.luaL_ref(self.state, c.LUA_REGISTRYINDEX);
        self.assertStack(0);
    }

    fn callPhase(self: *Runtime, phase: [*:0]const u8) !void {
        self.assertStack(0);
        for (self.extensions.items, 0..) |extension, i| {
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, extension.ref);
            _ = c.lua_pushstring(self.state, phase);
            _ = c.lua_rawget(self.state, -2);
            if (c.lua_type(self.state, -1) == c.LUA_TNIL) {
                self.pop(2);
                continue;
            }
            if (c.lua_type(self.state, -1) != c.LUA_TFUNCTION) {
                self.setError("extension {d} ({s}) field '{s}' must be a function", .{ i + 1, extension.path, std.mem.span(phase) });
                self.pop(2);
                self.assertStack(0);
                return error.ExtensionRunFailed;
            }

            c.lua_pushcclosure(self.state, traceback, 0);
            c.lua_insert(self.state, -2);
            const error_handler = c.lua_gettop(self.state) - 1;
            _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
            if (c.lua_pcall(self.state, 1, 0, error_handler) != 0) {
                self.setError("extension {d} ({s}) {s}: {s}", .{ i + 1, extension.path, std.mem.span(phase), self.stackError() });
                c.lua_settop(self.state, error_handler - 2);
                self.assertStack(0);
                return error.ExtensionRunFailed;
            }
            self.pop(2); // The extension table and protected-call error handler.
            self.assertStack(0);
        }
    }

    fn failLua(self: *Runtime, prefix: []const u8) void {
        self.setError("{s}: {s}", .{ prefix, self.stackError() });
        self.pop(1);
        self.assertStack(0);
    }

    fn stackError(self: *Runtime) []const u8 {
        const message = c.lua_tolstring(self.state, -1, null) orelse return "unknown Lua error";
        return std.mem.span(message);
    }

    fn setError(self: *Runtime, comptime format: []const u8, args: anytype) void {
        // Match snprintf's payload capacity from the former 1024-byte C buffer.
        var writer = std.Io.Writer.fixed(self.error_buffer[0 .. self.error_buffer.len - 1]);
        writer.print(format, args) catch {};
        self.error_len = writer.end;
    }

    fn pop(self: *Runtime, count: c_int) void {
        std.debug.assert(count >= 0 and c.lua_gettop(self.state) >= count);
        c.lua_settop(self.state, -count - 1);
    }

    fn assertStack(self: *Runtime, expected: c_int) void {
        std.debug.assert(c.lua_gettop(self.state) == expected);
    }
};

fn traceback(state: ?*c.lua_State) callconv(.c) c_int {
    const lua = state.?;
    if (c.lua_isstring(lua, 1) == 0) return 1;
    c.lua_getfield(lua, c.LUA_GLOBALSINDEX, "debug");
    if (c.lua_type(lua, -1) != c.LUA_TTABLE) {
        c.lua_settop(lua, -2);
        return 1;
    }
    c.lua_getfield(lua, -1, "traceback");
    if (c.lua_type(lua, -1) != c.LUA_TFUNCTION) {
        c.lua_settop(lua, -3);
        return 1;
    }
    c.lua_pushvalue(lua, 1);
    c.lua_pushinteger(lua, 2);
    c.lua_call(lua, 2, 1);
    return 1;
}
