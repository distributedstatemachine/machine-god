pub const top_level_specs = [_]TopLevelSpec{
    .{
        .kind = .help,
        .token = "help",
        .aliases = &.{ "--help", "-h" },
        .usage = "help",
        .summary = "Show this help",
    },
    .{
        .kind = .ask,
        .token = "ask",
        .usage = "ask <prompt>",
        .summary = "Run one request",
        .hidden_from_top_level_help = true,
    },
};

pub const slash_specs = [_]SlashSpec{
    .{ .kind = .help, .command = "/help", .presentation_category = .general },
    .{ .kind = .quit, .command = "/quit", .aliases = &.{"/exit"}, .presentation_category = .general },
};
