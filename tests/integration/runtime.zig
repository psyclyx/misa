const Harness = @import("harness.zig").Harness;

test "empty" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/empty.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "");
}

test "a final projected view flushes without keeping an idle session alive" {
    var h = try Harness.init();
    defer h.deinit();
    try h.write("final-view.fnl",
        \\{:setup (fn [] {:fx [{:type :register/view
        \\                     :handler (fn [] {:lines [{:spans [{:text "final"}]}]})}]})}
    );
    try h.config("{\"extensions\":[\"@WORK@/final-view.fnl\"]}");
    try h.expect(.{ .input = "" }, "");
}

test "contracts" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/contracts.json"));
    try h.expect(.{ .args = &.{"original"}, .input = "" }, "before,first:derived:ordered,second,after,before\n");
}

test "clock" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/clock.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "clock\n");
}

test "multi timer" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/multi-timer.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "timers\n");
}

test "clear state" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/clear-state.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "cleared\n");
}

test "dialog lifecycle" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/dialog-lifecycle.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "dialogs\n");
}

test "protected dialog" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/protected-dialog.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "protected dialog\n");
}

test "empty provider composition does not request credentials" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/providers.json"));
    try h.expect(.{}, "");
}

test "sandbox retains source loading and rejects native capability escapes" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/sandbox.json"));
    try h.expect(.{}, "");
}

test "animation lifecycle with motion enabled and disabled" {
    for ([_]bool{ true, false }) |enabled| {
        var h = try Harness.init();
        defer h.deinit();
        const config = @embedFile("configs/animations.json");
        try h.config(if (enabled) config else try @import("std").mem.replaceOwned(u8, h.allocator(), config, "\"enabled\":true", "\"enabled\":false"));
        try h.expect(.{}, "animations\n");
    }
}

test "explicit Lua extensions remain compatible beside bundled Fennel" {
    var h = try Harness.init();
    defer h.deinit();
    // This is deliberately Lua: user extension compatibility is a public contract.
    try h.write("compatibility.lua",
        \\return {setup=function()
        \\  return {fx={{type="register/event", name="app/start", handler=function()
        \\    return {fx={{type="view/commit",lines={{spans={{text="Lua compatibility"}}}}},{type="app/quit"}}}
        \\  end}}}
        \\end}
    );
    try h.config(
        \\{"extensions":["themes","theme.default","@WORK@/compatibility.lua"]}
    );
    try h.expect(.{}, "Lua compatibility\n");
}
