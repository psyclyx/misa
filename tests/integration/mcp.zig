const std = @import("std");
const support = @import("harness.zig");
const Harness = support.Harness;

test "MCP initialization advertises object capabilities accepted by strict SDK clients" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/mcp.fnl"));
    try h.expect(.{ .args = &.{"mcp"}, .input =
        \\{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"sdk","version":"1"}}}
        \\
    }, "{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocolVersion\":\"2025-03-26\",\"capabilities\":{\"tools\":{}},\"serverInfo\":{\"name\":\"misa\",\"version\":\"0.1.0\"}}}\n");
}

test "MCP ping preserves its complete JSON RPC envelope" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/mcp.fnl"));
    try h.expect(.{ .args = &.{"mcp"}, .input = "{\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"ping\"}\n" }, "{\"jsonrpc\":\"2.0\",\"id\":42,\"result\":{}}\n");
}

test "MCP shell tool returns the complete result envelope" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/mcp.fnl"));
    try h.expect(.{ .args = &.{"mcp"}, .input =
        \\{"jsonrpc":"2.0","id":43,"method":"tools/call","params":{"name":"shell","arguments":{"command":"printf mcp-tool-ok"}}}
        \\
    }, "{\"jsonrpc\":\"2.0\",\"id\":43,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"mcp-tool-ok\"}],\"isError\":false}}\n");
}

test "MCP exposes extension-defined tools" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/tool-loop.fnl"));
    const result = try h.run(.{ .args = &.{"mcp"}, .input =
        \\{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"value":"from tool"}}}
        \\
    });
    try support.success(result);
    try support.contains(result.stdout, "\"text\":\"from tool\"");
}

test "MCP initialization listing and file writes share the tool registry" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/native-tools.fnl"));
    const result = try h.run(.{ .args = &.{"mcp"}, .input = try h.expand(
        \\{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
        \\{"jsonrpc":"2.0","method":"notifications/initialized"}
        \\{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
        \\{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"@WORK@/mcp-tool.txt","content":"from mcp"}}}
        \\
    ) });
    try support.success(result);
    for ([_][]const u8{ "\"name\":\"read_file\"", "\"name\":\"list_directory\"", "\"name\":\"web_search\"", "\"isError\":false" }) |needle| try support.contains(result.stdout, needle);
    try std.testing.expectEqualStrings("from mcp", try h.read("mcp-tool.txt"));
}

test "MCP hashline edits check snapshots and return fresh anchors" {
    var h = try Harness.init();
    defer h.deinit();
    try h.config(@embedFile("configs/native-tools.fnl"));
    const result = try h.run(.{ .args = &.{"mcp"}, .input = try h.expand(
        \\{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"write_file","arguments":{"path":"@WORK@/hashline.txt","content":"alpha"}}}
        \\{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"@WORK@/hashline.txt"}}}
        \\{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"edit_file","arguments":{"path":"@WORK@/hashline.txt","snapshot":"8ED3F6AD685B959E","start":"1#8ED3F6AD","new_text":"beta"}}}
        \\{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"edit_file","arguments":{"path":"@WORK@/hashline.txt","snapshot":"8ED3F6AD685B959E","start":"1#8ED3F6AD","new_text":"stale"}}}
        \\
    ) });
    try support.success(result);
    try support.contains(result.stdout, "snapshot 8ED3F6AD685B959E\\n1#8ED3F6AD|alpha");
    try support.contains(result.stdout, "snapshot F44E64E75F3948E9\\n1#F44E64E7|beta");
    try support.contains(result.stdout, "StaleSnapshotReadFileAgain");
    try std.testing.expectEqualStrings("beta", try h.read("hashline.txt"));
}
