const std = @import("std");

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const config = b.createModule(.{
        .root_source_file = b.path("src/config/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const lua_runtime = b.createModule(.{
        .root_source_file = b.path("src/lua_runtime/root.zig"),
        .target = target,
        .optimize = optimize,
        .link_libc = true,
    });
    lua_runtime.addCSourceFile(.{ .file = b.path("src/lua_runtime/shim.c"), .flags = &.{"-std=c99"} });
    lua_runtime.linkSystemLibrary("luajit", .{ .use_pkg_config = .force });

    const main_module = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    main_module.addImport("misa_config", config);
    main_module.addImport("misa_lua_runtime", lua_runtime);

    const exe = b.addExecutable(.{ .name = "misa", .root_module = main_module });
    b.installArtifact(exe);

    const run = b.addRunArtifact(exe);
    if (b.args) |args| run.addArgs(args);
    b.step("run", "Run misa").dependOn(&run.step);

    const unit = b.addTest(.{ .root_module = config });
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&b.addRunArtifact(unit).step);

    const integration = b.addSystemCommand(&.{ "sh", b.pathFromRoot("tests/integration.sh") });
    integration.addFileArg(exe.getEmittedBin());
    test_step.dependOn(&integration.step);
}
