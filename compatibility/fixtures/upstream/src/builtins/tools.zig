const ToolSpec = tool_dispatch.Tool;

pub const read_file = ToolSpec{
    .name = "read_file",
};

pub const terminal = ToolSpec{
    .name = "terminal",
};

pub const all = [_]tool_dispatch.Tool{
    read_file,
    terminal,
};
