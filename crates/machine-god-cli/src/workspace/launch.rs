//! Bounded, effect-free parsing of leading launch-only workspace modifiers.

use std::{ffi::OsString, iter::Peekable, path::PathBuf};

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct LaunchWorkspaceOptions {
    pub(crate) directories: Vec<PathBuf>,
    pub(crate) suppress_saved: bool,
}

impl LaunchWorkspaceOptions {
    pub(crate) const EMPTY: Self = Self {
        directories: Vec::new(),
        suppress_saved: false,
    };

    pub(crate) fn selected(&self) -> bool {
        self.suppress_saved || !self.directories.is_empty()
    }

    pub(crate) fn parse(
        arguments: &mut Peekable<impl Iterator<Item = OsString>>,
    ) -> Result<Self, ()> {
        let mut options = Self::EMPTY;
        while let Some(argument) = arguments.peek() {
            if argument == "--no-additional-dirs" {
                if options.suppress_saved {
                    return Err(());
                }
                options.suppress_saved = true;
                arguments.next();
                continue;
            }
            let path = if argument == "--add-dir" {
                arguments.next();
                arguments.next().ok_or(())?
            } else if argument.as_encoded_bytes().starts_with(b"--add-dir=") {
                if argument.as_encoded_bytes().len() > b"--add-dir=".len() + super::MAX_PATH_BYTES {
                    return Err(());
                }
                let argument = arguments.next().ok_or(())?;
                inline_path(argument)?
            } else {
                break;
            };
            let bytes = path.as_encoded_bytes();
            if bytes.is_empty()
                || bytes.len() > super::MAX_PATH_BYTES
                || bytes.contains(&0)
                || options.directories.len() == 64
            {
                return Err(());
            }
            options.directories.push(PathBuf::from(path));
        }
        Ok(options)
    }
}

fn inline_path(argument: OsString) -> Result<OsString, ()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        let bytes = argument.into_vec();
        let path = bytes.strip_prefix(b"--add-dir=").ok_or(())?;
        Ok(OsString::from_vec(path.to_vec()))
    }
    #[cfg(not(unix))]
    {
        Ok(OsString::from(
            &argument.into_string().map_err(|_| ())?["--add-dir=".len()..],
        ))
    }
}
