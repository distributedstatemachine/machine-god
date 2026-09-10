# Skills CLI components

The complete skills CLI is being integrated as one feature. Its current
implementation and delivery gates are tracked only in the
[implementation plan](implementation-plan.md). The native routing API below
does not by itself register an interactive CLI command or install authority.
Existing model-facing [skill](skill.md) and [install_skill](install-skill.md)
tools keep their independent contracts and permission boundaries.

## Native command routing

`NativeSkillsCommand::from_str` accepts the payload after `/skills`. It is
bounded, synchronous and effect-free: it does not inspect a catalog, directory,
environment, process, credential or destination. The full payload is limited to
8,192 UTF-8 bytes before trimming or allocation. C0/C1 controls except tab are
rejected. Exact lowercase verbs use ASCII space/tab separators:

| Payload | Native operation |
| --- | --- |
| empty or `list` | `List` |
| `path` | `Path` |
| `show <selector>` | `Show` |
| `create <arguments>` | `Create` |
| `add <arguments>` or `install <arguments>` | `Install` |
| `remove <selector>` | `Remove` |

`list` and `path` accept no extra arguments. Other forms require a nonempty
remainder. Show/remove selectors are limited to 4,096 bytes. Outer ASCII
whitespace is removed; interior bytes, including Unicode and spaces, remain
unchanged. This routing layer does not interpret quoting, source URLs, locations,
installation flags or replacement consent. The corresponding native domain
adapter must validate those before admitting effects. Pasted `npx`/`bunx` text
is installer input data, never permission to execute a package manager.

All routing errors use the fixed `NativeSkillsCommandError`, displaying
`invalid native skills command`. Command debug output names only the operation,
never arguments, locations or source credentials. Programmatically constructed
enum values are likewise data: native domain adapters must still validate their
contents and obtain the appropriate owned admission.
