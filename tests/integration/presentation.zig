const Harness = @import("harness.zig").Harness;

test "indicators" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/indicators.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "indicators\n");
}

test "semantic components" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/semantic-components.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "swapped\n");
}

test "unicode layout" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/unicode-layout.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "unicode layout\n");
}

test "visual swap" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/visual-swap.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "independent chrome\n");
}

test "message redaction" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/message-redaction.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "redacted\n");
}

test "semantic transcript" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/semantic-transcript.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "semantic\n");
}

test "markdown rendering" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/markdown-rendering.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "markdown\n");
}

test "review regressions" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/review-regressions.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "review regressions\n");
}

test "transcript order" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/transcript-order.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "ordinary **bold**tail\nordered\n");
}

test "response metadata" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/response-metadata.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "firstsecond\nmetadata\n");
}

test "message controls" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/message-controls.json"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "ok red!\n");
}

test "presentation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/presentation.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "presentation\n");
}

test "selection document" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/selection-document.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "selection document\n");
}

test "markdown large" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/markdown-large.json"));
    try h.expect(.{ .args = &.{}, .input = "" }, "markdown regressions\n");
}

test "costs" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/costs.json"));
    try h.expect(.{}, "costs\n");
}

test "history" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/history.json"));
    try h.expect(.{}, "history\n");
}

test "choice compact" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/choice-compact.json"));
    try h.expect(.{}, "compact choices\n");
}

test "transcript interaction" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/transcript-interaction.json"));
    try h.expect(.{}, "transcript interaction\n");
}
