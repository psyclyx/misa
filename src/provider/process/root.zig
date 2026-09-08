//! Live execution of provider commands, separate from general-purpose tools.
const std = @import("std");
const process = @import("misa_process");

pub fn runWithActivity(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, spec: process.Spec, activity: ?process.ActivitySink) !process.Result {
    var owned = spec;
    owned.environment = environ;
    return process.runWithActivity(allocator, io, owned, activity);
}

pub fn runJsonLines(allocator: std.mem.Allocator, io: std.Io, environ: *const std.process.Environ.Map, spec: process.Spec, sink: process.StreamSink) !process.StreamResult {
    var owned = spec;
    owned.environment = environ;
    return process.runJsonLines(allocator, io, owned, sink);
}
