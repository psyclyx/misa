//! Thin CLI entry point: resolve configuration, then hand it to the Lua runtime.
const std = @import("std");
const auth = @import("misa_auth");
const provider_auth = @import("misa_provider_auth");
const lua = @import("misa_lua_runtime");
const mcp = @import("misa_mcp");
const standard_extensions = @import("misa_standard_extensions");
const state_module = @import("misa_state");
const syntax_module = @import("misa_syntax");
const terminal_module = @import("misa_terminal");
const session_module = @import("misa_session");

pub const panic = std.debug.FullPanic(struct {
    fn panic(message: []const u8, first_trace_addr: ?usize) noreturn {
        terminal_module.restoreAfterFailure();
        std.debug.defaultPanic(message, first_trace_addr);
    }
}.panic);

pub fn main(init: std.process.Init) !void {
    const allocator = init.gpa;
    var args_iterator = try init.minimal.args.iterateAllocator(allocator);
    defer args_iterator.deinit();
    var argv: std.ArrayList([:0]const u8) = .empty;
    defer argv.deinit(allocator);
    while (args_iterator.next()) |arg| try argv.append(allocator, arg);

    if (argv.items.len > 1 and (std.mem.eql(u8, argv.items[1], "login") or std.mem.eql(u8, argv.items[1], "logout") or std.mem.eql(u8, argv.items[1], "status"))) {
        if (argv.items.len != 3) fatal("usage: misa <login|logout|status> <openai|deepseek|groq|together|fireworks|xai|mistral|cerebras|deepinfra|huggingface|nvidia|moonshot|novita|siliconflow|venice|brave|tavily|openai-codex|anthropic|openrouter|kimi-coding|claude>");
        const action: auth.Action = if (std.mem.eql(u8, argv.items[1], "login")) .login else if (std.mem.eql(u8, argv.items[1], "logout")) .logout else .status;
        const result = provider_auth.commandTerminal(allocator, init.io, init.environ_map, action, argv.items[2]) catch |err| {
            if (err == error.UnknownProvider) fatal("unknown provider");
            return err;
        };
        defer result.deinit(allocator);
        if (action == .status) {
            const message = if (result.subscription_type) |subscription|
                try std.fmt.allocPrint(allocator, "{s} ({s})\n", .{ if (result.logged_in) "logged in" else "logged out", subscription })
            else
                try std.fmt.allocPrint(allocator, "{s}\n", .{if (result.logged_in) "logged in" else "logged out"});
            defer allocator.free(message);
            try std.Io.File.stdout().writeStreamingAll(init.io, message);
        }
        return;
    }

    const mcp_mode = argv.items.len > 1 and std.mem.eql(u8, argv.items[1], "mcp");
    var extension_argv: std.ArrayList([:0]const u8) = .empty;
    defer extension_argv.deinit(allocator);
    var config_path: ?[]const u8 = null;
    var forwarding_only = false;
    var i: usize = if (mcp_mode) 2 else 1;
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

    const grammar_dir = init.environ_map.get("MISA_TREE_SITTER_DIR") orelse syntax_module.default_grammar_dir;
    var runtime = lua.Runtime.init(allocator, .{ .object = .{} }, extension_argv.items) catch |err| {
        std.debug.print("misa: cannot initialize LuaJIT: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };
    defer runtime.deinit();

    const executable_path = try std.process.executablePathAlloc(init.io, allocator);
    defer allocator.free(executable_path);
    runtime.setHostInfo(executable_path, path);

    const extension_dir = init.environ_map.get("MISA_EXTENSION_DIR");

    try runtime.addModuleDirectory(extension_dir orelse standard_extensions.default_extension_dir, extension_dir != null);
    const absolute_path = std.Io.Dir.cwd().realPathFileAlloc(init.io, path, allocator) catch |err| {
        std.debug.print("misa: cannot read config '{s}': {s}\n", .{ path, @errorName(err) });
        std.process.exit(2);
    };
    defer allocator.free(absolute_path);
    try runtime.addModuleDirectory(std.fs.path.dirname(absolute_path).?, true);
    var evaluated_config = runtime.loadConfiguration(absolute_path) catch {
        std.debug.print("misa: {s}\n", .{runtime.lastError()});
        std.process.exit(1);
    };
    defer evaluated_config.deinit();
    runtime.installConfiguration() catch {
        std.debug.print("misa: {s}\n", .{runtime.lastError()});
        std.process.exit(1);
    };

    if (mcp_mode) {
        mcp.run(allocator, init.io, &runtime) catch |err| {
            std.debug.print("misa: MCP bridge failed: {s}\n", .{@errorName(err)});
            std.process.exit(1);
        };
        return;
    }

    runSession(init, allocator, &runtime, grammar_dir, evaluated_config.value) catch |err| {
        if (err == error.LuaTransactionFailed)
            std.debug.print("misa: {s}\n", .{runtime.lastError()})
        else
            std.debug.print("misa: session failed: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };
}

/// Keep terminal cleanup in a scope that unwinds before main chooses an exit status.
fn runSession(init: std.process.Init, allocator: std.mem.Allocator, runtime: *lua.Runtime, grammar_dir: []const u8, config: std.json.Value) !void {
    const terminal = try terminal_module.Driver.create(allocator, init.io, init.environ_map);
    defer terminal.destroy();
    const terminal_info = terminal.info();
    runtime.setTerminalInfo(.{
        .interactive = terminal_info.interactive,
        .images = terminal_info.images_supported,
        .columns = terminal_module.usableColumns(terminal_info.dimensions.columns),
        .lines = terminal_info.dimensions.lines,
    });
    var session: session_module.Session = .{
        .allocator = allocator,
        .io = init.io,
        .runtime = runtime,
        .dispatch_chain_limit = try session_module.dispatchChainLimit(config),
        .terminal = terminal,
        .interactive = terminal_info.interactive,
        .images_supported = terminal_info.images_supported,
        .dimensions = terminal_info.dimensions,
        .environ = init.environ_map,
        .operations = try .init(allocator, init.io),
        .timers = .init(allocator),
    };
    defer session.deinit();
    try session.operations.setGrammarDir(grammar_dir);
    try session.run();
}

fn fatal(message: []const u8) noreturn {
    std.debug.print("misa: {s}\n", .{message});
    std.process.exit(2);
}
