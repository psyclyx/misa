//! Query-free tree-sitter highlighting over dynamically loaded grammars.
const std = @import("std");
const options = @import("misa_syntax_options");
const c = @cImport(@cInclude("tree_sitter/api.h"));

pub const default_grammar_dir = options.default_grammar_dir;
pub const max_source_bytes: usize = 1024 * 1024;
pub const max_language_bytes: usize = 64;
const max_parsers: usize = 32;

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
/// by its owning Lua runtime thread.
pub const Highlighter = struct {
    allocator: std.mem.Allocator,
    grammar_dir: []u8,
    entries: std.ArrayList(Entry) = .empty,

    pub fn init(allocator: std.mem.Allocator, grammar_dir: []const u8) !Highlighter {
        return .{ .allocator = allocator, .grammar_dir = try allocator.dupe(u8, grammar_dir) };
    }

    pub fn deinit(self: *Highlighter) void {
        for (self.entries.items) |*entry| entry.deinit(self.allocator);
        self.entries.deinit(self.allocator);
        self.allocator.free(self.grammar_dir);
    }

    /// Returns generic semantic captures. Missing, incompatible, and unknown
    /// grammars deliberately produce an empty result so callers can render plain text.
    pub fn highlight(self: *Highlighter, allocator: std.mem.Allocator, language_label: []const u8, source: []const u8) ![]Capture {
        if (language_label.len == 0 or language_label.len > max_language_bytes) return error.InvalidLanguage;
        if (source.len > max_source_bytes) return error.SourceTooLarge;
        const name = canonicalLanguage(language_label) orelse return &.{};
        const parser = try self.getParser(name) orelse return &.{};
        const tree = c.ts_parser_parse_string(parser, null, source.ptr, @intCast(source.len)) orelse return &.{};
        defer c.ts_tree_delete(tree);

        var captures: std.ArrayList(Capture) = .empty;
        errdefer captures.deinit(allocator);
        try walk(c.ts_tree_root_node(tree), null, &captures, allocator);
        return captures.toOwnedSlice(allocator);
    }

    fn getParser(self: *Highlighter, name: []const u8) !?*c.TSParser {
        for (self.entries.items) |entry| if (std.mem.eql(u8, entry.name, name)) return entry.parser;
        if (self.entries.items.len >= max_parsers or self.grammar_dir.len == 0) return null;

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
        try self.entries.append(self.allocator, .{ .name = owned_name, .library = library, .parser = parser_ptr });
        keep_library = true;
        keep_parser = true;
        return parser_ptr;
    }
};

fn walk(node: c.TSNode, inherited: ?[]const u8, captures: *std.ArrayList(Capture), allocator: std.mem.Allocator) !void {
    const node_type = std.mem.span(c.ts_node_type(node));
    const own = classify(node_type);
    const child_count = c.ts_node_child_count(node);
    if (child_count == 0) {
        const semantic = if (inherited) |outer|
            if (std.mem.eql(u8, outer, names.string) and own != null and std.mem.eql(u8, own.?, names.escape)) own else outer
        else
            own orelse classifyContext(node);
        if (semantic) |capture| {
            const start = c.ts_node_start_byte(node);
            const end = c.ts_node_end_byte(node);
            if (end > start) try captures.append(allocator, .{ .start_byte = start, .end_byte = end, .capture = capture });
        }
        return;
    }
    const child_inherited = inherited orelse if (own) |semantic| if (isLexical(semantic)) semantic else null else null;
    for (0..child_count) |index| try walk(c.ts_node_child(node, @intCast(index)), child_inherited, captures, allocator);
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
