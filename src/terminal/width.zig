//! Dependency-free Unicode terminal cell widths.
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
    if (inIntervals(cp, &zero_width)) return 0;
    return if (inIntervals(cp, &wide)) 2 else 1;
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
