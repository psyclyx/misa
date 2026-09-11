//! Dependency-free Unicode terminal cell widths and grapheme clusters.
//!
//! The presenter measures frames with this module and `misa.ui.layout`
//! measures text in Lua through it, so both agree by construction.
const std = @import("std");

const WidthInterval = struct { first: u21, last: u21 };

fn inIntervals(cp: u21, comptime intervals: []const WidthInterval) bool {
    var low: usize = 0;
    var high: usize = intervals.len;
    while (low < high) {
        const middle = low + (high - low) / 2;
        const interval = intervals[middle];
        if (cp < interval.first) high = middle else if (cp > interval.last) low = middle + 1 else return true;
    }
    return false;
}

// Local wcwidth-style tables. Unlisted printable Unicode is one cell; this is
// more accurate than treating every non-ASCII codepoint as double-width.
const zero_width = [_]WidthInterval{
    .{ .first = 0x0300, .last = 0x036f },   .{ .first = 0x0483, .last = 0x0489 },
    .{ .first = 0x0591, .last = 0x05bd },   .{ .first = 0x05bf, .last = 0x05bf },
    .{ .first = 0x05c1, .last = 0x05c2 },   .{ .first = 0x05c4, .last = 0x05c5 },
    .{ .first = 0x0610, .last = 0x061a },   .{ .first = 0x064b, .last = 0x065f },
    .{ .first = 0x0670, .last = 0x0670 },   .{ .first = 0x06d6, .last = 0x06ed },
    .{ .first = 0x0711, .last = 0x0711 },   .{ .first = 0x0730, .last = 0x074a },
    .{ .first = 0x07a6, .last = 0x07b0 },   .{ .first = 0x07eb, .last = 0x07f3 },
    .{ .first = 0x0816, .last = 0x082d },   .{ .first = 0x0859, .last = 0x085b },
    .{ .first = 0x08d3, .last = 0x0902 },   .{ .first = 0x093a, .last = 0x093c },
    .{ .first = 0x0941, .last = 0x0948 },   .{ .first = 0x094d, .last = 0x094d },
    .{ .first = 0x0951, .last = 0x0957 },   .{ .first = 0x0962, .last = 0x0963 },
    .{ .first = 0x1ab0, .last = 0x1aff },   .{ .first = 0x1dc0, .last = 0x1dff },
    .{ .first = 0x200b, .last = 0x200f },   .{ .first = 0x202a, .last = 0x202e },
    .{ .first = 0x2060, .last = 0x206f },   .{ .first = 0x20d0, .last = 0x20ff },
    .{ .first = 0xfe00, .last = 0xfe0f },   .{ .first = 0xfe20, .last = 0xfe2f },
    .{ .first = 0xfeff, .last = 0xfeff },   .{ .first = 0x1f3fb, .last = 0x1f3ff },
    .{ .first = 0xe0020, .last = 0xe007f }, .{ .first = 0xe0100, .last = 0xe01ef },
};

const wide = [_]WidthInterval{
    .{ .first = 0x1100, .last = 0x115f },   .{ .first = 0x231a, .last = 0x231b },
    .{ .first = 0x2329, .last = 0x232a },   .{ .first = 0x23e9, .last = 0x23ec },
    .{ .first = 0x23f0, .last = 0x23f0 },   .{ .first = 0x23f3, .last = 0x23f3 },
    .{ .first = 0x25fd, .last = 0x25fe },   .{ .first = 0x2614, .last = 0x2615 },
    .{ .first = 0x2648, .last = 0x2653 },   .{ .first = 0x267f, .last = 0x267f },
    .{ .first = 0x2693, .last = 0x2693 },   .{ .first = 0x26a1, .last = 0x26a1 },
    .{ .first = 0x26aa, .last = 0x26ab },   .{ .first = 0x26bd, .last = 0x26be },
    .{ .first = 0x26c4, .last = 0x26c5 },   .{ .first = 0x26ce, .last = 0x26ce },
    .{ .first = 0x26d4, .last = 0x26d4 },   .{ .first = 0x26ea, .last = 0x26ea },
    .{ .first = 0x26f2, .last = 0x26f3 },   .{ .first = 0x26f5, .last = 0x26f5 },
    .{ .first = 0x26fa, .last = 0x26fa },   .{ .first = 0x26fd, .last = 0x26fd },
    .{ .first = 0x2705, .last = 0x2705 },   .{ .first = 0x270a, .last = 0x270b },
    .{ .first = 0x2728, .last = 0x2728 },   .{ .first = 0x274c, .last = 0x274c },
    .{ .first = 0x274e, .last = 0x274e },   .{ .first = 0x2753, .last = 0x2755 },
    .{ .first = 0x2757, .last = 0x2757 },   .{ .first = 0x2795, .last = 0x2797 },
    .{ .first = 0x27b0, .last = 0x27b0 },   .{ .first = 0x27bf, .last = 0x27bf },
    .{ .first = 0x2b1b, .last = 0x2b1c },   .{ .first = 0x2b50, .last = 0x2b50 },
    .{ .first = 0x2b55, .last = 0x2b55 },   .{ .first = 0x2e80, .last = 0x303e },
    .{ .first = 0x3040, .last = 0xa4cf },   .{ .first = 0xac00, .last = 0xd7a3 },
    .{ .first = 0xf900, .last = 0xfaff },   .{ .first = 0xfe10, .last = 0xfe19 },
    .{ .first = 0xfe30, .last = 0xfe6f },   .{ .first = 0xff00, .last = 0xff60 },
    .{ .first = 0xffe0, .last = 0xffe6 },   .{ .first = 0x1f004, .last = 0x1f004 },
    .{ .first = 0x1f0cf, .last = 0x1f0cf }, .{ .first = 0x1f18e, .last = 0x1f18e },
    .{ .first = 0x1f191, .last = 0x1f19a }, .{ .first = 0x1f200, .last = 0x1f251 },
    .{ .first = 0x1f300, .last = 0x1f64f }, .{ .first = 0x1f680, .last = 0x1f6ff },
    .{ .first = 0x1f900, .last = 0x1f9ff }, .{ .first = 0x1fa70, .last = 0x1faff },
    .{ .first = 0x20000, .last = 0x3fffd },
};

pub fn displayWidth(cp: u21) usize {
    if (inIntervals(cp, &zero_width) or isVirama(cp)) return 0;
    return if (inIntervals(cp, &wide)) 2 else 1;
}

fn isRegional(cp: u21) bool {
    return cp >= 0x1f1e6 and cp <= 0x1f1ff;
}

// UAX #29 GB9c linkers. Keeping this explicit and conservative avoids
// splitting common Indic conjuncts without pretending every combining mark
// joins an arbitrary following character.
fn isVirama(cp: u21) bool {
    return switch (cp) {
        0x094d,
        0x09cd,
        0x0a4d,
        0x0acd,
        0x0b4d,
        0x0bcd,
        0x0c4d,
        0x0ccd,
        0x0d3b,
        0x0d3c,
        0x0d4d,
        0x0dca,
        0x0e3a,
        0x0f84,
        0x1039,
        0x103a,
        0x1714,
        0x1715,
        0x1734,
        0x17d2,
        0x1a60,
        0x1b44,
        0x1baa,
        0x1bab,
        0x1bf2,
        0x1bf3,
        0xa806,
        0xa8c4,
        0xa953,
        0xa9c0,
        0xaaf6,
        0xabed,
        0x10a3f,
        0x11046,
        0x11070,
        0x11133,
        0x11134,
        0x111c0,
        0x11235,
        0x112ea,
        0x1134d,
        0x11442,
        0x114c2,
        0x115bf,
        0x115c0,
        0x1163f,
        0x116b6,
        0x1172b,
        0x11839,
        0x1193d,
        0x1193e,
        0x11943,
        0x119e0,
        0x11a34,
        0x11a47,
        0x11a99,
        0x11c3f,
        0x11d44,
        0x11d45,
        0x11d97,
        => true,
        else => false,
    };
}

fn isIndicLetter(cp: u21) bool {
    return (cp >= 0x0900 and cp <= 0x0dff) or (cp >= 0x1000 and cp <= 0x109f) or
        (cp >= 0x1780 and cp <= 0x17ff) or (cp >= 0xa800 and cp <= 0xabff) or
        (cp >= 0x11000 and cp <= 0x11dff);
}

pub const Cluster = struct { end: usize, width: usize };

pub fn nextCluster(text: []const u8, start: usize) !Cluster {
    if (start >= text.len) return .{ .end = start, .width = 0 };
    var it = std.unicode.Utf8Iterator{ .bytes = text, .i = start };
    const first = it.nextCodepointSlice() orelse return error.InvalidUtf8;
    const first_cp = try std.unicode.utf8Decode(first);
    var width = displayWidth(first_cp);
    var emoji = isRegional(first_cp);
    const flag = isRegional(first_cp);
    var after_virama = false;
    while (it.nextCodepointSlice()) |encoded| {
        const cp = try std.unicode.utf8Decode(encoded);
        if (after_virama and isIndicLetter(cp) and !inIntervals(cp, &zero_width)) {
            width = @max(width, displayWidth(cp));
            after_virama = false;
            continue;
        }
        if (cp == 0x200d) {
            const joined = it.nextCodepointSlice() orelse return .{ .end = it.i, .width = @max(width, 2) };
            width = @max(width, displayWidth(try std.unicode.utf8Decode(joined)));
            emoji = true;
            continue;
        }
        if (isVirama(cp) or inIntervals(cp, &zero_width)) {
            if (cp == 0xfe0f or cp == 0x20e3 or (cp >= 0x1f3fb and cp <= 0x1f3ff)) emoji = true;
            if (isVirama(cp)) after_virama = true;
            continue;
        }
        if (flag and isRegional(cp)) {
            emoji = true;
            return .{ .end = it.i, .width = @max(width, 2) };
        }
        return .{ .end = it.i - encoded.len, .width = if (emoji) @max(width, 2) else width };
    }
    return .{ .end = it.i, .width = if (emoji) @max(width, 2) else width };
}

/// Width of UTF-8 text by extended emoji cluster. This deliberately keeps the
/// dependency-free wcwidth tables, while treating variation-selector emoji,
/// flags, modifiers, and ZWJ chains as the single glyph terminals render.
pub fn textWidth(text: []const u8) !usize {
    var at: usize = 0;
    var total: usize = 0;
    while (at < text.len) {
        const cluster = try nextCluster(text, at);
        if (cluster.end <= at) return error.InvalidUtf8;
        total += cluster.width;
        at = cluster.end;
    }
    return total;
}

test "display width handles narrow scripts, combining sequences, and wide glyphs" {
    try std.testing.expectEqual(@as(usize, 1), displayWidth('é'));
    try std.testing.expectEqual(@as(usize, 1), displayWidth('Ω'));
    try std.testing.expectEqual(@as(usize, 1), displayWidth('Ж'));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0x0301));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0xfe0f));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0x200d));
    try std.testing.expectEqual(@as(usize, 0), displayWidth(0x1f3fb));
    try std.testing.expectEqual(@as(usize, 2), displayWidth('界'));
    try std.testing.expectEqual(@as(usize, 2), displayWidth(0x1f600));
}

test "emoji and Indic conjuncts occupy one grapheme" {
    try std.testing.expectEqual(@as(usize, 2), try textWidth("👩‍💻"));
    try std.testing.expectEqual(@as(usize, 2), try textWidth("🏳️‍🌈"));
    try std.testing.expectEqual(@as(usize, 2), try textWidth("©️"));
    try std.testing.expectEqual(@as(usize, 3), try textWidth("a👨‍👩‍👧‍👦"));
    const conjunct = "क्ष"; // KA + VIRAMA + SSA
    const cluster = try nextCluster(conjunct, 0);
    try std.testing.expectEqual(conjunct.len, cluster.end);
    try std.testing.expectEqual(@as(usize, 1), cluster.width);
    const bengali = "ক্ষ";
    try std.testing.expectEqual(bengali.len, (try nextCluster(bengali, 0)).end);
}
