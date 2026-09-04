//! Thin CLI entry point: resolve configuration, then hand it to the Lua runtime.
const std = @import("std");
const auth = @import("misa_auth");
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

    if (argv.items.len > 1 and (std.mem.eql(u8, argv.items[1], "login") or std.mem.eql(u8, argv.items[1], "logout") or std.mem.eql(u8, argv.items[1], "status"))) {
        if (argv.items.len != 3) fatal("usage: misa <login|logout|status> <openai|openai-codex|anthropic|openrouter|kimi-coding|claude>");
        const command = argv.items[1];
        if (std.mem.eql(u8, command, "login"))
            try login(init, allocator, argv.items[2])
        else if (std.mem.eql(u8, command, "logout"))
            try logout(init, allocator, argv.items[2])
        else
            try status(init, allocator, argv.items[2]);
        return;
    }

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
    const path = config_path orelse standard_extensions.default_config_path;

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

    var runtime = lua.Runtime.init(allocator, config.config_value, extension_argv.items) catch |err| {
        if (err == error.MaximumNestingDepth) {
            std.debug.print("misa: invalid config '{s}': nesting exceeds maximum depth of {d}\n", .{ path, lua.max_nesting_depth });
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
                    "misa: invalid config '{s}': unknown standard extension ID '{s}' (use a documented standard ID, or a path containing '/' or ending in .lua for a custom extension)\n",
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
    var auth_store: ?auth.Store = auth.Store.init(allocator, init.io, init.environ_map) catch null;
    defer if (auth_store) |*store| store.deinit();
    var session: session_module.Session = .{
        .allocator = allocator,
        .io = init.io,
        .runtime = runtime,
        .terminal = &terminal,
        .auth_store = if (auth_store) |*store| store else null,
    };
    defer session.deinit();
    try session.run();
}

fn login(init: std.process.Init, allocator: std.mem.Allocator, provider: []const u8) !void {
    if (std.mem.eql(u8, provider, "claude")) return claudeAuth(init, "login");
    var store = try auth.Store.init(allocator, init.io, init.environ_map);
    defer store.deinit();
    if (std.mem.eql(u8, provider, "openai-codex") or std.mem.eql(u8, provider, "kimi-coding")) {
        const credential = if (std.mem.eql(u8, provider, "openai-codex"))
            try auth.oauth.loginOpenAI(allocator, init.io)
        else
            try auth.oauth.loginKimi(allocator, init.io);
        defer credential.deinit(allocator);
        try store.putOAuth(provider, credential.access, credential.refresh, credential.expires, credential.account_id);
    } else if (std.mem.eql(u8, provider, "openrouter")) {
        const authorization = try auth.oauth.startOpenRouter(allocator, init.io);
        defer authorization.deinit(allocator);
        std.debug.print("Open this URL:\n{s}\n", .{authorization.url});
        const input = try auth.readSecret(allocator, init.io, "Paste the authorization code or redirect URL: ");
        defer allocator.free(input);
        const credential = try auth.oauth.finishOpenRouter(allocator, init.io, authorization.verifier, input);
        defer credential.deinit(allocator);
        try store.putOAuth(provider, credential.access, credential.refresh, credential.expires, credential.account_id);
    } else if (std.mem.eql(u8, provider, "openai") or std.mem.eql(u8, provider, "anthropic")) {
        const secret = try auth.readSecret(allocator, init.io, "API key: ");
        defer allocator.free(secret);
        try store.put(provider, secret);
    } else fatal("unknown login provider");
    std.debug.print("misa: saved {s} credential to {s}\n", .{ provider, store.path });
}

fn logout(init: std.process.Init, allocator: std.mem.Allocator, provider: []const u8) !void {
    if (std.mem.eql(u8, provider, "claude")) return claudeAuth(init, "logout");
    if (!managedProvider(provider)) fatal("unknown provider");
    var store = try auth.Store.init(allocator, init.io, init.environ_map);
    defer store.deinit();
    _ = try store.remove(provider);
}

fn status(init: std.process.Init, allocator: std.mem.Allocator, provider: []const u8) !void {
    if (std.mem.eql(u8, provider, "claude")) return claudeAuth(init, "status");
    if (!managedProvider(provider)) fatal("unknown provider");
    var store = try auth.Store.init(allocator, init.io, init.environ_map);
    defer store.deinit();
    try std.Io.File.stdout().writeStreamingAll(init.io, if (store.contains(provider)) "logged in\n" else "logged out\n");
}

fn managedProvider(provider: []const u8) bool {
    return std.mem.eql(u8, provider, "openai") or std.mem.eql(u8, provider, "openai-codex") or
        std.mem.eql(u8, provider, "anthropic") or std.mem.eql(u8, provider, "openrouter") or
        std.mem.eql(u8, provider, "kimi-coding");
}

fn claudeAuth(init: std.process.Init, command: []const u8) !void {
    var child = try std.process.spawn(init.io, .{ .argv = &.{ "claude", "auth", command } });
    defer child.kill(init.io);
    const term = try child.wait(init.io);
    if (term != .exited or term.exited != 0) return error.ClaudeAuthFailed;
}

fn fatal(message: []const u8) noreturn {
    std.debug.print("misa: {s}\n", .{message});
    std.process.exit(2);
}
