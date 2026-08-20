const ToolSpec = tool_dispatch.Tool;

// pub const all = [_]tool_dispatch.Tool{commented_fake};
const registry_text = "pub const all = [_]tool_dispatch.Tool{string_fake};";

pub const read_file = ToolSpec{
    .name = "read_file",
};

pub const terminal = ToolSpec{
    .name = "terminal",
};

pub const all = [_]tool_dispatch.Tool{
    // commented_fake,
    read_file,
    terminal,
};
