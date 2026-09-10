//! Optional URL-launcher authority captured on the blocking startup owner.
//! Native code validates the capability and owns all launch/cleanup effects.

use machine_god_native::{
    MAX_TERMINAL_ENVIRONMENT_BYTES, MAX_TERMINAL_ENVIRONMENT_ENTRIES,
    MAX_TERMINAL_ENVIRONMENT_KEY_BYTES, MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES,
    NativeBackgroundUrlExecutable, NativeInteractiveSessionOptions,
};
use rustix::fs::{Mode, OFlags};
use std::ffi::OsString;
use std::fs::File;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

const MAX_PROGRAM_BYTES: usize = 4096;
const DESKTOP_PATH: &str = "/usr/bin:/bin";
const DESKTOP_KEYS: [&str; 20] = [
    "HOME",
    "USER",
    "LOGNAME",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_CONFIG_DIRS",
    "XDG_DATA_DIRS",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "TMPDIR",
    "PATH",
];

pub(super) struct Authority {
    executable: NativeBackgroundUrlExecutable,
    environment: Vec<(OsString, OsString)>,
}

pub(super) fn configure(
    options: NativeInteractiveSessionOptions,
    authority: Option<Authority>,
) -> NativeInteractiveSessionOptions {
    match authority {
        Some(authority) => {
            options.with_background_url_opener(authority.executable, authority.environment)
        }
        None => options,
    }
}

/// No process starts here. Missing/invalid optional authority leaves startup intact.
/// This runs beside clipboard capture, before entering the async session owner.
pub(super) fn capture() -> Option<Authority> {
    capture_with(platform_program(), |key| std::env::var_os(key))
}

fn platform_program() -> &'static Path {
    if cfg!(target_os = "macos") {
        Path::new("/usr/bin/open")
    } else {
        Path::new("/usr/bin/xdg-open")
    }
}

// Private injection permits fixture paths and lookups without mutating the
// process environment. Production supplies only the fixed platform path above.
fn capture_with(program: &Path, lookup: impl FnMut(&str) -> Option<OsString>) -> Option<Authority> {
    let (program, file) = resolve(program)?;
    let executable = NativeBackgroundUrlExecutable::new(program, file).ok()?;
    let environment = desktop_environment(lookup)?;
    Some(Authority {
        executable,
        environment,
    })
}

fn resolve(program: &Path) -> Option<(PathBuf, File)> {
    if !program.is_absolute()
        || program.as_os_str().len() > MAX_PROGRAM_BYTES
        || program.as_os_str().as_bytes().contains(&0)
    {
        return None;
    }
    let named = program.symlink_metadata().ok()?;
    if !named.is_file() || named.permissions().mode() & 0o111 == 0 {
        return None;
    }
    let canonical = program.canonicalize().ok()?;
    if canonical.as_os_str() != program.as_os_str()
        || canonical.as_os_str().len() > MAX_PROGRAM_BYTES
    {
        return None;
    }
    let file = File::from(
        rustix::fs::open(
            program,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .ok()?,
    );
    let retained = file.metadata().ok()?;
    if !retained.is_file()
        || retained.permissions().mode() & 0o111 == 0
        || retained.dev() != named.dev()
        || retained.ino() != named.ino()
    {
        return None;
    }
    Some((canonical, file))
}

fn desktop_environment(
    mut lookup: impl FnMut(&str) -> Option<OsString>,
) -> Option<Vec<(OsString, OsString)>> {
    let mut environment = Vec::with_capacity(DESKTOP_KEYS.len());
    let mut bytes = 0_usize;
    for key in DESKTOP_KEYS {
        let value = if key == "PATH" {
            Some(OsString::from(DESKTOP_PATH))
        } else {
            lookup(key)
        };
        let Some(value) = value else {
            continue;
        };
        // var_os necessarily returns one owned value. Check it immediately,
        // before retaining it or allocating/copying its key. Never enumerate
        // unrelated process variables; native still performs full validation.
        bytes = bytes.checked_add(key.len())?.checked_add(value.len())?;
        if environment.len() >= MAX_TERMINAL_ENVIRONMENT_ENTRIES
            || key.len() > MAX_TERMINAL_ENVIRONMENT_KEY_BYTES
            || value.len() > MAX_TERMINAL_ENVIRONMENT_VALUE_BYTES
            || bytes > MAX_TERMINAL_ENVIRONMENT_BYTES
        {
            return None;
        }
        environment.push((OsString::from(key), value));
    }
    Some(environment)
}

#[cfg(test)]
mod tests;
