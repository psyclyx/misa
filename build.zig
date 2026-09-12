const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const wakeup = b.createModule(.{
        .root_source_file = b.path("src/wakeup/root.zig"),
        .target = target,
        .optimize = optimize,
    });

    const auth = b.createModule(.{
        .root_source_file = b.path("src/auth/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const syntax_options = b.addOptions();
    syntax_options.addOption([]const u8, "default_grammar_dir", b.option([]const u8, "tree-sitter-dir", "Directory containing tree-sitter <language>.so grammars") orelse "");
    const syntax = b.createModule(.{
        .root_source_file = b.path("src/syntax/root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    syntax.addOptions("misa_syntax_options", syntax_options);
    syntax.linkSystemLibrary("tree-sitter", .{ .use_pkg_config = .force });

    const image = b.createModule(.{
        .root_source_file = b.path("src/image/root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    image.linkSystemLibrary("libpng", .{ .use_pkg_config = .force });
    image.linkSystemLibrary("libturbojpeg", .{ .use_pkg_config = .force });

    const standard_extension_options = b.addOptions();
    standard_extension_options.addOption([]const u8, "default_extension_dir", b.getInstallPath(.{ .custom = "share/misa" }, "extensions"));
    standard_extension_options.addOption([]const u8, "default_config_path", b.getInstallPath(.{ .custom = "share/misa" }, "default.fnl"));
    const standard_extensions = b.createModule(.{
        .root_source_file = b.path("src/standard_extensions/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    standard_extensions.addOptions("misa_build_options", standard_extension_options);
    // The catalog is the single source for discovery and translation, so a file
    // that is not listed would silently never load. Capture the tree itself and
    // let the catalog's own test compare against it.
    const extension_manifest = b.addSystemCommand(&.{ "sh", "-c", "cd extensions && find . -name '*.fnl' | sed 's|^\\./||' | LC_ALL=C sort" });
    // The tree is the input, so adding a bundled extension invalidates the
    // captured manifest instead of leaving it a file behind.
    extension_manifest.addDirectoryArg(b.path("extensions"));
    standard_extensions.addAnonymousImport("misa_extension_manifest", .{ .root_source_file = extension_manifest.captureStdOut(.{}) });
    // The terminal presenter and the Lua layout both measure text, so the tables
    // and clustering live in one module each of them imports.
    const width = b.createModule(.{
        .root_source_file = b.path("src/width/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const lua_runtime = b.createModule(.{
        .root_source_file = b.path("src/lua_runtime/root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    lua_runtime.linkSystemLibrary("luajit", .{ .use_pkg_config = .force });
    lua_runtime.addImport("misa_width", width);
    const terminal = b.createModule(.{
        .root_source_file = b.path("src/terminal/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    terminal.addImport("misa_wakeup", wakeup);
    terminal.addImport("misa_width", width);
    const file_effect = b.createModule(.{
        .root_source_file = b.path("src/capability/file.zig"),
        .target = target,
        .optimize = optimize,
    });
    const process_effect = b.createModule(.{
        .root_source_file = b.path("src/capability/process.zig"),
        .target = target,
        .optimize = optimize,
    });
    const provider_process = b.createModule(.{
        .root_source_file = b.path("src/provider/process/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    provider_process.addImport("misa_process", process_effect);
    const fixture_process = b.createModule(.{
        .root_source_file = b.path("tests/fixtures/provider_process/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    fixture_process.addImport("misa_process", process_effect);
    const state = b.createModule(.{
        .root_source_file = b.path("src/state/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const conversation = b.createModule(.{
        .root_source_file = b.path("src/conversation/root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    conversation.linkSystemLibrary("sqlite3", .{ .use_pkg_config = .force });
    const http = b.createModule(.{
        .root_source_file = b.path("src/http/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    http.addImport("misa_auth", auth);
    const fixture_http = b.createModule(.{
        .root_source_file = b.path("tests/fixtures/http/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    fixture_http.addImport("misa_http_contract", http);
    const fixture_auth = b.createModule(.{
        .root_source_file = b.path("tests/fixtures/auth/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    fixture_auth.addImport("misa_auth", auth);
    const session = b.createModule(.{
        .root_source_file = b.path("src/session/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    session.addImport("misa_auth", auth);
    session.addImport("misa_provider_auth", auth);
    session.addImport("misa_provider_process", provider_process);
    session.addImport("misa_http", http);
    session.addImport("misa_wakeup", wakeup);
    session.addImport("misa_image", image);
    session.addImport("misa_syntax", syntax);
    session.addImport("misa_file", file_effect);
    session.addImport("misa_lua_runtime", lua_runtime);
    session.addImport("misa_process", process_effect);
    session.addImport("misa_state", state);
    session.addImport("misa_conversation", conversation);
    session.addImport("misa_terminal", terminal);
    const mcp = b.createModule(.{
        .root_source_file = b.path("src/mcp/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    mcp.addImport("misa_file", file_effect);
    mcp.addImport("misa_lua_runtime", lua_runtime);
    mcp.addImport("misa_process", process_effect);

    const main_module = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    main_module.addImport("misa_auth", auth);
    main_module.addImport("misa_provider_auth", auth);
    main_module.addImport("misa_lua_runtime", lua_runtime);
    main_module.addImport("misa_mcp", mcp);
    main_module.addImport("misa_standard_extensions", standard_extensions);
    main_module.addImport("misa_state", state);
    main_module.addImport("misa_syntax", syntax);
    main_module.addImport("misa_terminal", terminal);
    main_module.addImport("misa_session", session);

    const exe = b.addExecutable(.{ .name = "misa", .root_module = main_module });
    // Fixture applications share session behavior, but acquire provider I/O
    // from fixture dependencies. There is no runtime switch to enable live I/O.
    const fixture_session = b.createModule(.{
        .root_source_file = b.path("src/session/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    copyImports(fixture_session, session);
    fixture_session.addImport("misa_provider_auth", fixture_auth);
    fixture_session.addImport("misa_provider_process", fixture_process);
    fixture_session.addImport("misa_http", fixture_http);
    const fixture_main = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    copyImports(fixture_main, main_module);
    fixture_main.addImport("misa_session", fixture_session);
    fixture_main.addImport("misa_provider_auth", fixture_auth);
    const fixture_exe = b.addExecutable(.{ .name = "misa-fixture", .root_module = fixture_main });
    const install_fixture = b.addInstallArtifact(fixture_exe, .{});
    install_fixture.step.dependOn(b.getInstallStep());
    b.step("fixture-app", "Build the application with fixture provider dependencies").dependOn(&install_fixture.step);
    b.installArtifact(exe);
    b.installDirectory(.{
        .source_dir = b.path("extensions"),
        .install_dir = .{ .custom = "share/misa" },
        .install_subdir = "extensions",
    });
    // Catalog installs use translated Lua; explicit source overrides keep Fennel.
    // A single host process compiles all sources, with declared cache inputs.
    const translate = b.addSystemCommand(&.{"luajit"});
    translate.addFileArg(b.path("tools/compile-fennel.lua"));
    translate.addFileInput(b.path("src/lua_runtime/vendor/fennel.lua"));
    for ([_][]const u8{ "state", "subscriptions", "framework" }) |name| {
        translate.addFileArg(b.path(b.fmt("src/lua_runtime/{s}.fnl", .{name})));
        const output = translate.addOutputFileArg(b.fmt("runtime/{s}.lua", .{name}));
        lua_runtime.addAnonymousImport(b.fmt("misa_core_{s}", .{name}), .{ .root_source_file = output });
    }
    const catalog = @import("src/standard_extensions/root.zig");
    for (catalog.entries) |entry| {
        const source = entry.path;
        const generated = b.fmt("{s}.lua", .{source[0 .. source.len - 4]});
        translate.addFileArg(b.path(b.fmt("extensions/{s}", .{source})));
        const output = translate.addOutputFileArg(generated);
        const install = b.addInstallFileWithDir(output, .{ .custom = "share/misa/extensions" }, generated);
        b.getInstallStep().dependOn(&install.step);
    }
    b.installFile("config/default.fnl", "share/misa/default.fnl");

    const run = b.addRunArtifact(exe);
    run.step.dependOn(b.getInstallStep());
    // The default inferred mode captures stdio, so misa sees pipes instead of
    // the caller's terminal and immediately receives EOF. A TUI run must own
    // the real terminal for its lifetime.
    run.stdio = .inherit;
    if (b.args) |args| run.addArgs(args);
    b.step("run", "Run misa").dependOn(&run.step);

    const auth_unit = b.addTest(.{ .root_module = auth });
    const oauth_test_module = b.createModule(.{
        .root_source_file = b.path("src/auth/oauth.zig"),
        .target = target,
        .optimize = optimize,
    });
    const oauth_unit = b.addTest(.{ .root_module = oauth_test_module });
    const operation_test_module = b.createModule(.{
        .root_source_file = b.path("src/session/operation.zig"),
        .target = target,
        .optimize = optimize,
    });
    operation_test_module.addImport("misa_auth", auth);
    operation_test_module.addImport("misa_provider_auth", fixture_auth);
    operation_test_module.addImport("misa_provider_process", fixture_process);
    operation_test_module.addImport("misa_http", fixture_http);
    operation_test_module.addImport("misa_wakeup", wakeup);
    operation_test_module.addImport("misa_image", image);
    operation_test_module.addImport("misa_syntax", syntax);
    operation_test_module.addImport("misa_file", file_effect);
    operation_test_module.addImport("misa_process", process_effect);
    operation_test_module.addImport("misa_state", state);
    operation_test_module.addImport("misa_conversation", conversation);
    // The effect contract validates `view/commit` lines and decodes contained
    // input, so this test module needs the terminal and protected-input owners.
    operation_test_module.addImport("misa_terminal", terminal);
    const operation_unit = b.addTest(.{ .root_module = operation_test_module });
    const runtime_unit = b.addTest(.{ .root_module = lua_runtime });
    const resolver_unit = b.addTest(.{ .root_module = standard_extensions });
    const process_unit = b.addTest(.{ .root_module = process_effect });
    const file_unit = b.addTest(.{ .root_module = file_effect });
    const state_unit = b.addTest(.{ .root_module = state });
    const conversation_unit = b.addTest(.{ .root_module = conversation });
    const image_unit = b.addTest(.{ .root_module = image });
    const syntax_unit = b.addTest(.{ .root_module = syntax });
    const terminal_unit = b.addTest(.{ .root_module = terminal });
    const width_unit = b.addTest(.{ .root_module = width });
    const session_unit = b.addTest(.{ .root_module = fixture_session });
    const test_step = b.step("test", "Run unit and integration tests");
    test_step.dependOn(&b.addRunArtifact(auth_unit).step);
    test_step.dependOn(&b.addRunArtifact(oauth_unit).step);
    test_step.dependOn(&b.addRunArtifact(operation_unit).step);
    test_step.dependOn(&b.addRunArtifact(runtime_unit).step);
    test_step.dependOn(&b.addRunArtifact(resolver_unit).step);
    test_step.dependOn(&b.addRunArtifact(process_unit).step);
    test_step.dependOn(&b.addRunArtifact(file_unit).step);
    test_step.dependOn(&b.addRunArtifact(state_unit).step);
    test_step.dependOn(&b.addRunArtifact(conversation_unit).step);
    test_step.dependOn(&b.addRunArtifact(image_unit).step);
    test_step.dependOn(&b.addRunArtifact(syntax_unit).step);
    test_step.dependOn(&b.addRunArtifact(terminal_unit).step);
    test_step.dependOn(&b.addRunArtifact(width_unit).step);
    test_step.dependOn(&b.addRunArtifact(session_unit).step);

    const integration_options = b.addOptions();
    integration_options.addOption([]const u8, "source_root", b.pathFromRoot("."));
    integration_options.addOptionPath("binary", fixture_exe.getEmittedBin());
    integration_options.addOption([]const u8, "installed_binary", b.getInstallPath(.bin, "misa-fixture"));
    const integration_module = b.createModule(.{
        .root_source_file = b.path("tests/integration/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    integration_module.addOptions("integration_options", integration_options);
    const integration_tests = b.addTest(.{ .root_module = integration_module });
    const integration = b.addRunArtifact(integration_tests);
    integration.step.dependOn(b.getInstallStep());
    integration.step.dependOn(&install_fixture.step);
    test_step.dependOn(&integration.step);
    b.step("test-integration", "Run isolated application integration cases").dependOn(&integration.step);

    // Nix is intentionally opt-in rather than part of normal package checks.
    const nix_eval = b.addSystemCommand(&.{ "nix-instantiate", "--eval", "--strict", b.pathFromRoot("tests/nix-eval.nix") });
    b.step("test-nix", "Evaluate Nix API tests").dependOn(&nix_eval.step);
}

fn copyImports(destination: *std.Build.Module, source: *std.Build.Module) void {
    for (source.import_table.keys(), source.import_table.values()) |name, module| destination.addImport(name, module);
}
