use std::ffi::{OsStr, OsString};

use crate::NativeEnvironment;

pub(crate) trait StateEnvironmentReader {
    fn read(&mut self, name: &'static str) -> Option<OsString>;
}

pub(crate) struct ProcessStateEnvironmentReader;

impl StateEnvironmentReader for ProcessStateEnvironmentReader {
    fn read(&mut self, name: &'static str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

pub(crate) fn capture_state_environment(
    reader: &mut impl StateEnvironmentReader,
) -> NativeEnvironment {
    let xdg_state_home = reader.read("XDG_STATE_HOME");
    let home = if xdg_state_home.as_deref().is_none_or(OsStr::is_empty) {
        reader.read("HOME")
    } else {
        None
    };
    NativeEnvironment::new(None, xdg_state_home, home)
}
