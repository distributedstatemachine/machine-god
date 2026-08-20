// pub const TopLevelKind = enum { commented_fake };
const declaration_text = "pub const SlashKind = enum { string_fake };";
const declaration_multiline =
    \\pub const TopLevelKind = enum { multiline_fake };
;

pub const TopLevelKind = enum {
    // The public help command.
    help,
    ask,
};

pub const SlashKind = enum {
    quit,
    help,
};
