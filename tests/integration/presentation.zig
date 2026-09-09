const Harness = @import("harness.zig").Harness;

test "indicators" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/indicators.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "indicators\n");
}

test "semantic components" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/semantic-components.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "swapped\n");
}

test "unicode layout" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/unicode-layout.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "unicode layout\n");
}

test "visual swap" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/visual-swap.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "independent chrome\n");
}

test "message redaction" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/message-redaction.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "redacted\n");
}

test "semantic transcript" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/semantic-transcript.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "semantic\n");
}

test "markdown rendering" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/markdown-rendering.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "markdown\n");
}

test "review regressions" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/review-regressions.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "review regressions\n");
}

test "transcript order" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/transcript-order.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "ordinary **bold**tail\nordered\n");
}

test "response metadata" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/response-metadata.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "firstsecond\nmetadata\n");
}

test "message controls" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/message-controls.fnl"));
    try h.expect(.{ .args = &.{"hello"}, .input = "" }, "ok red!\n");
}

test "presentation" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/presentation.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "presentation\n");
}

test "selection document" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/selection-document.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "selection document\n");
}

test "markdown large" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/markdown-large.fnl"));
    try h.expect(.{ .args = &.{}, .input = "" }, "markdown regressions\n");
}

test "costs" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/costs.fnl"));
    try h.expect(.{}, "costs\n");
}

test "history" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/history.fnl"));
    try h.expect(.{}, "history\n");
}

test "choice compact" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/choice-compact.fnl"));
    try h.expect(.{}, "compact choices\n");
}

test "transcript interaction" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/transcript-interaction.fnl"));
    try h.expect(.{}, "transcript interaction\n");
}
