//! OAuth device and PKCE flows for subscription providers.
const std = @import("std");
const builtin = @import("builtin");

pub const Credential = struct {
    access: []u8,
    refresh: []u8,
    expires: i64,
    account_id: ?[]u8 = null,

    pub fn deinit(self: Credential, allocator: std.mem.Allocator) void {
        allocator.free(self.access);
        allocator.free(self.refresh);
        if (self.account_id) |value| allocator.free(value);
    }
};

const Response = struct {
    status: u16,
    body: []u8,
    fn deinit(self: Response, allocator: std.mem.Allocator) void {
        allocator.free(self.body);
    }
};

const openai_client_id = "app_EMoamEEZ73f0CkXaXp7hrann";
const kimi_client_id = "17e5f671-d194-4dfb-9706-5516cb48c098";

pub const Prompt = struct {
    correlation: []const u8,
    kind: []const u8 = "progress",
    title: []const u8,
    message: []const u8,
    url: ?[]const u8 = null,
    code: ?[]const u8 = null,
    progress: ?[]const u8 = null,
    cancellable: bool = true,
    input: bool = false,
    protected: bool = false,
    hints: []const []const u8 = &.{},
};

pub fn loginDevice(allocator: std.mem.Allocator, io: std.Io, authorization_url: []const u8, token_url: []const u8, client_id: []const u8, interaction: anytype) !Credential {
    const form = try formEncode(allocator, &.{.{ "client_id", client_id }});
    defer allocator.free(form);
    const started = try post(allocator, io, authorization_url, "application/x-www-form-urlencoded", form);
    defer started.deinit(allocator);
    if (started.status < 200 or started.status >= 300) return error.DeviceAuthorizationFailed;
    var device = try parseObject(allocator, started.body);
    defer device.deinit();
    const object = device.value.object;
    const device_code = string(object, "device_code") orelse return error.InvalidOAuthResponse;
    const user_code = string(object, "user_code") orelse return error.InvalidOAuthResponse;
    const verification = string(object, "verification_uri_complete") orelse string(object, "verification_uri") orelse return error.InvalidOAuthResponse;
    const interval = positiveInteger(object, "interval") orelse 5;
    const expires_in = positiveInteger(object, "expires_in") orelse 900;
    try interaction.emit(.{ .correlation = "device", .title = "Authorize device", .message = "Open the URL and enter the code. This screen remains active while authorization is checked.", .url = verification, .code = user_code, .progress = "Waiting for authorization…" });
    _ = try launchBrowser(io, verification);

    var elapsed: i64 = 0;
    while (elapsed < expires_in) : (elapsed += interval) {
        io.sleep(.fromSeconds(interval), .awake) catch return error.LoginCancelled;
        const progress = try std.fmt.allocPrint(allocator, "Waiting for authorization… {d}s", .{elapsed + interval});
        defer allocator.free(progress);
        try interaction.emit(.{ .correlation = "device", .title = "Authorize device", .message = "Open the URL and enter the code.", .url = verification, .code = user_code, .progress = progress });
        const token_form = try formEncode(allocator, &.{
            .{ "client_id", client_id },
            .{ "device_code", device_code },
            .{ "grant_type", "urn:ietf:params:oauth:grant-type:device_code" },
        });
        defer allocator.free(token_form);
        const response = try post(allocator, io, token_url, "application/x-www-form-urlencoded", token_form);
        defer response.deinit(allocator);
        var parsed = parseObject(allocator, response.body) catch continue;
        defer parsed.deinit();
        if (response.status >= 200 and response.status < 300 and string(parsed.value.object, "access_token") != null)
            return credentialFromToken(allocator, io, parsed.value.object, null);
        const code = string(parsed.value.object, "error") orelse "";
        if (std.mem.eql(u8, code, "authorization_pending") or std.mem.eql(u8, code, "slow_down")) continue;
        return error.DeviceTokenFailed;
    }
    return error.DeviceAuthorizationExpired;
}

pub fn loginKimi(allocator: std.mem.Allocator, io: std.Io, authorization_url: []const u8, token_url: []const u8, interaction: anytype) !Credential {
    return loginDevice(allocator, io, authorization_url, token_url, kimi_client_id, interaction);
}

pub fn loginOpenAI(allocator: std.mem.Allocator, io: std.Io, interaction: anytype) !Credential {
    const request = try std.json.Stringify.valueAlloc(allocator, .{ .client_id = openai_client_id }, .{});
    defer allocator.free(request);
    const started = try post(allocator, io, "https://auth.openai.com/api/accounts/deviceauth/usercode", "application/json", request);
    defer started.deinit(allocator);
    if (started.status < 200 or started.status >= 300) return error.DeviceAuthorizationFailed;
    var device = try parseObject(allocator, started.body);
    defer device.deinit();
    const object = device.value.object;
    const device_id = string(object, "device_auth_id") orelse return error.InvalidOAuthResponse;
    const user_code = string(object, "user_code") orelse return error.InvalidOAuthResponse;
    const interval = positiveInteger(object, "interval") orelse 5;
    try interaction.emit(.{ .correlation = "device", .title = "Authorize device", .message = "Open the URL and enter the code. This screen remains active while authorization is checked.", .url = "https://auth.openai.com/codex/device", .code = user_code, .progress = "Waiting for authorization…" });
    _ = try launchBrowser(io, "https://auth.openai.com/codex/device");

    var elapsed: i64 = 0;
    while (elapsed < 900) : (elapsed += interval) {
        io.sleep(.fromSeconds(interval), .awake) catch return error.LoginCancelled;
        const progress = try std.fmt.allocPrint(allocator, "Waiting for authorization… {d}s", .{elapsed + interval});
        defer allocator.free(progress);
        try interaction.emit(.{ .correlation = "device", .title = "Authorize device", .message = "Open the URL and enter the code.", .url = "https://auth.openai.com/codex/device", .code = user_code, .progress = progress });
        const poll_body = try std.json.Stringify.valueAlloc(allocator, .{ .device_auth_id = device_id, .user_code = user_code }, .{});
        defer allocator.free(poll_body);
        const response = try post(allocator, io, "https://auth.openai.com/api/accounts/deviceauth/token", "application/json", poll_body);
        defer response.deinit(allocator);
        if (response.status == 403 or response.status == 404) continue;
        if (response.status < 200 or response.status >= 300) return error.DeviceTokenFailed;
        var code = try parseObject(allocator, response.body);
        defer code.deinit();
        const authorization_code = string(code.value.object, "authorization_code") orelse return error.InvalidOAuthResponse;
        const verifier = string(code.value.object, "code_verifier") orelse return error.InvalidOAuthResponse;
        const exchange = try formEncode(allocator, &.{
            .{ "grant_type", "authorization_code" },                            .{ "client_id", openai_client_id },
            .{ "code", authorization_code },                                    .{ "code_verifier", verifier },
            .{ "redirect_uri", "https://auth.openai.com/deviceauth/callback" },
        });
        defer allocator.free(exchange);
        const token = try post(allocator, io, "https://auth.openai.com/oauth/token", "application/x-www-form-urlencoded", exchange);
        defer token.deinit(allocator);
        if (token.status < 200 or token.status >= 300) return error.TokenExchangeFailed;
        var parsed = try parseObject(allocator, token.body);
        defer parsed.deinit();
        const access = string(parsed.value.object, "access_token") orelse return error.InvalidOAuthResponse;
        const account = try openAIAccountId(allocator, access);
        errdefer allocator.free(account);
        return credentialFromToken(allocator, io, parsed.value.object, account);
    }
    return error.DeviceAuthorizationExpired;
}

pub fn refresh(allocator: std.mem.Allocator, io: std.Io, provider: []const u8, refresh_token: []const u8, stored_endpoint: ?[]const u8, stored_profile: ?[]const u8) !Credential {
    const endpoint: []const u8 = if (std.mem.eql(u8, provider, "openai-codex"))
        "https://auth.openai.com/oauth/token"
    else if (std.mem.eql(u8, provider, "kimi-coding")) blk: {
        const candidate = stored_endpoint orelse return error.RefreshProfileMissing;
        if (!std.mem.eql(u8, candidate, "https://auth.kimi.ai/api/oauth/token") and !std.mem.eql(u8, candidate, "https://auth.kimi.com/api/oauth/token")) return error.UntrustedRefreshEndpoint;
        break :blk candidate;
    } else if (std.mem.startsWith(u8, provider, "generic/")) blk: {
        const candidate = stored_endpoint orelse return error.RefreshProfileMissing;
        if (!std.mem.startsWith(u8, candidate, "https://") or std.mem.indexOfAny(u8, candidate["https://".len..], "?#\r\n\x00") != null) return error.UntrustedRefreshEndpoint;
        break :blk candidate;
    } else return error.RefreshUnsupported;
    const client_id = if (std.mem.eql(u8, provider, "openai-codex")) openai_client_id else if (std.mem.eql(u8, provider, "kimi-coding")) kimi_client_id else stored_profile orelse return error.RefreshProfileMissing;
    const body = try formEncode(allocator, &.{
        .{ "grant_type", "refresh_token" }, .{ "client_id", client_id }, .{ "refresh_token", refresh_token },
    });
    defer allocator.free(body);
    const response = try post(allocator, io, endpoint, "application/x-www-form-urlencoded", body);
    defer response.deinit(allocator);
    if (response.status < 200 or response.status >= 300) return error.TokenRefreshFailed;
    var parsed = try parseObject(allocator, response.body);
    defer parsed.deinit();
    const access = string(parsed.value.object, "access_token") orelse return error.InvalidOAuthResponse;
    const account = if (std.mem.eql(u8, provider, "openai-codex")) try openAIAccountId(allocator, access) else null;
    errdefer if (account) |value| allocator.free(value);
    return credentialFromToken(allocator, io, parsed.value.object, account);
}

pub const OpenRouterAuthorization = struct {
    verifier: []u8,
    state: []u8,
    url: []u8,
    pub fn deinit(self: OpenRouterAuthorization, allocator: std.mem.Allocator) void {
        allocator.free(self.verifier);
        allocator.free(self.state);
        allocator.free(self.url);
    }
};

pub fn startOpenRouter(allocator: std.mem.Allocator, io: std.Io) !OpenRouterAuthorization {
    var random: [32]u8 = undefined;
    try io.randomSecure(&random);
    const encoder = std.base64.url_safe_no_pad.Encoder;
    const verifier = try allocator.alloc(u8, encoder.calcSize(random.len));
    _ = encoder.encode(verifier, &random);
    errdefer allocator.free(verifier);
    var state_random: [24]u8 = undefined;
    try io.randomSecure(&state_random);
    const state = try allocator.alloc(u8, encoder.calcSize(state_random.len));
    _ = encoder.encode(state, &state_random);
    errdefer allocator.free(state);
    var digest: [32]u8 = undefined;
    std.crypto.hash.sha2.Sha256.hash(verifier, &digest, .{});
    const challenge = try allocator.alloc(u8, encoder.calcSize(digest.len));
    defer allocator.free(challenge);
    _ = encoder.encode(challenge, &digest);
    const query = try formEncode(allocator, &.{
        .{ "code_challenge", challenge },
        .{ "code_challenge_method", "S256" },
        .{ "state", state },
    });
    defer allocator.free(query);
    return .{ .verifier = verifier, .state = state, .url = try std.fmt.allocPrint(allocator, "https://openrouter.ai/auth?{s}", .{query}) };
}

/// Prefer a temporary loopback callback. If binding or launching a browser is
/// unavailable, request a pasted code through the same generic interaction.
pub fn authorizeOpenRouter(allocator: std.mem.Allocator, io: std.Io, authorization: OpenRouterAuthorization, interaction: anytype) ![]u8 {
    const net = std.Io.net;
    var port: u16 = 14553;
    var server: ?net.Server = null;
    while (port < 14569) : (port += 1) {
        var address: net.IpAddress = .{ .ip4 = .loopback(port) };
        server = address.listen(io, .{ .reuse_address = true }) catch |err| switch (err) {
            error.AddressInUse => continue,
            error.Canceled => return error.Canceled,
            else => null,
        };
        if (server != null) break;
    }
    if (server) |*listener| {
        defer listener.deinit(io);
        const callback = try std.fmt.allocPrint(allocator, "http://127.0.0.1:{d}/callback", .{port});
        defer allocator.free(callback);
        const encoded_callback = try formEncode(allocator, &.{.{ "callback_url", callback }});
        defer allocator.free(encoded_callback);
        const url = try std.fmt.allocPrint(allocator, "{s}&{s}", .{ authorization.url, encoded_callback });
        defer allocator.free(url);
        const launched = try launchBrowser(io, url);
        if (launched) {
            try interaction.emit(.{ .correlation = "callback", .title = "Authorize in browser", .message = "Complete authorization in your browser. The callback is accepted only from this device.", .url = url, .progress = "Waiting for browser callback…" });
            // Browsers and local probes may open malformed/stale connections.
            // Reject them and keep the one-shot listener alive for the valid
            // state-bearing callback instead of aborting the login.
            for (0..16) |_| {
                var stream = try listener.accept(io);
                defer stream.close(io);
                var in_buffer: [8192]u8 = undefined;
                var out_buffer: [2048]u8 = undefined;
                var reader = stream.reader(io, &in_buffer);
                var writer = stream.writer(io, &out_buffer);
                var http_server = std.http.Server.init(&reader.interface, &writer.interface);
                var request = http_server.receiveHead() catch continue;
                if (request.head.method != .GET or !std.mem.startsWith(u8, request.head.target, "/callback?")) {
                    try request.respond("Invalid OAuth callback.", .{ .status = .bad_request });
                    continue;
                }
                const code = callbackCodeAlloc(allocator, request.head.target, authorization.state) catch {
                    try request.respond("Invalid OAuth callback encoding.", .{ .status = .bad_request });
                    continue;
                } orelse {
                    try request.respond("Authorization failed: callback state did not match. You can close this tab.", .{ .status = .bad_request });
                    continue;
                };
                if (code.len == 0 or code.len > 4096) {
                    allocator.free(code);
                    try request.respond("Invalid authorization code.", .{ .status = .bad_request });
                    continue;
                }
                errdefer allocator.free(code);
                try request.respond("Authorization complete. You can close this tab and return to misa.", .{ .status = .ok });
                return code;
            }
            return error.TooManyInvalidCallbacks;
        }
    }
    try interaction.emit(.{ .correlation = "paste", .kind = "modal", .title = "Paste authorization code", .message = "Automatic browser callback is unavailable. Open the URL, then paste the code or redirect URL here.", .url = authorization.url, .input = true });
    const value = try interaction.input("paste");
    if (value.len > 8192) return error.AuthorizationResponseTooLarge;
    if (std.mem.indexOf(u8, value, "://") != null)
        return try callbackCodeAlloc(allocator, value, authorization.state) orelse error.CallbackStateMismatch;
    return allocator.dupe(u8, value);
}

fn launchBrowser(io: std.Io, url: []const u8) !bool {
    const Task = struct {
        done: std.atomic.Value(bool) = .init(false),
        success: std.atomic.Value(bool) = .init(false),
        fn run(self: *@This(), task_io: std.Io, target: []const u8) std.Io.Cancelable!void {
            defer self.done.store(true, .release);
            var child = std.process.spawn(task_io, .{ .argv = &.{ if (builtin.os.tag == .macos) "open" else "xdg-open", target }, .stdin = .ignore, .stdout = .ignore, .stderr = .ignore }) catch |err| {
                if (err == error.Canceled) return error.Canceled;
                return;
            };
            defer child.kill(task_io);
            const term = child.wait(task_io) catch |err| {
                if (err == error.Canceled) return error.Canceled;
                return;
            };
            self.success.store(term == .exited and term.exited == 0, .release);
        }
    };
    var task: Task = .{};
    var group: std.Io.Group = .init;
    group.async(io, Task.run, .{ &task, io, url });
    const deadline = std.Io.Timestamp.now(io, .awake).nanoseconds + 5 * std.time.ns_per_s;
    while (!task.done.load(.acquire) and std.Io.Timestamp.now(io, .awake).nanoseconds < deadline)
        std.Io.sleep(io, .fromMilliseconds(10), .awake) catch {
            group.cancel(io);
            return error.Canceled;
        };
    if (!task.done.load(.acquire)) {
        group.cancel(io);
        return false;
    }
    try group.await(io);
    return task.success.load(.acquire);
}

fn callbackCodeAlloc(allocator: std.mem.Allocator, target: []const u8, state: []const u8) !?[]u8 {
    const query_start = std.mem.indexOfScalar(u8, target, '?') orelse return null;
    const query = target[query_start + 1 ..];
    var encoded_code: ?[]const u8 = null;
    var encoded_state: ?[]const u8 = null;
    var fields = std.mem.splitScalar(u8, query, '&');
    while (fields.next()) |field| {
        if (std.mem.startsWith(u8, field, "code=")) encoded_code = field[5..] else if (std.mem.startsWith(u8, field, "state=")) encoded_state = field[6..];
    }
    const state_value = try percentDecode(allocator, encoded_state orelse return null);
    defer allocator.free(state_value);
    if (!std.mem.eql(u8, state_value, state)) return null;
    return try percentDecode(allocator, encoded_code orelse return null);
}

fn percentDecode(allocator: std.mem.Allocator, encoded: []const u8) ![]u8 {
    var out: std.ArrayList(u8) = .empty;
    errdefer out.deinit(allocator);
    var i: usize = 0;
    while (i < encoded.len) {
        if (encoded[i] == '%') {
            if (i + 2 >= encoded.len) return error.InvalidPercentEncoding;
            const high = std.fmt.charToDigit(encoded[i + 1], 16) catch return error.InvalidPercentEncoding;
            const low = std.fmt.charToDigit(encoded[i + 2], 16) catch return error.InvalidPercentEncoding;
            const byte: u8 = @intCast(high * 16 + low);
            if (byte == 0) return error.InvalidPercentEncoding;
            try out.append(allocator, byte);
            i += 3;
        } else {
            try out.append(allocator, if (encoded[i] == '+') ' ' else encoded[i]);
            i += 1;
        }
    }
    return out.toOwnedSlice(allocator);
}

pub fn finishOpenRouter(allocator: std.mem.Allocator, io: std.Io, verifier: []const u8, input: []const u8) !Credential {
    const code = authorizationCode(input) orelse return error.AuthorizationCodeMissing;
    const body = try std.json.Stringify.valueAlloc(allocator, .{ .code = code, .code_verifier = verifier, .code_challenge_method = "S256" }, .{});
    defer allocator.free(body);
    const response = try post(allocator, io, "https://openrouter.ai/api/v1/auth/keys", "application/json", body);
    defer response.deinit(allocator);
    if (response.status < 200 or response.status >= 300) return error.TokenExchangeFailed;
    var parsed = try parseObject(allocator, response.body);
    defer parsed.deinit();
    const key = string(parsed.value.object, "key") orelse return error.InvalidOAuthResponse;
    const access = try allocator.dupe(u8, key);
    errdefer allocator.free(access);
    return .{
        .access = access,
        .refresh = try allocator.dupe(u8, ""),
        .expires = std.math.maxInt(i64),
    };
}

fn credentialFromToken(allocator: std.mem.Allocator, io: std.Io, object: std.json.ObjectMap, account_id: ?[]u8) !Credential {
    const access = string(object, "access_token") orelse return error.InvalidOAuthResponse;
    const refresh_value = string(object, "refresh_token") orelse return error.InvalidOAuthResponse;
    const expires = positiveInteger(object, "expires_in") orelse return error.InvalidOAuthResponse;
    const access_copy = try allocator.dupe(u8, access);
    errdefer allocator.free(access_copy);
    const refresh_copy = try allocator.dupe(u8, refresh_value);
    return .{
        .access = access_copy,
        .refresh = refresh_copy,
        .expires = std.Io.Clock.real.now(io).toSeconds() + expires,
        .account_id = account_id,
    };
}

fn post(allocator: std.mem.Allocator, io: std.Io, url: []const u8, content_type: []const u8, payload: []const u8) !Response {
    var client: std.http.Client = .{ .allocator = allocator, .io = io };
    defer client.deinit();
    var output: BoundedWriter = .{ .allocator = allocator, .limit = 1024 * 1024 };
    errdefer output.deinit();
    const result = client.fetch(.{
        .location = .{ .url = url },
        .method = .POST,
        .payload = payload,
        .extra_headers = &.{ .{ .name = "content-type", .value = content_type }, .{ .name = "accept", .value = "application/json" } },
        .response_writer = &output.writer,
    }) catch |err| {
        if (output.exceeded) return error.OAuthResponseTooLarge;
        return err;
    };
    return .{ .status = @intFromEnum(result.status), .body = try output.toOwnedSlice() };
}

const BoundedWriter = struct {
    writer: std.Io.Writer = .{ .vtable = &.{ .drain = drain }, .buffer = &.{} },
    allocator: std.mem.Allocator,
    limit: usize,
    bytes: std.ArrayList(u8) = .empty,
    exceeded: bool = false,

    fn deinit(self: *BoundedWriter) void {
        self.bytes.deinit(self.allocator);
    }
    fn toOwnedSlice(self: *BoundedWriter) ![]u8 {
        return self.bytes.toOwnedSlice(self.allocator);
    }
    fn drain(writer: *std.Io.Writer, data: []const []const u8, splat: usize) std.Io.Writer.Error!usize {
        const self: *BoundedWriter = @alignCast(@fieldParentPtr("writer", writer));
        var consumed: usize = 0;
        if (data.len == 0) return 0;
        for (data[0 .. data.len - 1]) |part| {
            self.append(part) catch return error.WriteFailed;
            consumed += part.len;
        }
        for (0..splat) |_| {
            const part = data[data.len - 1];
            self.append(part) catch return error.WriteFailed;
            consumed += part.len;
        }
        return consumed;
    }
    fn append(self: *BoundedWriter, value: []const u8) !void {
        if (value.len > self.limit -| self.bytes.items.len) {
            self.exceeded = true;
            return error.ResponseTooLarge;
        }
        try self.bytes.appendSlice(self.allocator, value);
    }
};

fn parseObject(allocator: std.mem.Allocator, source: []const u8) !std.json.Parsed(std.json.Value) {
    var parsed = try std.json.parseFromSlice(std.json.Value, allocator, source, .{ .allocate = .alloc_always });
    errdefer parsed.deinit();
    if (parsed.value != .object) return error.InvalidOAuthResponse;
    return parsed;
}

fn string(object: std.json.ObjectMap, name: []const u8) ?[]const u8 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .string => |item| item,
        else => null,
    };
}

fn positiveInteger(object: std.json.ObjectMap, name: []const u8) ?i64 {
    const value = object.get(name) orelse return null;
    return switch (value) {
        .integer => |item| if (item > 0) item else null,
        .float => |item| if (item > 0 and @trunc(item) == item) @intFromFloat(item) else null,
        .string => |item| std.fmt.parseInt(i64, item, 10) catch null,
        else => null,
    };
}

fn formEncode(allocator: std.mem.Allocator, fields: []const struct { []const u8, []const u8 }) ![]u8 {
    var result: std.ArrayList(u8) = .empty;
    errdefer result.deinit(allocator);
    for (fields, 0..) |field, index| {
        if (index != 0) try result.append(allocator, '&');
        try percentEncode(&result, allocator, field[0]);
        try result.append(allocator, '=');
        try percentEncode(&result, allocator, field[1]);
    }
    return result.toOwnedSlice(allocator);
}

fn percentEncode(out: *std.ArrayList(u8), allocator: std.mem.Allocator, value: []const u8) !void {
    const hex = "0123456789ABCDEF";
    for (value) |byte| {
        if (std.ascii.isAlphanumeric(byte) or byte == '-' or byte == '_' or byte == '.' or byte == '~')
            try out.append(allocator, byte)
        else
            try out.appendSlice(allocator, &.{ '%', hex[byte >> 4], hex[byte & 15] });
    }
}

fn authorizationCode(input: []const u8) ?[]const u8 {
    const trimmed = std.mem.trim(u8, input, " \t\r\n");
    if (std.mem.indexOf(u8, trimmed, "code=")) |start| {
        const rest = trimmed[start + 5 ..];
        return rest[0 .. std.mem.indexOfAny(u8, rest, "&#") orelse rest.len];
    }
    return if (trimmed.len == 0) null else trimmed;
}

fn openAIAccountId(allocator: std.mem.Allocator, access: []const u8) ![]u8 {
    var parts = std.mem.splitScalar(u8, access, '.');
    _ = parts.next() orelse return error.InvalidAccessToken;
    const payload = parts.next() orelse return error.InvalidAccessToken;
    const decoder = std.base64.url_safe_no_pad.Decoder;
    const size = try decoder.calcSizeForSlice(payload);
    const decoded = try allocator.alloc(u8, size);
    defer allocator.free(decoded);
    try decoder.decode(decoded, payload);
    var parsed = try parseObject(allocator, decoded);
    defer parsed.deinit();
    const claim = switch (parsed.value.object.get("https://api.openai.com/auth") orelse return error.AccountIdMissing) {
        .object => |object| object,
        else => return error.AccountIdMissing,
    };
    return allocator.dupe(u8, string(claim, "chatgpt_account_id") orelse return error.AccountIdMissing);
}

test "OAuth response writer rejects before crossing its allocation bound" {
    var writer: BoundedWriter = .{ .allocator = std.testing.allocator, .limit = 4 };
    defer writer.deinit();
    try writer.append("1234");
    try std.testing.expectError(error.ResponseTooLarge, writer.append("5"));
    try std.testing.expectEqual(@as(usize, 4), writer.bytes.items.len);
    try std.testing.expect(writer.exceeded);
}

test "OpenRouter callback percent-decodes code and state" {
    const code = (try callbackCodeAlloc(std.testing.allocator, "/callback?code=a%2Bb%20c&state=ex%70ected", "expected")).?;
    defer std.testing.allocator.free(code);
    try std.testing.expectEqualStrings("a+b c", code);
    try std.testing.expect((try callbackCodeAlloc(std.testing.allocator, "/callback?code=abc&state=wrong", "expected")) == null);
    try std.testing.expect((try callbackCodeAlloc(std.testing.allocator, "/callback?code=abc", "expected")) == null);
    try std.testing.expectError(error.InvalidPercentEncoding, callbackCodeAlloc(std.testing.allocator, "/callback?code=%GG&state=expected", "expected"));
}

test "authorization input accepts redirect URLs and bare codes" {
    try std.testing.expectEqualStrings("abc", authorizationCode("http://localhost/callback?code=abc&state=x").?);
    try std.testing.expectEqualStrings("abc", authorizationCode(" abc\n").?);
}
