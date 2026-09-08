use super::invalid;
use crate::tool_output_serializer::{CompactToolOutputLimits, measure_json_value_compact};
use machine_god_core::{
    CancellationToken, Capability, PermissionError, PermissionInvocation, PermissionRequest,
    ToolCall,
};
use std::io::{self, Write};

pub(super) fn copy_invocation(
    request: &PermissionRequest,
    invocation: PermissionInvocation<'_>,
    cancellation: &CancellationToken,
) -> Result<ToolCall, PermissionError> {
    if request.reason.len() > 128 * 1024 || !scalar_capability_bounds(&request.capability) {
        return Err(invalid());
    }
    let terminal = invocation.tool_name.as_str() == "terminal";
    let limits = CompactToolOutputLimits {
        output_bytes: if terminal {
            crate::MAX_TERMINAL_PREPARED_ARGUMENT_BYTES
        } else {
            1024 * 1024
        },
        json_nodes: if terminal {
            crate::MAX_TERMINAL_ACTION_ARGUMENT_NODES + 128
        } else {
            65_536
        },
        json_depth: machine_god_core::MAX_SAFE_JSON_DEPTH,
    };
    measure_json_value_compact(invocation.arguments, limits, cancellation)
        .map_err(|_| invalid())?;
    if let Capability::Tool { arguments, .. }
    | Capability::Custom {
        details: arguments, ..
    } = &request.capability
    {
        let capability_limits = CompactToolOutputLimits {
            output_bytes: limits.output_bytes.saturating_add(64 * 1024),
            json_nodes: limits.json_nodes.saturating_add(128),
            ..limits
        };
        measure_json_value_compact(arguments, capability_limits, cancellation)
            .map_err(|_| invalid())?;
    }
    // All recursive JSON has already passed the iterative node/depth envelope.
    // Bound the remaining vectors/strings before cloning the actual request.
    let mut counter = Counter {
        remaining: limits.output_bytes.saturating_add(128 * 1024),
        cancellation,
    };
    serde_json::to_writer(&mut counter, request).map_err(|_| invalid())?;
    super::check(cancellation)?;
    Ok(ToolCall {
        name: invocation.tool_name.clone(),
        id: invocation.call_id.clone(),
        arguments: invocation.arguments.clone(),
    })
}

fn scalar_capability_bounds(capability: &Capability) -> bool {
    let text = |value: &str| value.len() <= 1024 * 1024;
    let strings = |values: &[String]| values.len() <= 65_536 && values.iter().all(|v| text(v));
    let network =
        |target: &machine_god_core::NetworkTarget| text(&target.scheme) && text(&target.host);
    match capability {
        Capability::Tool { .. } => true, // opaque IDs and measured JSON
        Capability::Filesystem { path, .. } | Capability::OpenFile { path } => text(path),
        Capability::FilesystemRename { old_path, new_path } => text(old_path) && text(new_path),
        Capability::FilesystemCopy {
            source,
            destination,
        } => text(source) && text(destination),
        Capability::Process {
            program,
            arguments,
            working_directory,
            environment,
            ..
        } => {
            text(program)
                && strings(arguments)
                && text(working_directory)
                && text(&environment.profile)
                && text(&environment.sha256)
        }
        Capability::Network { target } => network(target),
        Capability::Vision { paths, target } => strings(paths) && network(target),
        Capability::Custom { name, .. } => text(name),
        _ => false,
    }
}
struct Counter<'a> {
    remaining: usize,
    cancellation: &'a CancellationToken,
}
impl Write for Counter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.cancellation.is_cancelled() || bytes.len() > self.remaining {
            return Err(io::Error::other("permission input limit"));
        }
        self.remaining -= bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
