const Harness = @import("harness.zig").Harness;

test "headless completion drains queued rejection diagnostics before exit" {
    for ([_][]const u8{
        "(local standard (require :misa.standard))\n\n(standard.application\n  {:config {}\n   :modules {\n    \"module-1\" {:priority 0 :build (require \"json\")}\n    \"module-2\" {:priority 1000 :build (require \"layout\")}\n    \"module-3\" {:priority 2000 :build (require \"commands\")}\n    \"module-4\" {:priority 3000 :build (require \"choices\")}\n    \"module-5\" {:priority 4000 :build (require \"agent\")}\n    \"module-6\" {:priority 5000 :build (require \"queue\")}\n    \"module-7\" {:priority 6000 :build (require \"editor\")}\n    \"module-8\" {:priority 7000 :build ((. (require :fennel) :dofile) \"@ROOT@/tests/integration/fixtures/queued-rejection.fnl\")}}})\n",
        "(local standard (require :misa.standard))\n\n(standard.application\n  {:config {}\n   :modules {\n    \"module-1\" {:priority 0 :build (require \"json\")}\n    \"module-2\" {:priority 1000 :build (require \"layout\")}\n    \"module-3\" {:priority 2000 :build (require \"commands\")}\n    \"module-4\" {:priority 3000 :build (require \"choices\")}\n    \"module-5\" {:priority 4000 :build (require \"agent\")}\n    \"module-6\" {:priority 5000 :build (require \"editor\")}\n    \"module-7\" {:priority 6000 :build (require \"queue\")}\n    \"module-8\" {:priority 7000 :build ((. (require :fennel) :dofile) \"@ROOT@/tests/integration/fixtures/queued-rejection.fnl\")}}})\n",
    }) |config| {
        var h = try Harness.init();
        defer h.deinit();
        try h.config(config);
        try h.expect(.{ .args = &.{"startup"}, .input = "", .timeout_ms = 3000 }, "rejection completed\n");
    }
}

test "Claude reports a failed MCP connection instead of silently running without tools" {
    var h = try Harness.init();
    defer h.deinit();
    try h.executable("claude",
        \\#!/bin/sh
        \\cat >/dev/null
        \\printf '%s\n' '{"type":"system","subtype":"init","mcp_servers":[{"name":"misa","status":"failed"}]}'
        \\exec sleep 10
    );
    try h.config(@embedFile("configs/claude.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .timeout_ms = 3000 }, "Claude could not connect to Misa's MCP tools (failed).\n");
}

test "unserializable" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/unserializable.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "request options cannot be serialized: unknown\n");
}

test "fake" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/fake.fnl"));
    try h.expect(.{ .args = &.{ "hello", "world" }, .input = "" }, "fake response\n");
}

test "stream" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/stream.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "stream works\n");
}

test "rate check" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/rate-check.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "rated\n");
}

test "interrupted" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/interrupted.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "stream interrupted\n");
}

test "effort cycle" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/effort-cycle.fnl"));
    try h.expect(.{ .args = &.{}, .input = "\x1bfhello\n" }, "cycled\n");
}

test "effort fallback" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/effort-fallback.fnl"));
    try h.expect(.{ .args = &.{}, .input = "/model fake/narrow\nhello\n" }, "fallback\n");
}

test "effort unsupported" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/effort-unsupported.fnl"));
    try h.expect(.{ .args = &.{}, .input = "\x1bfhello\n" }, "plain\n");
}

test "required option" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/required-option.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "required request options are missing: region\n");
}

test "dynamic models" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/dynamic-models.fnl"));
    try h.expect(.{ .args = &.{"test"}, .input = "" }, "dynamic model\n");
}

test "unavailable models" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/unavailable-models.fnl"));
    try h.expect(.{ .args = &.{}, .input = "hello\n" }, "configured model is unavailable: private/model\n");
}

test "tool loop" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/tool-loop.fnl"));
    try h.expect(.{ .args = &.{ "use", "tool" }, .input = "" }, "after tool\n");
}

test "parallel tools" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/parallel-tools.fnl"));
    try h.expect(.{ .args = &.{"parallel"}, .input = "" }, "parallel ordered\n");
}

test "cross provider" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/cross-provider.fnl"));
    try h.expect(.{ .args = &.{"replay"}, .input = "" }, "portable replay\n");
}

test "cancel tools" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/cancel-tools.fnl"));
    try h.expect(.{ .args = &.{"cancel"}, .input = "" }, "Cancelled\ncancel tools\n");
}

test "ui first" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/ui-first.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "ui first\n");
}

test "native file and shell tools complete an agent loop" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/native-tools.fnl"));
    try h.expect(.{ .args = &.{ "use", "native", "tools" } }, "tools done\n");
    try @import("std").testing.expectEqualStrings("beta", try h.read("native-tool.txt"));
}

test "command argv stays literal and invalid output bytes are sanitized" {
    var h = try Harness.init();
    defer h.deinit();
    try h.executable("provider", @embedFile("fixtures/integration-provider"));
    try h.config(@embedFile("configs/command.fnl"));
    try h.expect(.{ .args = &.{ "$(touch SHOULD_NOT_EXIST);", "it's", "literal" } }, "result $(touch SHOULD_NOT_EXIST); it's literal:�\n");
    try @import("std").testing.expectError(error.FileNotFound, h.temporary.dir.statFile(@import("harness.zig").io, "SHOULD_NOT_EXIST", .{}));
}

test "Claude semantic completion terminates the process before EOF" {
    const std = @import("std");
    const io = @import("harness.zig").io;
    var h = try Harness.init();
    defer h.deinit();
    try h.executable("claude", @embedFile("fixtures/integration-claude"));
    // The CLI's MCP arguments must reference the selected configuration.
    try h.write("claude.fnl", @embedFile("configs/claude.fnl"));
    try h.environ.put("MISA_CONFIG", try h.path("claude.fnl"));
    const started = std.Io.Timestamp.now(io, .awake);
    try h.expect(.{ .args = &.{"hello"} }, "claude result\n");
    const elapsed = started.durationTo(.now(io, .awake));
    try std.testing.expect(elapsed.toMilliseconds() < 2000);
}

test "provider-owned tool messages preserve distinct stream indices" {
    var h = try Harness.init();
    defer h.deinit();
    try h.executable("claude", @embedFile("fixtures/runtime-regressions-claude"));
    try h.config(@embedFile("configs/runtime-regressions.fnl"));
    try h.environ.put("MISA_EXPECT_CONFIG", try h.path("config.fnl"));
    try h.environ.put("MISA_CONFIG", try h.path("not-selected.fnl"));
    try h.expect(.{ .args = &.{ "--config", "@WORK@/config.fnl", "test" } }, "");
}

test "invalid tool input becomes a recoverable tool result" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/invalid.fnl"));
    try h.expect(.{ .args = &.{"test"} }, "");
}

test "agent reset ignores a stale tool completion" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/reset.fnl"));
    try h.expect(.{}, "");
}
