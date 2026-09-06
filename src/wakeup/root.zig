//! Coalescing, nonblocking POSIX wakeup shared by native event owners.
const std = @import("std");
const posix = std.posix;

pub const Wakeup = struct {
    read_fd: posix.fd_t,
    write_fd: posix.fd_t,

    pub fn init() !Wakeup {
        var fds: [2]posix.fd_t = undefined;
        switch (posix.errno(posix.system.pipe2(&fds, .{ .NONBLOCK = true, .CLOEXEC = true }))) {
            .SUCCESS => {},
            else => return error.WakeupPipeUnavailable,
        }
        return .{ .read_fd = fds[0], .write_fd = fds[1] };
    }

    pub fn deinit(self: Wakeup) void {
        _ = posix.system.close(self.read_fd);
        _ = posix.system.close(self.write_fd);
    }

    /// Drain the level-triggered wakeup. Writers may coalesce notifications;
    /// channel and completion state remain authoritative.
    pub fn consume(self: Wakeup) void {
        var bytes: [128]u8 = undefined;
        while (true) switch (posix.errno(posix.system.read(self.read_fd, &bytes, bytes.len))) {
            .SUCCESS => continue,
            .AGAIN => return,
            .INTR => continue,
            else => return,
        };
    }

    pub fn notify(self: Wakeup) void {
        const byte: [1]u8 = .{1};
        while (true) switch (posix.errno(posix.system.write(self.write_fd, &byte, byte.len))) {
            .SUCCESS, .AGAIN => return,
            .INTR => continue,
            else => return,
        };
    }
};
