//! Fixture applications describe requests but have no live HTTP implementation.
const std = @import("std");
const contract = @import("misa_http_contract");
pub const Spec = contract.Spec;
pub const Activity = contract.Activity;
pub const StreamSink = contract.StreamSink;
pub const Result = contract.Result;
pub const SseResult = contract.SseResult;

pub fn request(_: std.mem.Allocator, _: std.Io, _: *const std.process.Environ.Map, _: Spec, _: ?Activity) anyerror!Result {
    return error.UnmatchedHttpFixture;
}

pub fn requestSse(_: std.mem.Allocator, _: std.Io, _: *const std.process.Environ.Map, _: Spec, _: StreamSink) anyerror!SseResult {
    return error.UnmatchedHttpFixture;
}
