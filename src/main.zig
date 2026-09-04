//! Thin CLI entry point: resolve configuration, then hand it to the Lua runtime.
const std = @import("std");
const config_module = @import("misa_config");
const lua = @import("misa_lua_runtime");
const standard_extensions = @import("misa_standard_extensions");
const terminal_module = @import("misa_terminal");
const session_module = @import("misa_session");

pub fn main(init: std.process.Init) !void {
    const allocator = init.gpa;
    var args_iterator = try init.minimal.args.iterateAllocator(allocator);
    defer args_iterator.deinit();
    var argv: std.ArrayList([:0]const u8) = .empty;
    defer argv.deinit(allocator);
    while (args_iterator.next()) |arg| try argv.append(allocator, arg);

    var extension_argv: std.ArrayList([:0]const u8) = .empty;
    defer extension_argv.deinit(allocator);
    var config_path: ?[]const u8 = null;
    var forwarding_only = false;
    var i: usize = 1;
    while (i < argv.items.len) : (i += 1) {
        const arg = argv.items[i];
        if (!forwarding_only and std.mem.eql(u8, arg, "--")) {
            forwarding_only = true;
            continue;
        }
        if (!forwarding_only and std.mem.eql(u8, arg, "--config")) {
            if (config_path != null) fatal("--config may only be specified once");
            i += 1;
            if (i >= argv.items.len) fatal("--config requires a path");
            config_path = argv.items[i];
            continue;
        }
        try extension_argv.append(allocator, arg);
    }
    if (config_path == null) config_path = init.environ_map.get("MISA_CONFIG");
    const path = config_path orelse fatal("configuration required: pass --config PATH or set MISA_CONFIG");

    const source = std.Io.Dir.cwd().readFileAlloc(init.io, path, allocator, .limited(16 * 1024 * 1024)) catch |err| {
        std.debug.print("misa: cannot read config '{s}': {s}\n", .{ path, @errorName(err) });
        std.process.exit(2);
    };
    defer allocator.free(source);

    var config = config_module.parse(allocator, source) catch |err| {
        std.debug.print("misa: invalid config '{s}': {s}\n", .{ path, @errorName(err) });
        std.process.exit(2);
    };
    defer config.deinit();

    var runtime = lua.Runtime.init(allocator, config.config_json, config.config_value, extension_argv.items) catch |err| {
        if (err == error.ConfigNestingTooDeep) {
            std.debug.print("misa: invalid config '{s}': nesting exceeds maximum depth of {d}\n", .{ path, lua.max_json_nesting_depth });
            std.process.exit(2);
        }
        std.debug.print("misa: cannot initialize LuaJIT: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };
    defer runtime.deinit();

    const extension_dir = init.environ_map.get("MISA_EXTENSION_DIR");

    for (0..config.extensions.len) |extension_index| {
        const configured = config.extensionPath(extension_index);
        const resolved = standard_extensions.resolve(allocator, configured, extension_dir) catch |err| switch (err) {
            error.UnknownStandardExtension => {
                std.debug.print(
                    "misa: invalid config '{s}': unknown standard extension ID '{s}' (expected agent, provider.fake, provider.command, or ui; use a path containing '/' or ending in .lua for a custom extension)\n",
                    .{ path, configured },
                );
                std.process.exit(2);
            },
            error.ExtensionPathContainsNul => {
                std.debug.print("misa: invalid config '{s}': extension path contains NUL\n", .{path});
                std.process.exit(2);
            },
            else => return err,
        };
        defer allocator.free(resolved);
        runtime.loadExtension(resolved) catch {
            std.debug.print("misa: {s}\n", .{runtime.lastError()});
            std.process.exit(1);
        };
    }
    runtime.setup() catch {
        std.debug.print("misa: {s}\n", .{runtime.lastError()});
        std.process.exit(1);
    };

    runSession(init, allocator, &runtime) catch |err| {
        if (err == error.LuaTransactionFailed)
            std.debug.print("misa: {s}\n", .{runtime.lastError()})
        else
            std.debug.print("misa: session failed: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };
}

/// Keep terminal cleanup in a scope that unwinds before main chooses an exit status.
fn runSession(init: std.process.Init, allocator: std.mem.Allocator, runtime: *lua.Runtime) !void {
    var terminal = try terminal_module.Terminal.init(allocator, init.io, init.environ_map);
    defer terminal.deinit();
    runtime.setTerminalInfo(.{
        .interactive = terminal.interactive,
        .columns = terminal.dimensions.columns,
        .lines = terminal.dimensions.lines,
    });
    var session: session_module.Session = .{ .allocator = allocator, .io = init.io, .runtime = runtime, .terminal = &terminal };
    defer session.deinit();
    try session.run();
}

fn fatal(message: []const u8) noreturn {
    std.debug.print("misa: {s}\n", .{message});
    std.process.exit(2);
}
