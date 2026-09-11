//! LuaJIT lifetime and the checked Lua/data boundary.
const std = @import("std");
const c = @cImport({
    @cInclude("lua.h");
    @cInclude("lauxlib.h");
    @cInclude("lualib.h");
});

const framework = @embedFile("misa_core_framework");
const state_updates = @embedFile("misa_core_state");
const subscriptions = @embedFile("misa_core_subscriptions");
const fennel = @embedFile("vendor/fennel.lua");
const width = @import("misa_width");
pub const max_nesting_depth: usize = 128;

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

/// A projection copied out of Lua into one short-lived arena.
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

    pub fn deinit(self: *Transaction) void {
        self.arena.deinit();
    }
};

pub const Runtime = struct {
    state: *c.lua_State,
    allocator: std.mem.Allocator,
    context_ref: c_int = c.LUA_NOREF,
    configuration_ref: c_int = c.LUA_NOREF,
    json_null_ref: c_int = c.LUA_NOREF,
    traceback_ref: c_int = c.LUA_NOREF,
    fennel_dofile_ref: c_int = c.LUA_NOREF,
    terminal_info: ?TerminalInfo = null,
    error_buffer: [2048]u8 = undefined,
    error_len: usize = 0,

    pub fn init(allocator: std.mem.Allocator, config: std.json.Value, argv: anytype) !Runtime {
        const state = c.luaL_newstate() orelse return error.LuaInitializationFailed;
        var self: Runtime = .{ .state = state, .allocator = allocator };
        errdefer self.deinit();
        c.luaL_openlibs(state);

        try self.initializeFennel();
        self.installNativeLayout();
        self.assertStack(0);
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(state, -1, "json-null");
        c.lua_remove(state, -2);
        self.json_null_ref = c.luaL_ref(state, c.LUA_REGISTRYINDEX);
        try self.setContext(config, argv);
        return self;
    }

    /// Load build-translated core Lua with correlated Fennel source lines.
    /// Retain the compiler and its require searcher for custom Fennel modules.
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
        if (c.luaL_loadbuffer(state, state_updates.ptr, state_updates.len, "@state.fnl") != 0 or
            c.lua_pcall(state, 0, 1, 2) != 0)
        {
            self.failLua("initializing state updates");
            return error.LuaInitializationFailed;
        }
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "package");
        c.lua_getfield(state, -1, "loaded");
        c.lua_pushvalue(state, 3);
        c.lua_setfield(state, -2, "misa.runtime.state");
        self.pop(3);
        if (c.luaL_loadbuffer(state, subscriptions.ptr, subscriptions.len, "@subscriptions.fnl") != 0 or
            c.lua_pcall(state, 0, 1, 2) != 0)
        {
            self.failLua("initializing subscriptions");
            return error.LuaInitializationFailed;
        }
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "package");
        c.lua_getfield(state, -1, "loaded");
        c.lua_pushvalue(state, 3);
        c.lua_setfield(state, -2, "misa.runtime.subscriptions");
        self.pop(3);
        if (c.luaL_loadbuffer(state, framework.ptr, framework.len, "@framework.fnl") != 0 or
            c.lua_pcall(state, 0, 0, 2) != 0)
        {
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

    /// Expose the terminal's own text measurement. `misa.ui.layout` measures
    /// text in Lua for wrapping and cursor mapping while the presenter measures
    /// it natively for frame validation, so both must answer identically.
    fn installNativeLayout(self: *Runtime) void {
        const state = self.state;
        c.lua_getfield(state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_createtable(state, 0, 3);
        c.lua_pushcfunction(state, nativeWidth);
        c.lua_setfield(state, -2, "width");
        c.lua_pushcfunction(state, nativeClusters);
        c.lua_setfield(state, -2, "clusters");
        c.lua_pushcfunction(state, nativeCellWidth);
        c.lua_setfield(state, -2, "cell-width");
        c.lua_setfield(state, -2, "native");
        self.pop(1);
    }

    pub fn deinit(self: *Runtime) void {
        c.lua_close(self.state);
    }

    /// Add ordinary Lua/Fennel module lookup under a caller-owned directory.
    /// Installed standard modules use translated Lua; explicit source roots
    /// and user configuration directories may also contain Fennel modules.
    pub fn addModuleDirectory(self: *Runtime, directory: []const u8, fennel_source: bool) !void {
        self.assertStack(0);
        errdefer c.lua_settop(self.state, 0);
        if (std.mem.indexOfAny(u8, directory, ";?\x00") != null) return error.InvalidModuleDirectory;
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "package");
        try self.prependModulePath(directory, "lua");
        if (fennel_source) {
            c.lua_getfield(self.state, -1, "loaded");
            c.lua_getfield(self.state, -1, "fennel");
            try self.prependModulePath(directory, "fnl");
            self.pop(2);
        }
        self.pop(1);
    }

    fn prependModulePath(self: *Runtime, directory: []const u8, suffix: []const u8) !void {
        c.lua_getfield(self.state, -1, "path");
        const previous_pointer = c.lua_tolstring(self.state, -1, null);
        const previous: []const u8 = if (previous_pointer != null) std.mem.span(previous_pointer) else "";
        const path = try std.fmt.allocPrint(self.allocator, "{s}/?.{s};{s}/?/init.{s};{s}", .{ directory, suffix, directory, suffix, previous });
        defer self.allocator.free(path);
        self.pop(1);
        _ = c.lua_pushlstring(self.state, path.ptr, path.len);
        c.lua_setfield(self.state, -2, "path");
    }

    /// Evaluate a configuration value without translating its callback tables
    /// through JSON. Only the native configuration data leaves this VM.
    pub fn loadConfiguration(self: *Runtime, path: []const u8) !OwnedValue {
        self.assertStack(0);
        errdefer c.lua_settop(self.state, 0);
        if (self.configuration_ref != c.LUA_NOREF) return error.ConfigurationAlreadyLoaded;
        try self.loadTable(path);
        c.lua_pushnil(self.state);
        while (c.lua_next(self.state, -2) != 0) {
            const key = if (c.lua_type(self.state, -2) == c.LUA_TSTRING) std.mem.span(c.lua_tolstring(self.state, -2, null)) else "";
            if (!std.mem.eql(u8, key, "config") and !std.mem.eql(u8, key, "definitions")) {
                self.setError("{s}: configuration must return only config and definitions", .{path});
                return error.ConfigurationLoadFailed;
            }
            self.pop(1);
        }
        c.lua_getfield(self.state, -1, "definitions");
        if (c.lua_type(self.state, -1) != c.LUA_TTABLE) {
            self.setError("{s}: configuration must contain a definitions table", .{path});
            return error.ConfigurationLoadFailed;
        }
        self.pop(1);
        c.lua_getfield(self.state, -1, "config");
        if (c.lua_type(self.state, -1) == c.LUA_TNIL) {
            self.pop(1);
            c.lua_createtable(self.state, 0, 0);
        }
        var result: OwnedValue = .{ .arena = .init(self.allocator), .value = undefined };
        errdefer result.deinit();
        var active: std.ArrayList(?*const anyopaque) = .empty;
        defer active.deinit(self.allocator);
        result.value = self.readLuaValue(result.arena.allocator(), -1, 0, &active) catch |err| {
            if (err == error.MaximumNestingDepth)
                self.setError("{s}: config nesting exceeds maximum depth of {d}", .{ path, max_nesting_depth })
            else
                self.setError("{s}: config must contain only JSON values: {s}", .{ path, @errorName(err) });
            return error.ConfigurationLoadFailed;
        };
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
        c.lua_pushvalue(self.state, -2);
        c.lua_setfield(self.state, -2, "config");
        self.pop(2);
        self.configuration_ref = c.luaL_ref(self.state, c.LUA_REGISTRYINDEX);
        return result;
    }

    pub fn installConfiguration(self: *Runtime) !void {
        self.assertStack(0);
        if (self.configuration_ref == c.LUA_NOREF) return error.ConfigurationNotLoaded;
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_install");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const error_handler = c.lua_gettop(self.state) - 1;
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.configuration_ref);
        c.lua_getfield(self.state, -1, "definitions");
        c.lua_remove(self.state, -2);
        _ = c.lua_rawgeti(self.state, c.LUA_REGISTRYINDEX, self.context_ref);
        if (c.lua_pcall(self.state, 2, 0, error_handler) != 0) {
            self.failLua("installing configuration");
            c.lua_settop(self.state, 0);
            return error.ConfigurationInstallFailed;
        }
        self.pop(1);
        c.luaL_unref(self.state, c.LUA_REGISTRYINDEX, self.configuration_ref);
        self.configuration_ref = c.LUA_NOREF;
    }

    fn loadTable(self: *Runtime, path: []const u8) !void {
        self.assertStack(0);
        if (std.mem.indexOfScalar(u8, path, 0) != null) return error.ConfigurationLoadFailed;
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
            return error.ConfigurationLoadFailed;
        }
        c.lua_remove(self.state, 1);
        if (c.lua_type(self.state, -1) != c.LUA_TTABLE) {
            self.setError("{s}: file must return a table", .{path});
            self.pop(1);
            return error.ConfigurationLoadFailed;
        }
    }

    pub fn setTerminalInfo(self: *Runtime, info: TerminalInfo) void {
        self.assertStack(0);
        std.debug.assert(info.columns > 0 and info.lines > 0);
        self.terminal_info = info;
    }

    /// Dispatch one event and copy its effects out of Lua.
    pub fn dispatch(self: *Runtime, event_json: []const u8, clock: ClockInfo) !Transaction {
        self.assertStack(0);
        errdefer self.rollbackTransaction();
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
        if (c.lua_pcall(self.state, 3, 1, error_handler) != 0) {
            self.setError("event dispatch: {s}", .{self.stackError()});
            c.lua_settop(self.state, 0);
            return error.EventDispatchFailed;
        }

        var transaction: Transaction = .{
            .arena = std.heap.ArenaAllocator.init(self.allocator),
            .effects = undefined,
        };
        errdefer transaction.deinit();
        var active: std.ArrayList(?*const anyopaque) = .empty;
        defer active.deinit(self.allocator);
        const arena = transaction.arena.allocator();
        const effects_value = self.readLuaValue(arena, -1, 0, &active) catch |err| {
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
        c.lua_settop(self.state, 0);
        return transaction;
    }

    /// Project committed state without running handlers or executing effects.
    /// The caller validates the semantic view before accepting its cache.
    pub fn project(self: *Runtime, clock: ClockInfo) !OwnedValue {
        self.assertStack(0);
        errdefer self.rollbackProjection();
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_project");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        const error_handler = c.lua_gettop(self.state) - 1;
        self.pushTerminalInfo(self.terminal_info orelse return error.TerminalInfoMissing);
        self.pushClockInfo(clock);
        if (c.lua_pcall(self.state, 2, 1, error_handler) != 0) {
            self.setError("view projection: {s}", .{self.stackError()});
            c.lua_settop(self.state, 0);
            return error.ViewProjectionFailed;
        }
        var result: OwnedValue = .{ .arena = .init(self.allocator), .value = undefined };
        errdefer result.deinit();
        var active: std.ArrayList(?*const anyopaque) = .empty;
        defer active.deinit(self.allocator);
        result.value = self.readLuaValue(result.arena.allocator(), -1, 0, &active) catch |err| {
            self.setError("invalid view: {s}", .{@errorName(err)});
            c.lua_settop(self.state, 0);
            return error.ViewProjectionFailed;
        };
        c.lua_settop(self.state, 0);
        return result;
    }

    pub fn commitProjection(self: *Runtime) !void {
        self.assertStack(0);
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_commit_projection");
        c.lua_remove(self.state, -2);
        self.pushTraceback();
        c.lua_insert(self.state, -2);
        if (c.lua_pcall(self.state, 0, 0, 1) != 0) {
            self.failLua("committing projection");
            c.lua_settop(self.state, 0);
            return error.ViewProjectionFailed;
        }
        self.pop(1);
    }

    pub fn rollbackProjection(self: *Runtime) void {
        c.lua_settop(self.state, 0);
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_rollback_projection");
        c.lua_remove(self.state, -2);
        _ = c.lua_pcall(self.state, 0, 0, 0);
        c.lua_settop(self.state, 0);
    }

    pub fn reportProjectionError(self: *Runtime, failure: anyerror) void {
        self.setError("invalid view: {s}", .{@errorName(failure)});
    }

    pub fn mcpTools(self: *Runtime) !OwnedValue {
        return self.callMcp("_mcp_tools", null, null, null, null);
    }

    pub fn mcpToolEffect(self: *Runtime, name: []const u8, arguments: std.json.Value, id: []const u8, clock: ClockInfo) !OwnedValue {
        return self.callMcp("_mcp_tool_effect", name, arguments, id, clock);
    }

    fn callMcp(self: *Runtime, function: []const u8, name: ?[]const u8, arguments: ?std.json.Value, id: ?[]const u8, clock: ?ClockInfo) !OwnedValue {
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
            self.pushTerminalInfo(self.terminal_info orelse return error.TerminalInfoMissing);
            self.pushClockInfo(clock.?);
            break :blk 5;
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

    pub fn rollbackTransaction(self: *Runtime) void {
        c.lua_settop(self.state, 0);
        c.lua_getfield(self.state, c.LUA_GLOBALSINDEX, "misa");
        c.lua_getfield(self.state, -1, "_rollback");
        c.lua_remove(self.state, -2);
        _ = c.lua_pcall(self.state, 0, 0, 0);
        c.lua_settop(self.state, 0);
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
                c.lua_getfield(self.state, -1, "json-null");
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

/// Terminal cells for a whole string, or nil when it is not valid UTF-8.
fn nativeWidth(state: ?*c.lua_State) callconv(.c) c_int {
    const lua_state = state.?;
    var length: usize = 0;
    const text = c.lua_tolstring(lua_state, 1, &length) orelse {
        c.lua_pushnil(lua_state);
        return 1;
    };
    const cells = width.textWidth(text[0..length]) catch {
        c.lua_pushnil(lua_state);
        return 1;
    };
    c.lua_pushnumber(lua_state, @floatFromInt(cells));
    return 1;
}

/// Terminal cells for one codepoint.
fn nativeCellWidth(state: ?*c.lua_State) callconv(.c) c_int {
    const lua_state = state.?;
    const codepoint = c.lua_tonumber(lua_state, 1);
    if (!(codepoint >= 0 and codepoint <= 0x10ffff)) {
        c.lua_pushnil(lua_state);
        return 1;
    }
    c.lua_pushnumber(lua_state, @floatFromInt(width.displayWidth(@intFromFloat(codepoint))));
    return 1;
}

/// Grapheme clusters as a flat table: end byte, cells, end byte, cells, ...
/// One call per string keeps the Lua side iterating an array instead of asking
/// per codepoint.
fn nativeClusters(state: ?*c.lua_State) callconv(.c) c_int {
    const lua_state = state.?;
    var length: usize = 0;
    const text = c.lua_tolstring(lua_state, 1, &length) orelse {
        c.lua_pushnil(lua_state);
        return 1;
    };
    const slice = text[0..length];
    if (std.unicode.utf8ValidateSlice(slice)) {} else {
        c.lua_pushnil(lua_state);
        return 1;
    }
    var count: usize = 0;
    var at: usize = 0;
    while (at < slice.len) {
        const cluster = width.nextCluster(slice, at) catch {
            c.lua_pushnil(lua_state);
            return 1;
        };
        if (cluster.end == at) break;
        count += 1;
        at = cluster.end;
    }
    c.lua_createtable(lua_state, @intCast(count * 2), 0);
    at = 0;
    var index: c_int = 1;
    while (at < slice.len) {
        const cluster = width.nextCluster(slice, at) catch unreachable;
        if (cluster.end == at) break;
        c.lua_pushnumber(lua_state, @floatFromInt(cluster.end));
        c.lua_rawseti(lua_state, -2, index);
        c.lua_pushnumber(lua_state, @floatFromInt(cluster.width));
        c.lua_rawseti(lua_state, -2, index + 1);
        index += 2;
        at = cluster.end;
    }
    return 1;
}

test "Fennel configuration imports ordinary modules and retains callback values" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const io = std.testing.io;
    const allocator = std.testing.allocator;
    const directory = try temporary.dir.realPathFileAlloc(io, ".", allocator);
    defer allocator.free(directory);
    const path = try std.fs.path.join(allocator, &.{ directory, "config.fnl" });
    defer allocator.free(path);
    try temporary.dir.writeFile(io, .{ .sub_path = "profile_helper.fnl", .data =
        \\(fn [label]
        \\  {:config {:label label}
        \\   :definitions {:events {:start {:event :app/start
        \\                                  :handler (fn [_ _ cofx] {:patch {:label cofx.config.label}})}}
        \\                 :views {:main (fn [db] {:lines [{:spans [{:text db.label}]}]})}}})
    });
    try temporary.dir.writeFile(io, .{ .sub_path = "config.fnl", .data = "(local make (require :profile_helper))\n(make :retained)\n" });
    var runtime = try Runtime.init(allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    try runtime.addModuleDirectory(directory, true);
    var configuration = try runtime.loadConfiguration(path);
    defer configuration.deinit();
    try std.testing.expectEqualStrings("retained", configuration.value.object.get("label").?.string);
    try runtime.installConfiguration();
    runtime.setTerminalInfo(.{ .interactive = true, .columns = 80, .lines = 24, .images = false });
    const clock: ClockInfo = .{ .wall_ms = 0, .monotonic_ms = 0 };
    var transaction = try runtime.dispatch("{\"type\":\"app/start\"}", clock);
    defer transaction.deinit();
    try runtime.commitTransaction();
    var projection = try runtime.project(clock);
    defer projection.deinit();
    try std.testing.expectEqualStrings("retained", projection.value.object.get("lines").?.array.items[0].object.get("spans").?.array.items[0].object.get("text").?.string);
    try runtime.commitProjection();
}

test "configuration data rejects callbacks without retaining a failed application" {
    var temporary = std.testing.tmpDir(.{});
    defer temporary.cleanup();
    const io = std.testing.io;
    const allocator = std.testing.allocator;
    const directory = try temporary.dir.realPathFileAlloc(io, ".", allocator);
    defer allocator.free(directory);
    const path = try std.fs.path.join(allocator, &.{ directory, "config.lua" });
    defer allocator.free(path);
    try temporary.dir.writeFile(io, .{ .sub_path = "config.lua", .data = "return {config={bad=function() end},definitions={}}" });
    var runtime = try Runtime.init(allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    try std.testing.expectError(error.ConfigurationLoadFailed, runtime.loadConfiguration(path));
    runtime.assertStack(0);
    try temporary.dir.writeFile(io, .{ .sub_path = "config.lua", .data = "return {definitions={},modules={}}" });
    try std.testing.expectError(error.ConfigurationLoadFailed, runtime.loadConfiguration(path));
    try temporary.dir.writeFile(io, .{ .sub_path = "config.lua", .data = "return {definitions={}}" });
    var configuration = try runtime.loadConfiguration(path);
    defer configuration.deinit();
    try runtime.installConfiguration();
}

test "model dispatches settle independently of projection" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    const source =
        \\local projections = 0
        \\misa._install({events = {
        \\  increment = {event="increment", handler=function(db)
        \\    assert(projections == 0, "dispatch eagerly projected an intermediate state")
        \\    return {patch={count=(db.count or 0)+1}} end}},
        \\  views = {main=function(db)
        \\    projections = projections + 1
        \\    return {lines={{spans={{text=tostring(db.count)}}}}} end}
        \\}, {argv={}, config={}})
    ;
    try std.testing.expectEqual(@as(c_int, 0), c.luaL_loadbuffer(runtime.state, source.ptr, source.len, "@independent-projection.lua"));
    try std.testing.expectEqual(@as(c_int, 0), c.lua_pcall(runtime.state, 0, 0, 0));
    runtime.setTerminalInfo(.{ .interactive = true, .columns = 80, .lines = 24, .images = false });
    const clock: ClockInfo = .{ .wall_ms = 0, .monotonic_ms = 0 };
    for (0..2) |_| {
        var transaction = try runtime.dispatch("{\"type\":\"increment\"}", clock);
        defer transaction.deinit();
        try runtime.commitTransaction();
    }
    var projection = try runtime.project(clock);
    defer projection.deinit();
    try std.testing.expectEqualStrings("2", projection.value.object.get("lines").?.array.items[0].object.get("spans").?.array.items[0].object.get("text").?.string);
    try runtime.commitProjection();
}

test "projection decoding rejection preserves committed model and prior projection cache" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    const source =
        \\local original
        \\misa._install({subscriptions = {
        \\  probe = {inputs=function() return {{"db/path", "value"}} end,
        \\    compute=function(inputs) return {value=inputs[1]} end}},
        \\  events = {set = {event="set", handler=function(db, event)
        \\    if event.previous then assert(db.value == event.previous, "projection rejection rolled back model") end
        \\    return {patch={value=event.value, invalid=event.invalid or false}} end}},
        \\  views = {main=function(db)
        \\    local current = misa.sub(db, {"probe"})
        \\    if db.value == 1 then
        \\      if original then assert(current == original, "rejected native view replaced cache") end
        \\      original = current
        \\    end
        \\    return {lines={}, invalid=db.invalid and function() end or nil}
        \\  end}
        \\}, {argv={}, config={}})
    ;
    try std.testing.expectEqual(@as(c_int, 0), c.luaL_loadbuffer(runtime.state, source.ptr, source.len, "@subscription-rejection.lua"));
    try std.testing.expectEqual(@as(c_int, 0), c.lua_pcall(runtime.state, 0, 0, 0));
    runtime.setTerminalInfo(.{ .interactive = true, .columns = 80, .lines = 24, .images = false });
    const clock: ClockInfo = .{ .wall_ms = 0, .monotonic_ms = 0 };
    var first = try runtime.dispatch("{\"type\":\"set\",\"value\":1}", clock);
    defer first.deinit();
    try runtime.commitTransaction();
    var initial_view = try runtime.project(clock);
    defer initial_view.deinit();
    try runtime.commitProjection();
    var changed = try runtime.dispatch("{\"type\":\"set\",\"value\":2,\"invalid\":true}", clock);
    defer changed.deinit();
    try runtime.commitTransaction();
    try std.testing.expectError(error.ViewProjectionFailed, runtime.project(clock));
    var recovered = try runtime.dispatch("{\"type\":\"set\",\"value\":1,\"previous\":2}", clock);
    defer recovered.deinit();
    try runtime.commitTransaction();
    var recovered_view = try runtime.project(clock);
    defer recovered_view.deinit();
    try runtime.commitProjection();
}

test "translated core retains source locations and leaves bootstrap stack empty" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    try std.testing.expectEqual(@as(c_int, 0), c.lua_gettop(runtime.state));
    const source = "misa.patch(nil, {})";
    runtime.pushTraceback();
    try std.testing.expectEqual(@as(c_int, 0), c.luaL_loadbuffer(runtime.state, source.ptr, source.len, "@core-diagnostics.lua"));
    try std.testing.expect(c.lua_pcall(runtime.state, 0, 0, 1) != 0);
    runtime.failLua("core diagnostic probe");
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "state.fnl:") != null);
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "patch state must be a table") != null);
}

test "bundled Fennel loads applications after framework restrictions" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    var configuration = try runtime.loadConfiguration("src/lua_runtime/fixtures/extension.fnl");
    defer configuration.deinit();
    try runtime.installConfiguration();
}

test "Fennel loading reports source location and restores stack after failure" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    try std.testing.expectError(error.ConfigurationLoadFailed, runtime.loadConfiguration("src/lua_runtime/fixtures/failure.fnl"));
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "failure.fnl:2") != null);
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "Fennel fixture failed") != null);
    try std.testing.expectError(error.ConfigurationLoadFailed, runtime.loadConfiguration("src/lua_runtime/fixtures/missing.fnl"));
    try std.testing.expect(std.mem.indexOf(u8, runtime.lastError(), "missing.fnl") != null);
    var configuration = try runtime.loadConfiguration("src/lua_runtime/fixtures/extension.fnl");
    defer configuration.deinit();
    try runtime.installConfiguration();
}

test "application data installs once and preserves explicit replacements" {
    var runtime = try Runtime.init(std.testing.allocator, .null, &[_][]const u8{});
    defer runtime.deinit();
    const source =
        \\local first = {config={label="stock"},definitions={services={deferred=7,removed=true}}}
        \\local application = misa.snapshot(first)
        \\application.definitions.services.deferred = 8
        \\application.definitions.services.removed = nil
        \\application.config.label = "custom"
        \\local additions = {
        \\  services={["fixture.value"]=42},
        \\  actions={fixture={label="Fixture", event={type="fixture"}}},
        \\  effects={["fixture/translate"]=function() return {type="register/obsolete"} end},
        \\  events={fixture={event="fixture",handler=function() return {fx={{type="fixture/translate"}}} end}},
        \\  requirements={fixture={"deferred", "fixture.value"}}
        \\}
        \\for kind, entries in pairs(additions) do
        \\  application.definitions[kind] = application.definitions[kind] or {}
        \\  for id, value in pairs(entries) do application.definitions[kind][id] = value end
        \\end
        \\assert(misa.deferred == nil and first.definitions.services.deferred == 7)
        \\misa._install(application.definitions, {argv={},config=application.config})
        \\assert(misa.deferred == 8 and misa.fixture.value == 42 and misa.removed == nil)
        \\assert(misa.configuration().label == "custom" and first.config.label == "stock")
        \\assert(misa.actions.lookup("fixture").binding.action == "fixture")
        \\assert(not pcall(misa._install, application.definitions, {}))
        \\local ok, err = pcall(misa._dispatch, {type="fixture"}, {}, {monotonic_ms=0,wall_ms=0})
        \\assert(not ok and tostring(err):match("registration declarations cannot run as effects"), tostring(err))
    ;
    try std.testing.expectEqual(@as(c_int, 0), c.luaL_loadbuffer(runtime.state, source.ptr, source.len, "@application-definitions-test.lua"));
    if (c.lua_pcall(runtime.state, 0, 0, 0) != 0) {
        std.debug.print("application definitions test: {s}\n", .{runtime.stackError()});
        return error.ApplicationDefinitionsTestFailed;
    }
    runtime.assertStack(0);
}
