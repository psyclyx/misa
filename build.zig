const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const auth = b.createModule(.{
        .root_source_file = b.path("src/auth/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const config = b.createModule(.{
        .root_source_file = b.path("src/config/root.zig"),
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

    const standard_extension_options = b.addOptions();
    standard_extension_options.addOption([]const u8, "default_extension_dir", b.getInstallPath(.{ .custom = "share/misa" }, "extensions"));
    standard_extension_options.addOption([]const u8, "default_config_path", b.getInstallPath(.{ .custom = "share/misa" }, "default.json"));
    const standard_extensions = b.createModule(.{
        .root_source_file = b.path("src/standard_extensions/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    standard_extensions.addOptions("misa_build_options", standard_extension_options);
    const lua_runtime = b.createModule(.{
        .root_source_file = b.path("src/lua_runtime/root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    lua_runtime.linkSystemLibrary("luajit", .{ .use_pkg_config = .force });
    lua_runtime.addImport("misa_syntax", syntax);
    const terminal = b.createModule(.{
        .root_source_file = b.path("src/terminal/root.zig"),
        .target = target,
        .optimize = optimize,
    });
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
    const state = b.createModule(.{
        .root_source_file = b.path("src/state/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const session = b.createModule(.{
        .root_source_file = b.path("src/session/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    session.addImport("misa_auth", auth);
    session.addImport("misa_file", file_effect);
    session.addImport("misa_lua_runtime", lua_runtime);
    session.addImport("misa_process", process_effect);
    session.addImport("misa_state", state);
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
    main_module.addImport("misa_config", config);
    main_module.addImport("misa_lua_runtime", lua_runtime);
    main_module.addImport("misa_mcp", mcp);
    main_module.addImport("misa_standard_extensions", standard_extensions);
    main_module.addImport("misa_state", state);
    main_module.addImport("misa_syntax", syntax);
    main_module.addImport("misa_terminal", terminal);
    main_module.addImport("misa_session", session);

    const exe = b.addExecutable(.{ .name = "misa", .root_module = main_module });
    b.installArtifact(exe);
    b.installDirectory(.{
        .source_dir = b.path("extensions"),
        .install_dir = .{ .custom = "share/misa" },
        .install_subdir = "extensions",
    });
    b.installFile("config/default.json", "share/misa/default.json");

    const run = b.addRunArtifact(exe);
    run.step.dependOn(b.getInstallStep());
    // The default inferred mode captures stdio, so misa sees pipes instead of
    // the caller's terminal and immediately receives EOF. A TUI run must own
    // the real terminal for its lifetime.
    run.stdio = .inherit;
    run.setEnvironmentVariable("MISA_EXTENSION_DIR", b.pathFromRoot("extensions"));
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
    operation_test_module.addImport("misa_file", file_effect);
    operation_test_module.addImport("misa_process", process_effect);
    operation_test_module.addImport("misa_state", state);
    const operation_unit = b.addTest(.{ .root_module = operation_test_module });
    const config_unit = b.addTest(.{ .root_module = config });
    const resolver_unit = b.addTest(.{ .root_module = standard_extensions });
    const process_unit = b.addTest(.{ .root_module = process_effect });
    const state_unit = b.addTest(.{ .root_module = state });
    const syntax_unit = b.addTest(.{ .root_module = syntax });
    const terminal_unit = b.addTest(.{ .root_module = terminal });
    const session_unit = b.addTest(.{ .root_module = session });
    const test_step = b.step("test", "Run unit and integration tests");
    test_step.dependOn(&b.addRunArtifact(auth_unit).step);
    test_step.dependOn(&b.addRunArtifact(oauth_unit).step);
    test_step.dependOn(&b.addRunArtifact(operation_unit).step);
    test_step.dependOn(&b.addRunArtifact(config_unit).step);
    test_step.dependOn(&b.addRunArtifact(resolver_unit).step);
    test_step.dependOn(&b.addRunArtifact(process_unit).step);
    test_step.dependOn(&b.addRunArtifact(state_unit).step);
    test_step.dependOn(&b.addRunArtifact(syntax_unit).step);
    test_step.dependOn(&b.addRunArtifact(terminal_unit).step);
    test_step.dependOn(&b.addRunArtifact(session_unit).step);

    const integration = b.addSystemCommand(&.{ "sh", b.pathFromRoot("tests/integration.sh") });
    integration.addFileArg(exe.getEmittedBin());
    integration.setEnvironmentVariable("MISA_EXTENSION_DIR", b.pathFromRoot("extensions"));
    test_step.dependOn(&integration.step);

    // Exercise the actual install layout with source-tree overrides absent.
    const installed_smoke = b.addSystemCommand(&.{ "sh", b.pathFromRoot("tests/installed-layout.sh") });
    installed_smoke.addArg(b.getInstallPath(.bin, "misa"));
    installed_smoke.step.dependOn(b.getInstallStep());
    test_step.dependOn(&installed_smoke.step);

    // Nix is intentionally opt-in rather than part of normal package checks.
    const nix_eval = b.addSystemCommand(&.{ "nix-instantiate", "--eval", "--strict", b.pathFromRoot("tests/nix-eval.nix") });
    b.step("test-nix", "Evaluate Nix API tests").dependOn(&nix_eval.step);
}
