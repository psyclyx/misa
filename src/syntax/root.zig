//! Query-free tree-sitter highlighting over dynamically loaded grammars.
const std = @import("std");
const options = @import("misa_syntax_options");
const c = @cImport(@cInclude("tree_sitter/api.h"));

pub const default_grammar_dir = options.default_grammar_dir;
pub const Spec = @import("operation.zig").Spec;
pub const Service = @import("operation.zig").Service;
pub const max_source_bytes: usize = 1024 * 1024;
pub const max_language_bytes: usize = 64;
const max_parsers: usize = 32;
pub const max_ast_depth: usize = 512;
pub const max_ast_nodes: usize = 250_000;
pub const max_ast_work: usize = 1_000_000;

pub const Capture = struct {
    start_byte: u32,
    end_byte: u32,
    capture: []const u8,
};

const Entry = struct {
    name: []u8,
    library: std.DynLib,
    parser: *c.TSParser,

    fn deinit(self: *Entry, allocator: std.mem.Allocator) void {
        c.ts_parser_delete(self.parser);
        self.library.close();
        allocator.free(self.name);
    }
};

/// Owns the bounded lazy parser cache. It is synchronous and must only be used
/// by one native worker at a time.
pub const Highlighter = struct {
    allocator: std.mem.Allocator,
    grammar_dir: []u8,
    entries: std.ArrayList(Entry) = .empty,

    pub fn init(allocator: std.mem.Allocator, grammar_dir: []const u8) !Highlighter {
        var result: Highlighter = .{ .allocator = allocator, .grammar_dir = try allocator.dupe(u8, grammar_dir) };
        errdefer allocator.free(result.grammar_dir);
        try result.entries.ensureTotalCapacity(allocator, max_parsers);
        return result;
    }

    pub fn deinit(self: *Highlighter) void {
        for (self.entries.items) |*entry| entry.deinit(self.allocator);
        self.entries.deinit(self.allocator);
        self.allocator.free(self.grammar_dir);
    }

    /// Returns generic semantic captures. Missing, incompatible, and unknown
    /// grammars deliberately produce an empty result so callers can render plain text.
    pub fn highlight(self: *Highlighter, allocator: std.mem.Allocator, language_label: []const u8, source: []const u8) ![]Capture {
        return self.highlightWithProgress(allocator, language_label, source, null);
    }

    pub fn highlightCancelable(self: *Highlighter, allocator: std.mem.Allocator, io: std.Io, language_label: []const u8, source: []const u8, timeout_ms: u32) ![]Capture {
        try io.checkCancel();
        var progress: Progress = .{ .io = io, .deadline = std.Io.Timestamp.now(io, .awake).nanoseconds + @as(i96, timeout_ms) * std.time.ns_per_ms };
        return self.highlightWithProgress(allocator, language_label, source, &progress);
    }

    fn highlightWithProgress(self: *Highlighter, allocator: std.mem.Allocator, language_label: []const u8, source: []const u8, progress: ?*Progress) ![]Capture {
        if (language_label.len == 0 or language_label.len > max_language_bytes) return error.InvalidLanguage;
        if (source.len > max_source_bytes) return error.SourceTooLarge;
        const name = canonicalLanguage(language_label) orelse return &.{};
        const parser = try self.getParser(name) orelse return &.{};
        var input = source;
        const parsed = if (progress) |context| c.ts_parser_parse_with_options(parser, null, .{
            .payload = @ptrCast(&input),
            .read = readSource,
            .encoding = c.TSInputEncodingUTF8,
            .decode = null,
        }, .{ .payload = context, .progress_callback = parseProgress }) else c.ts_parser_parse_string(parser, null, source.ptr, @intCast(source.len));
        const tree = parsed orelse {
            // Tree-sitter otherwise resumes an interrupted parse on the next
            // request, whose source may be completely different.
            c.ts_parser_reset(parser);
            if (progress) |context| if (context.canceled) return error.Canceled;
            return &.{};
        };
        defer c.ts_tree_delete(tree);
        if (progress) |context| try context.io.checkCancel();

        var captures: std.ArrayList(Capture) = .empty;
        errdefer captures.deinit(allocator);
        walk(c.ts_tree_root_node(tree), &captures, allocator, if (progress) |context| context.io else null) catch |err| switch (err) {
            error.SyntaxTooComplex => {
                captures.clearRetainingCapacity();
                return captures.toOwnedSlice(allocator);
            },
            else => return err,
        };
        return captures.toOwnedSlice(allocator);
    }

    fn getParser(self: *Highlighter, name: []const u8) !?*c.TSParser {
        for (self.entries.items, 0..) |entry, index| if (std.mem.eql(u8, entry.name, name)) {
            // Oldest is at index zero. Promote every hit so grammar use over a
            // long session is bounded by memory, never by a permanent cutoff.
            const promoted = self.entries.orderedRemove(index);
            self.entries.appendAssumeCapacity(promoted);
            return promoted.parser;
        };
        if (self.grammar_dir.len == 0) return null;

        const filename = try std.fmt.allocPrint(self.allocator, "{s}.so", .{name});
        defer self.allocator.free(filename);
        const path = try std.fs.path.join(self.allocator, &.{ self.grammar_dir, filename });
        defer self.allocator.free(path);
        var library = std.DynLib.open(path) catch return null;
        var keep_library = false;
        defer if (!keep_library) library.close();
        const symbol = try std.fmt.allocPrint(self.allocator, "tree_sitter_{s}", .{name});
        defer self.allocator.free(symbol);
        const symbol_name = try self.allocator.dupeZ(u8, symbol);
        defer self.allocator.free(symbol_name);
        const LanguageFn = *const fn () callconv(.c) ?*const c.TSLanguage;
        const language_fn = library.lookup(LanguageFn, symbol_name) orelse return null;
        const language = language_fn() orelse return null;
        const parser_ptr = c.ts_parser_new() orelse return error.OutOfMemory;
        var keep_parser = false;
        defer if (!keep_parser) c.ts_parser_delete(parser_ptr);
        if (!c.ts_parser_set_language(parser_ptr, language)) return null;
        const owned_name = try self.allocator.dupe(u8, name);
        errdefer self.allocator.free(owned_name);
        if (self.entries.items.len == max_parsers) {
            var evicted = self.entries.orderedRemove(0);
            evicted.deinit(self.allocator);
        }
        self.entries.appendAssumeCapacity(.{ .name = owned_name, .library = library, .parser = parser_ptr });
        keep_library = true;
        keep_parser = true;
        return parser_ptr;
    }
};

const Progress = struct { io: std.Io, deadline: i96, canceled: bool = false };

fn readSource(payload: ?*anyopaque, byte_index: u32, _: c.TSPoint, bytes_read: [*c]u32) callconv(.c) [*c]const u8 {
    const source: *[]const u8 = @ptrCast(@alignCast(payload.?));
    if (byte_index >= source.len) {
        bytes_read.* = 0;
        return null;
    }
    bytes_read.* = @intCast(source.len - byte_index);
    return source.ptr + byte_index;
}

fn parseProgress(state: [*c]c.TSParseState) callconv(.c) bool {
    const progress: *Progress = @ptrCast(@alignCast(state.*.payload.?));
    progress.io.checkCancel() catch {
        progress.canceled = true;
        return true;
    };
    return std.Io.Timestamp.now(progress.io, .awake).nanoseconds >= progress.deadline;
}

fn walk(root: c.TSNode, captures: *std.ArrayList(Capture), allocator: std.mem.Allocator, io: ?std.Io) !void {
    const Pending = struct { node: c.TSNode, inherited: ?[]const u8, depth: usize };
    var pending: std.ArrayList(Pending) = .empty;
    defer pending.deinit(allocator);
    try pending.append(allocator, .{ .node = root, .inherited = null, .depth = 0 });
    var nodes: usize = 0;
    var work: usize = 1;
    while (pending.pop()) |current| {
        nodes += 1;
        if (nodes % 256 == 0) if (io) |context| try context.checkCancel();
        if (nodes > max_ast_nodes or current.depth > max_ast_depth) return error.SyntaxTooComplex;
        const node_type = std.mem.span(c.ts_node_type(current.node));
        const own = classify(node_type);
        const child_count: usize = c.ts_node_child_count(current.node);
        if (child_count == 0) {
            const semantic = if (current.inherited) |outer|
                if (std.mem.eql(u8, outer, names.string) and own != null and std.mem.eql(u8, own.?, names.escape)) own else outer
            else
                own orelse classifyContext(current.node);
            if (semantic) |capture| {
                const start = c.ts_node_start_byte(current.node);
                const end = c.ts_node_end_byte(current.node);
                if (end > start) try captures.append(allocator, .{ .start_byte = start, .end_byte = end, .capture = capture });
            }
            continue;
        }
        if (child_count > max_ast_work -| work) return error.SyntaxTooComplex;
        work += child_count;
        const child_inherited = current.inherited orelse if (own) |semantic| if (isLexical(semantic)) semantic else null else null;
        // LIFO reverse insertion preserves tree-sitter's source order.
        var index = child_count;
        while (index != 0) {
            index -= 1;
            try pending.append(allocator, .{ .node = c.ts_node_child(current.node, @intCast(index)), .inherited = child_inherited, .depth = current.depth + 1 });
        }
    }
}

fn isLexical(semantic: []const u8) bool {
    return std.mem.eql(u8, semantic, names.comment) or std.mem.eql(u8, semantic, names.string) or std.mem.eql(u8, semantic, names.embedded);
}

fn classifyContext(node: c.TSNode) ?[]const u8 {
    const parent = c.ts_node_parent(node);
    if (c.ts_node_is_null(parent)) return null;
    const kind = std.mem.span(c.ts_node_type(parent));
    if (contains(kind, "call") or contains(kind, "function") or contains(kind, "method") or contains(kind, "constructor") or contains(kind, "declarator")) return names.function_name;
    if (contains(kind, "type") or contains(kind, "class") or contains(kind, "interface") or contains(kind, "trait")) return names.type_name;
    if (contains(kind, "field") or contains(kind, "property") or contains(kind, "member")) return names.property;
    if (contains(kind, "parameter") or contains(kind, "variable")) return names.variable;
    return null;
}

const names = struct {
    const comment = "comment";
    const string = "string";
    const number = "number";
    const keyword = "keyword";
    const type_name = "type";
    const function_name = "function";
    const constant = "constant";
    const variable = "variable";
    const property = "property";
    const tag = "tag";
    const attribute = "attribute";
    const operator = "operator";
    const punctuation = "punctuation";
    const escape = "escape";
    const embedded = "embedded";
};

fn classify(kind: []const u8) ?[]const u8 {
    if (contains(kind, "comment") or contains(kind, "shebang")) return names.comment;
    if (contains(kind, "escape")) return names.escape;
    if (contains(kind, "string") or contains(kind, "character") or contains(kind, "heredoc") or contains(kind, "regex")) return names.string;
    if (contains(kind, "number") or contains(kind, "integer") or contains(kind, "float")) return names.number;
    if (contains(kind, "keyword") or contains(kind, "modifier")) return names.keyword;
    if (contains(kind, "type") or contains(kind, "class_name") or contains(kind, "interface_name")) return names.type_name;
    if (contains(kind, "function") or contains(kind, "method") or contains(kind, "constructor")) return names.function_name;
    if (contains(kind, "constant") or contains(kind, "boolean") or std.mem.eql(u8, kind, "null") or std.mem.eql(u8, kind, "nil")) return names.constant;
    if (contains(kind, "property") or contains(kind, "field")) return names.property;
    if (contains(kind, "attribute")) return names.attribute;
    if (contains(kind, "tag_name")) return names.tag;
    if (contains(kind, "variable") or contains(kind, "parameter")) return names.variable;
    if (contains(kind, "operator")) return names.operator;
    if (contains(kind, "punctuation")) return names.punctuation;
    if (contains(kind, "embedded")) return names.embedded;
    if (isKeyword(kind)) return names.keyword;
    if (isOperator(kind)) return names.operator;
    if (isPunctuation(kind)) return names.punctuation;
    return null;
}

fn contains(haystack: []const u8, needle: []const u8) bool {
    return std.mem.indexOf(u8, haystack, needle) != null;
}

fn isKeyword(kind: []const u8) bool {
    const keywords = [_][]const u8{ "if", "else", "then", "for", "while", "do", "end", "return", "break", "continue", "switch", "case", "default", "match", "let", "var", "const", "fn", "fun", "function", "class", "struct", "enum", "union", "interface", "trait", "impl", "import", "from", "export", "as", "in", "of", "new", "try", "catch", "throw", "async", "await", "yield", "pub", "private", "protected", "static", "where", "with", "def", "lambda", "package", "namespace", "using" };
    for (keywords) |keyword| if (std.mem.eql(u8, kind, keyword)) return true;
    return false;
}

fn isOperator(kind: []const u8) bool {
    if (kind.len == 0 or kind.len > 4) return false;
    for (kind) |byte| if (std.mem.indexOfScalar(u8, "+-*/%=!<>|&^~?:", byte) == null) return false;
    return true;
}

fn isPunctuation(kind: []const u8) bool {
    if (kind.len != 1) return false;
    return std.mem.indexOfScalar(u8, "()[]{}.,;", kind[0]) != null;
}

fn canonicalLanguage(label: []const u8) ?[]const u8 {
    const aliases = .{
        .{ "js", "javascript" }, .{ "node", "javascript" }, .{ "ts", "typescript" },
        .{ "py", "python" },     .{ "rb", "ruby" },         .{ "rs", "rust" },
        .{ "sh", "bash" },       .{ "shell", "bash" },      .{ "zsh", "bash" },
        .{ "c++", "cpp" },       .{ "cc", "cpp" },          .{ "cxx", "cpp" },
        .{ "c#", "c_sharp" },    .{ "cs", "c_sharp" },      .{ "golang", "go" },
        .{ "yml", "yaml" },      .{ "md", "markdown" },     .{ "html5", "html" },
    };
    inline for (aliases) |alias| if (std.ascii.eqlIgnoreCase(label, alias[0])) return alias[1];
    for (label) |byte| if (!std.ascii.isAlphanumeric(byte) and byte != '_') return null;
    // Grammar files and symbols are lower-case; reject mixed case rather than allocate.
    for (label) |byte| if (std.ascii.isUpper(byte)) return null;
    return label;
}

test "aliases and generic classification are finite" {
    try std.testing.expectEqualStrings("javascript", canonicalLanguage("js").?);
    try std.testing.expectEqualStrings("c_sharp", canonicalLanguage("C#").?);
    try std.testing.expect(canonicalLanguage("../python") == null);
    try std.testing.expectEqualStrings("comment", classify("line_comment").?);
    try std.testing.expectEqualStrings("keyword", classify("return").?);
    try std.testing.expectEqualStrings("operator", classify("=>").?);
    try std.testing.expect(classify("identifier") == null);
}

test "absent and unknown grammars gracefully fall back" {
    var highlighter = try Highlighter.init(std.testing.allocator, "/directory/which/does/not/exist");
    defer highlighter.deinit();
    const absent = try highlighter.highlight(std.testing.allocator, "python", "return 1");
    defer std.testing.allocator.free(absent);
    try std.testing.expectEqual(@as(usize, 0), absent.len);
    const unsafe = try highlighter.highlight(std.testing.allocator, "../python", "return 1");
    defer std.testing.allocator.free(unsafe);
    try std.testing.expectEqual(@as(usize, 0), unsafe.len);
    const oversized = try std.testing.allocator.alloc(u8, max_source_bytes + 1);
    defer std.testing.allocator.free(oversized);
    try std.testing.expectError(error.SourceTooLarge, highlighter.highlight(std.testing.allocator, "python", oversized));
}

test "parser cache evicts least recently used grammars instead of freezing" {
    if (default_grammar_dir.len == 0) return error.SkipZigTest;
    var highlighter = try Highlighter.init(std.testing.allocator, default_grammar_dir);
    defer highlighter.deinit();
    var directory = try std.Io.Dir.openDirAbsolute(std.testing.io, default_grammar_dir, .{ .iterate = true });
    defer directory.close(std.testing.io);
    var iterator = directory.iterate();
    var first: ?[]u8 = null;
    defer if (first) |name| std.testing.allocator.free(name);
    var second: ?[]u8 = null;
    defer if (second) |name| std.testing.allocator.free(name);
    var loaded: usize = 0;
    while (try iterator.next(std.testing.io)) |file| {
        if (!std.mem.endsWith(u8, file.name, ".so")) continue;
        const name = file.name[0 .. file.name.len - 3];
        if (canonicalLanguage(name) == null) continue;
        if (try highlighter.getParser(name) != null) {
            loaded += 1;
            if (first == null) first = try std.testing.allocator.dupe(u8, name) else if (second == null) second = try std.testing.allocator.dupe(u8, name);
            if (loaded == max_parsers) try std.testing.expect(try highlighter.getParser(first.?) != null);
            if (loaded == max_parsers + 1) break;
        }
    }
    if (loaded <= max_parsers) return error.SkipZigTest;
    try std.testing.expectEqual(max_parsers, highlighter.entries.items.len);
    var retained_first = false;
    for (highlighter.entries.items) |entry| {
        retained_first = retained_first or std.mem.eql(u8, entry.name, first.?);
        try std.testing.expect(!std.mem.eql(u8, entry.name, second.?));
    }
    try std.testing.expect(retained_first);
    try std.testing.expect(try highlighter.getParser(second.?) != null);
    try std.testing.expectEqual(max_parsers, highlighter.entries.items.len);
    try std.testing.expectEqualStrings(second.?, highlighter.entries.items[highlighter.entries.items.len - 1].name);
}

test "bundled grammar complexity limit falls back without recursion" {
    if (default_grammar_dir.len == 0) return error.SkipZigTest;
    var highlighter = try Highlighter.init(std.testing.allocator, default_grammar_dir);
    defer highlighter.deinit();
    var source: std.ArrayList(u8) = .empty;
    defer source.deinit(std.testing.allocator);
    try source.appendNTimes(std.testing.allocator, '(', max_ast_depth + 32);
    try source.append(std.testing.allocator, '1');
    try source.appendNTimes(std.testing.allocator, ')', max_ast_depth + 32);
    const captures = try highlighter.highlight(std.testing.allocator, "python", source.items);
    defer std.testing.allocator.free(captures);
    try std.testing.expectEqual(@as(usize, 0), captures.len);
}

test "bundled grammar produces ordered semantic captures" {
    if (default_grammar_dir.len == 0) return error.SkipZigTest;
    var highlighter = try Highlighter.init(std.testing.allocator, default_grammar_dir);
    defer highlighter.deinit();
    const captures = try highlighter.highlight(std.testing.allocator, "py", "def answer():\n  # note\n  return 42\n");
    defer std.testing.allocator.free(captures);
    try std.testing.expect(captures.len > 0);
    var previous_end: u32 = 0;
    var saw_comment = false;
    var saw_number = false;
    for (captures) |capture| {
        try std.testing.expect(capture.start_byte >= previous_end);
        try std.testing.expect(capture.end_byte > capture.start_byte);
        previous_end = capture.end_byte;
        saw_comment = saw_comment or std.mem.eql(u8, capture.capture, "comment");
        saw_number = saw_number or std.mem.eql(u8, capture.capture, "number");
    }
    try std.testing.expect(saw_comment and saw_number);
}

test "canceled parse state is reset before the cached parser handles new source" {
    if (default_grammar_dir.len == 0) return error.SkipZigTest;
    const allocator = std.testing.allocator;
    var highlighter = try Highlighter.init(allocator, default_grammar_dir);
    defer highlighter.deinit();
    var source: std.ArrayList(u8) = .empty;
    defer source.deinit(allocator);
    for (0..20000) |_| try source.appendSlice(allocator, "value = 42\n");
    var expired: Progress = .{ .io = std.testing.io, .deadline = 0 };
    const interrupted = try highlighter.highlightWithProgress(allocator, "python", source.items, &expired);
    defer allocator.free(interrupted);
    try std.testing.expectEqual(@as(usize, 0), interrupted.len);
    const parser = (try highlighter.getParser("python")).?;
    const resumed = try highlighter.highlightCancelable(allocator, std.testing.io, "python", "answer = 123\n", 1000);
    defer allocator.free(resumed);
    try std.testing.expect(resumed.len != 0);
    try std.testing.expectEqual(parser, (try highlighter.getParser("python")).?);
    for (resumed) |capture| try std.testing.expect(capture.end_byte <= "answer = 123\n".len);
}

test "syntax service initializes its cache on first worker request" {
    var service = try Service.init(std.testing.allocator, "");
    defer service.deinit();
    try std.testing.expect(service.highlighter == null);
    const captures = try service.run(std.testing.allocator, std.testing.io, .{ .id = "test", .completion = "test/done", .language = "unknown", .source = "private source" });
    defer std.testing.allocator.free(captures);
    try std.testing.expect(service.highlighter != null);
    try std.testing.expectEqual(@as(usize, 0), captures.len);
}
