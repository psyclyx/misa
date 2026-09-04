//! Ownership boundary for the embedded system LuaJIT state.
const std = @import("std");

const Native = opaque {};
extern fn misa_lua_new() ?*Native;
extern fn misa_lua_free(runtime: *Native) void;
extern fn misa_lua_set_context(runtime: *Native, config_json: [*:0]const u8, argc: c_int, argv: [*]const [*:0]const u8) c_int;
extern fn misa_lua_load_extension(runtime: *Native, path: [*:0]const u8) c_int;
extern fn misa_lua_run(runtime: *Native) c_int;
extern fn misa_lua_error(runtime: *const Native) [*:0]const u8;

pub const Runtime = struct {
    native: *Native,
    allocator: std.mem.Allocator,

    pub fn init(allocator: std.mem.Allocator, config_json: []const u8, argv: anytype) !Runtime {
        const native = misa_lua_new() orelse return error.LuaInitializationFailed;
        errdefer misa_lua_free(native);
        const json_z = try allocator.dupeZ(u8, config_json);
        defer allocator.free(json_z);
        const pointers = try allocator.alloc([*:0]const u8, argv.len);
        defer allocator.free(pointers);
        for (argv, 0..) |arg, i| pointers[i] = arg.ptr;
        if (misa_lua_set_context(native, json_z.ptr, @intCast(argv.len), pointers.ptr) == 0)
            return error.LuaContextFailed;
        return .{ .native = native, .allocator = allocator };
    }

    pub fn deinit(self: *Runtime) void {
        misa_lua_free(self.native);
    }

    pub fn loadExtension(self: *Runtime, path: []const u8) !void {
        const path_z = try self.allocator.dupeZ(u8, path);
        defer self.allocator.free(path_z);
        if (misa_lua_load_extension(self.native, path_z.ptr) == 0) return error.ExtensionLoadFailed;
    }

    pub fn run(self: *Runtime) !void {
        if (misa_lua_run(self.native) == 0) return error.ExtensionRunFailed;
    }

    pub fn lastError(self: Runtime) []const u8 {
        return std.mem.span(misa_lua_error(self.native));
    }
};
