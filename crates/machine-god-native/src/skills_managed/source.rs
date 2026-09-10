use super::NativeSkillManagedErrorKind as Error;
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSkillSourceKind {
    Local,
    Git,
}

#[derive(Clone, Eq, PartialEq)]
pub struct NativeSkillInstallSource {
    pub(crate) source: String,
    pub(crate) filter: Option<String>,
    pub(crate) kind: NativeSkillSourceKind,
}
impl fmt::Debug for NativeSkillInstallSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeSkillInstallSource")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}
impl NativeSkillInstallSource {
    /// Classifies before effects. Two-component relative local paths require `./`.
    /// Pasted package-manager commands are syntax, never executable instructions.
    /// # Errors
    /// Rejects malformed syntax, unsupported protocols, and conflicting filters.
    pub fn parse(input: &str, explicit_filter: Option<&str>) -> Result<Self, Error> {
        if input.len() > 4096
            || input
                .chars()
                .any(|c| c == '\0' || (c.is_control() && !c.is_ascii_whitespace()))
        {
            return Err(Error::InvalidSource);
        }
        let (source, command_filter) = pasted(input.trim())?;
        let (source, inline_filter) = normalize_inline(source)?;
        let mut filter = None;
        for candidate in [
            command_filter,
            inline_filter,
            explicit_filter.map(str::to_owned),
        ]
        .into_iter()
        .flatten()
        .filter(|value| !value.is_empty())
        {
            validate_name(&candidate)?;
            if filter.as_ref().is_some_and(|value| value != &candidate) {
                return Err(Error::ConflictingFilter);
            }
            filter = Some(candidate);
        }
        let (source, kind) = classify(source)?;
        Ok(Self {
            source,
            filter,
            kind,
        })
    }
    #[must_use]
    pub const fn kind(&self) -> NativeSkillSourceKind {
        self.kind
    }
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }
    #[must_use]
    pub fn filter(&self) -> Option<&str> {
        self.filter.as_deref()
    }
}

pub(super) fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty()
        || name.len() > 256
        || matches!(name, "." | "..")
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\'))
    {
        return Err(Error::InvalidName);
    }
    Ok(())
}
pub(super) fn validate_destination(name: &str) -> Result<(), Error> {
    validate_name(name)?;
    if name.len() > 255 || name.to_ascii_lowercase().starts_with(".machine-god-skill-") {
        return Err(Error::InvalidName);
    }
    Ok(())
}

fn pasted(input: &str) -> Result<(String, Option<String>), Error> {
    let mut words = input.split_ascii_whitespace();
    if !matches!(words.next(), Some("npx" | "bunx")) {
        return Ok((input.to_owned(), None));
    }
    let mut package = words.next();
    while matches!(package, Some("-g" | "-y" | "--yes" | "--global")) {
        package = words.next();
    }
    if package != Some("skills") || words.next() != Some("add") {
        return Err(Error::InvalidSource);
    }
    let mut source = None;
    let mut filter = None;
    while let Some(word) = words.next() {
        match word {
            "-g" | "-y" | "--yes" | "--global" => {}
            "--skill" => merge_filter(&mut filter, words.next().ok_or(Error::InvalidSource)?)?,
            value if value.starts_with("--skill=") => merge_filter(&mut filter, &value[8..])?,
            value if value.starts_with('-') => return Err(Error::InvalidSource),
            value if source.is_none() => source = Some(value.to_owned()),
            _ => return Err(Error::InvalidSource),
        }
    }
    Ok((source.ok_or(Error::InvalidSource)?, filter))
}
fn merge_filter(filter: &mut Option<String>, value: &str) -> Result<(), Error> {
    if value.is_empty() {
        return Ok(());
    }
    if filter.as_deref().is_some_and(|previous| previous != value) {
        return Err(Error::ConflictingFilter);
    }
    *filter = Some(value.to_owned());
    Ok(())
}

fn normalize_inline(source: String) -> Result<(String, Option<String>), Error> {
    let skills_url = source
        .strip_prefix("https://skills.sh/")
        .or_else(|| source.strip_prefix("http://skills.sh/"))
        .or_else(|| source.strip_prefix("skills.sh/"));
    if let Some(rest) = skills_url {
        let parts = rest.trim_end_matches('/').split('/').collect::<Vec<_>>();
        if !(2..=3).contains(&parts.len()) || !parts[..2].iter().all(|part| repo_component(part)) {
            return Err(Error::InvalidSource);
        }
        return Ok((
            format!("{}/{}", parts[0], parts[1]),
            parts.get(2).map(ToString::to_string),
        ));
    }
    if !source.starts_with(['/', '.'])
        && !source.contains("://")
        && !source.starts_with("git@")
        && let Some((repository, filter)) = source.rsplit_once('@')
        && repository.split('/').count() == 2
    {
        return Ok((repository.to_owned(), Some(filter.to_owned())));
    }
    Ok((source, None))
}
fn repo_component(value: &str) -> bool {
    !value.is_empty()
        && !matches!(value, "." | "..")
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}
fn classify(source: String) -> Result<(String, NativeSkillSourceKind), Error> {
    if source.is_empty()
        || source.len() > 4096
        || source.contains('\0')
        || (source.contains("::") && !source.contains("://") && !source.starts_with("git@"))
        || source.starts_with('-')
    {
        return Err(Error::InvalidSource);
    }
    if let Some(remote) = source.strip_prefix("git@") {
        let Some((host, path)) = remote.split_once(':') else {
            return Err(Error::InvalidSource);
        };
        if host.is_empty()
            || path.is_empty()
            || source.chars().any(char::is_whitespace)
            || path.starts_with('-')
        {
            return Err(Error::InvalidSource);
        }
        return Ok((source, NativeSkillSourceKind::Git));
    }
    if source.contains("://") {
        let parsed = url::Url::parse(&source).map_err(|_| Error::InvalidSource)?;
        if !matches!(parsed.scheme(), "http" | "https" | "ssh")
            || parsed.host_str().is_none()
            || parsed.fragment().is_some()
            || parsed.password().is_some()
            || source.chars().any(char::is_whitespace)
        {
            return Err(Error::InvalidSource);
        }
        return Ok((source, NativeSkillSourceKind::Git));
    }
    let parts = source.split('/').collect::<Vec<_>>();
    if parts.len() == 2 && parts.iter().all(|part| repo_component(part)) {
        return Ok((
            format!("https://github.com/{source}.git"),
            NativeSkillSourceKind::Git,
        ));
    }
    Ok((source, NativeSkillSourceKind::Local))
}

/// Parses management flags without invoking a package manager or shell.
/// # Errors
/// Rejects ambiguous operands, repeated conflicting filters, or unknown options.
pub fn parse_skill_install_command(
    arguments: &str,
) -> Result<(NativeSkillInstallSource, bool), Error> {
    if arguments.len() > 8192 {
        return Err(Error::ResourceLimit);
    }
    let mut source = Vec::new();
    let mut filter = None;
    let mut replace = false;
    let mut words = arguments.split_ascii_whitespace();
    while let Some(word) = words.next() {
        match word {
            "--replace" => replace = true,
            "--skill" => merge_filter(&mut filter, words.next().ok_or(Error::InvalidSource)?)?,
            value if value.starts_with("--skill=") => merge_filter(&mut filter, &value[8..])?,
            value => source.push(value),
        }
    }
    let source = source.join(" ");
    if !source.starts_with("npx ")
        && !source.starts_with("bunx ")
        && source.split_ascii_whitespace().count() != 1
    {
        return Err(Error::InvalidSource);
    }
    NativeSkillInstallSource::parse(&source, filter.as_deref()).map(|source| (source, replace))
}

/// Parses a native managed name and explicit replacement consent marker.
/// # Errors
/// Rejects invalid names or repeated/embedded command flags.
pub fn parse_skill_create_command(arguments: &str) -> Result<(String, bool), Error> {
    if arguments.len() > 512 {
        return Err(Error::ResourceLimit);
    }
    let mut argument = arguments.trim();
    let mut replace = false;
    if let Some(rest) = argument.strip_prefix("--replace ") {
        replace = true;
        argument = rest.trim();
    }
    if let Some(rest) = argument.strip_suffix(" --replace") {
        replace = true;
        argument = rest.trim();
    }
    if argument.starts_with("--") || argument.contains(" --") {
        return Err(Error::InvalidName);
    }
    validate_destination(argument)?;
    Ok((argument.to_owned(), replace))
}
