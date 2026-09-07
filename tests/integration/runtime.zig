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

test "syntax highlighting completes asynchronously without exposing a synchronous API" {
    var h = try Harness.init();
    defer h.deinit();
    try h.write("syntax-effect.lua",
        \\return {setup=function(context)
        \\  assert(misa.syntax == nil or misa.syntax.highlight == nil, "synchronous syntax capability remains exposed")
        \\  local source = "local answer = 42 -- comment\n"
        \\  return {fx={
        \\    {type="register/event",name="app/start",handler=function(db)
        \\      db.received = {}
        \\      return {db=db,fx={
        \\        {type="syntax/highlight",id="known",language="lua",source=source,completion="fixture/highlighted"},
        \\        {type="syntax/highlight",id="unknown",language="fixture-unknown-language",source=source,completion="fixture/highlighted"},
        \\        {type="syntax/highlight",id="empty",language="lua",source="",completion="fixture/highlighted"}}}
        \\    end},
        \\    {type="register/event",name="fixture/highlighted",handler=function(db,event)
        \\      assert(event.type == "fixture/highlighted" and event.ok == true)
        \\      assert(event.id == "known" or event.id == "unknown" or event.id == "empty")
        \\      assert(not db.received[event.id], "duplicate syntax completion")
        \\      assert(event.source == nil and event.language == nil, "syntax completion echoed request content")
        \\      assert(type(event.data) == "table")
        \\      if event.id ~= "known" then assert(#event.data == 0, "empty/unknown grammar did not fall back") end
        \\      if event.id == "known" and context.config.expect_captures then assert(#event.data > 0, "installed Lua grammar produced no captures") end
        \\      local previous = 0
        \\      for _,capture in ipairs(event.data) do
        \\        assert(type(capture.start_byte) == "number" and capture.start_byte >= previous)
        \\        assert(capture.end_byte > capture.start_byte and capture.end_byte <= #source)
        \\        assert(type(capture.capture) == "string" and capture.capture ~= "")
        \\        previous = capture.end_byte
        \\      end
        \\      db.received[event.id] = true
        \\      if db.received.known and db.received.unknown and db.received.empty then
        \\        return {db=db,fx={{type="view/commit",lines={{spans={{text="async syntax"}}}}},{type="app/quit"}}}
        \\      end
        \\      return {db=db}
        \\    end}}}
        \\end}
    );
    const expect_captures = if (h.environ.get("MISA_TREE_SITTER_DIR")) |value| value.len != 0 else false;
    try h.config(if (expect_captures)
        "{\"extensions\":[\"@WORK@/syntax-effect.lua\"],\"config\":{\"expect_captures\":true}}"
    else
        "{\"extensions\":[\"@WORK@/syntax-effect.lua\"]}");
    try h.expect(.{ .timeout_ms = 3000 }, "async syntax\n");
}

test "syntax highlighting rejects malformed requests before execution" {
    const std = @import("std");
    const support = @import("harness.zig");
    for ([_][]const u8{
        "effect.id = nil",
        "effect.id = ''",
        "effect.language = nil",
        "effect.language = ''",
        "effect.language = string.rep('x', 65)",
        "effect.source = nil",
        "effect.source = 17",
        "effect.source = string.char(255)",
        "effect.source = string.rep('x', 1024 * 1024 + 1)",
        "effect.completion = nil",
        "effect.completion = ''",
    }) |mutation| {
        var h = try Harness.init();
        defer h.deinit();
        const source = try std.fmt.allocPrint(h.allocator(),
            \\return {{setup=function()
            \\  return {{fx={{{{type="register/event",name="app/start",handler=function()
            \\    local effect = {{type="syntax/highlight",id="bad",language="lua",source="local x=1",completion="done"}}
            \\    {s}
            \\    return {{fx={{effect}}}}
            \\  end}}}}}}
            \\end}}
        , .{mutation});
        try h.write("invalid-syntax.lua", source);
        try h.config("{\"extensions\":[\"@WORK@/invalid-syntax.lua\"]}");
        const result = try h.run(.{ .timeout_ms = 3000 });
        try std.testing.expect(result.term == .exited and result.term.exited != 0);
        try support.contains(result.stderr, "InvalidEffect");
    }
}

test "component resolution preserves cached semantic spans across themes" {
    var h = try Harness.init();
    defer h.deinit();
    try h.write("cached-component.lua",
        \\return {setup=function()
        \\  local cached = {lines={{spans={{text="cached",style="plain",animation={id="cached",interval_ms=40,frames={{style="bold"},{text="second"}}}}}}}}
        \\  return {fx={
        \\    {type="register/theme",id="second",value={palette={ink="red"},styles={plain={foreground="ink"}}}},
        \\    {type="register/component",id="default.cached",value={render=function() return cached end}},
        \\    {type="register/event",name="app/start",handler=function(db)
        \\      local first = misa.render_component(db,"cached",{})
        \\      assert(cached.lines[1].spans[1].style == "plain")
        \\      assert(cached.lines[1].spans[1].animation.frames[1].style == "bold")
        \\      local old = first.lines[1].spans[1].style.foreground
        \\      db = misa.swap_theme(db,"second")
        \\      local second = misa.render_component(db,"cached",{})
        \\      assert(second.lines[1].spans[1].style.foreground == "red")
        \\      assert(first.lines[1].spans[1].style.foreground == old)
        \\      assert(cached.lines[1].spans[1].animation.frames[1].style == "bold")
        \\      return {fx={{type="view/commit",lines={{spans={{text="pure components"}}}}},{type="app/quit"}}}
        \\    end}}}
        \\end}
    );
    try h.config("{\"extensions\":[\"themes\",\"theme.default\",\"components\",\"@WORK@/cached-component.lua\"],\"config\":{\"themes\":{\"persist\":false},\"components\":{\"persist\":false}}}");
    try h.expect(.{}, "pure components\n");
}
