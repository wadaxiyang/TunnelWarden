use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use thiserror::Error;
use tunnel_domain::{AuthConfig, HostId, HostKeyPolicy, SshHost};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_FILES: usize = 32;
const MAX_ENTRIES: usize = 512;
const MAX_WARNINGS: usize = 64;

#[derive(Clone, Debug)]
pub struct SshImportEntry {
    pub host: SshHost,
    pub alias: String,
    pub proxy_jump: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SshImportPreview {
    pub entries: Vec<SshImportEntry>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Error)]
pub enum SshImportError {
    #[error("SSH config read failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SSH config exceeds import limits")]
    TooLarge,
    #[error("SSH config is not valid UTF-8")]
    Utf8(#[from] std::string::FromUtf8Error),
}

#[derive(Default)]
struct Limits {
    bytes: usize,
    files: usize,
    visited: HashSet<PathBuf>,
    warnings: Vec<String>,
}

impl Limits {
    fn warn(&mut self, message: String) {
        if self.warnings.len() < MAX_WARNINGS {
            self.warnings.push(message);
        }
    }
}

#[derive(Default)]
struct Block {
    patterns: Vec<String>,
    values: Vec<(String, String)>,
}

pub fn preview_ssh_config(path: &Path) -> Result<SshImportPreview, SshImportError> {
    let mut lines = Vec::new();
    let mut limits = Limits::default();
    read_lines(path, &mut limits, &mut lines)?;
    let mut blocks = vec![Block {
        patterns: vec!["*".into()],
        values: Vec::new(),
    }];
    let mut aliases = Vec::new();
    for (line_number, line) in lines.iter().enumerate() {
        let words = split_words(line);
        if words.is_empty() {
            continue;
        }
        let key = words[0].to_ascii_lowercase();
        if key == "host" {
            if words.len() < 2 {
                limits.warn(format!("Line {}: empty Host directive", line_number + 1));
                continue;
            }
            let patterns = words[1..].to_vec();
            for pattern in &patterns {
                if !pattern.starts_with('!')
                    && !pattern.contains(['*', '?'])
                    && aliases.len() < MAX_ENTRIES
                    && !aliases.contains(pattern)
                {
                    aliases.push(pattern.clone());
                }
            }
            blocks.push(Block {
                patterns,
                values: Vec::new(),
            });
        } else if key == "match" {
            limits.warn(format!(
                "Line {}: Match blocks are unsupported",
                line_number + 1
            ));
            blocks.push(Block::default());
        } else if let Some(value) = words.get(1)
            && let Some(block) = blocks.last_mut()
        {
            block.values.push((key, value.clone()));
        }
    }
    if aliases.len() == MAX_ENTRIES {
        limits.warn("Only the first 512 explicit hosts are previewed".into());
    }
    let default_user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let mut entries = Vec::with_capacity(aliases.len());
    for alias in aliases {
        let mut values = std::collections::HashMap::<&str, &str>::new();
        let mut warnings = Vec::new();
        for block in &blocks {
            if !matches_alias(&block.patterns, &alias) {
                continue;
            }
            for (key, value) in &block.values {
                match key.as_str() {
                    "hostname"
                    | "user"
                    | "port"
                    | "identityfile"
                    | "identitiesonly"
                    | "proxyjump"
                    | "connecttimeout"
                    | "serveraliveinterval"
                    | "serveralivecountmax"
                    | "hostkeyalias"
                    | "userknownhostsfile" => {
                        values.entry(key).or_insert(value);
                    }
                    _ => {
                        if warnings.len() < 16 {
                            warnings.push(format!("Unsupported: {key}"));
                        }
                    }
                }
            }
        }
        let hostname = values
            .get("hostname")
            .copied()
            .unwrap_or(&alias)
            .replace("%h", &alias);
        let username = values
            .get("user")
            .copied()
            .unwrap_or(&default_user)
            .to_owned();
        let port = values
            .get("port")
            .and_then(|value| value.parse::<u16>().ok())
            .filter(|port| *port > 0)
            .unwrap_or(22);
        if values.contains_key("port") && port == 22 && values["port"] != "22" {
            warnings.push("Invalid port; using 22".into());
        }
        let proxy_jump = values.get("proxyjump").map(|value| (*value).to_owned());
        if proxy_jump.is_some() {
            warnings
                .push("ProxyJump requires a tunnel Jump Chain; not applied to this jumper".into());
        }
        for unsupported in ["hostkeyalias", "userknownhostsfile"] {
            if values.contains_key(unsupported) {
                warnings.push(format!(
                    "{unsupported} is not mapped; verify host key settings before use"
                ));
            }
        }
        if values.get("identitiesonly") == Some(&"yes") && !values.contains_key("identityfile") {
            warnings.push("IdentitiesOnly yes has no IdentityFile".into());
        }
        let auth = if let Some(path) = values.get("identityfile") {
            let key_path = path
                .strip_prefix("~/")
                .map_or_else(|| PathBuf::from(path), |rest| home_dir().join(rest));
            AuthConfig::PrivateKey {
                key_path,
                passphrase_ref: None,
            }
        } else {
            AuthConfig::Agent { socket: None }
        };
        let seconds = |key: &str, default: u64| {
            values
                .get(key)
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(default)
        };
        let host = SshHost {
            id: HostId(format!("ssh-{alias}")),
            name: alias.clone(),
            hostname,
            port,
            username,
            auth,
            host_key_policy: HostKeyPolicy::Strict,
            connect_timeout: Duration::from_secs(seconds("connecttimeout", 10).max(1)),
            keepalive_interval: Duration::from_secs(seconds("serveraliveinterval", 30).max(1)),
            keepalive_max: seconds("serveralivecountmax", 3).clamp(1, u32::MAX as u64) as u32,
            notes: String::new(),
        };
        if host.username.is_empty() {
            warnings.push("User is empty; choose a username before importing".into());
        }
        entries.push(SshImportEntry {
            host,
            alias,
            proxy_jump,
            warnings,
        });
    }
    Ok(SshImportPreview {
        entries,
        warnings: limits.warnings,
    })
}

fn matches_alias(patterns: &[String], alias: &str) -> bool {
    let mut positive = false;
    for pattern in patterns {
        let excluded = pattern.starts_with('!');
        let pattern = pattern.strip_prefix('!').unwrap_or(pattern);
        let matches = glob::Pattern::new(pattern).is_ok_and(|pattern| pattern.matches(alias));
        if matches && excluded {
            return false;
        }
        if matches {
            positive = true;
        }
    }
    positive
}

fn read_lines(
    path: &Path,
    limits: &mut Limits,
    lines: &mut Vec<String>,
) -> Result<(), SshImportError> {
    let path = path.canonicalize()?;
    if !limits.visited.insert(path.clone()) {
        return Ok(());
    }
    limits.files += 1;
    if limits.files > MAX_FILES {
        return Err(SshImportError::TooLarge);
    }
    let metadata = fs::metadata(&path)?;
    let remaining = MAX_BYTES.saturating_sub(limits.bytes);
    if metadata.len() > remaining as u64 {
        return Err(SshImportError::TooLarge);
    }
    let bytes = fs::read(&path)?;
    if bytes.len() > remaining {
        return Err(SshImportError::TooLarge);
    }
    limits.bytes += bytes.len();
    let content = String::from_utf8(bytes)?;
    for line in content.lines() {
        let words = split_words(line);
        if words
            .first()
            .is_some_and(|word| word.eq_ignore_ascii_case("include"))
        {
            for include in words.iter().skip(1) {
                let include = if let Some(rest) = include.strip_prefix("~/") {
                    home_dir().join(rest)
                } else {
                    let include = PathBuf::from(include);
                    if include.is_absolute() {
                        include
                    } else {
                        path.parent().unwrap_or(Path::new(".")).join(include)
                    }
                };
                let pattern = include.to_string_lossy();
                match glob::glob(&pattern) {
                    Ok(paths) => {
                        for result in paths.take(MAX_FILES + 1) {
                            if limits.files >= MAX_FILES {
                                limits.warn("Include file limit reached".into());
                                break;
                            }
                            match result {
                                Ok(child) => {
                                    if let Err(error) = read_lines(&child, limits, lines) {
                                        limits
                                            .warn(format!("Include {}: {error}", child.display()));
                                    }
                                }
                                Err(error) => limits.warn(format!("Include: {error}")),
                            }
                        }
                    }
                    Err(error) => limits.warn(format!("Include {pattern}: {error}")),
                }
            }
        } else {
            lines.push(line.to_owned());
        }
    }
    Ok(())
}

fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn split_words(line: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for ch in line.chars() {
        if quote.is_none() && ch == '#' {
            break;
        }
        if ch == '\'' || ch == '"' {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            } else {
                word.push(ch);
            }
        } else if ch.is_whitespace() && quote.is_none() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    if let Some(first) = words.first_mut() {
        if let Some((key, value)) = first.split_once('=') {
            let key = key.to_owned();
            let value = value.to_owned();
            *first = key;
            words.insert(1, value);
        } else if words.get(1).is_some_and(|value| value == "=") {
            words.remove(1);
        }
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_includes_first_wins_and_warns_about_unmapped_proxy_jump() {
        let directory = tempfile::tempdir().expect("tempdir");
        fs::write(
            directory.path().join("extra"),
            "Host bastion\n HostName 1.2.3.4\n User ops\n",
        )
        .expect("extra");
        fs::write(directory.path().join("config"), "Include extra\nHost lab\n HostName 10.0.0.2\n User alice\n ProxyJump bastion\n BadDirective yes\nHost *\n Port 2222\n User default\n").expect("config");
        let preview = preview_ssh_config(&directory.path().join("config")).expect("preview");
        assert_eq!(preview.entries.len(), 2);
        let lab = preview
            .entries
            .iter()
            .find(|entry| entry.alias == "lab")
            .expect("lab");
        assert_eq!(lab.host.username, "alice");
        assert_eq!(lab.host.port, 2222);
        assert_eq!(lab.proxy_jump.as_deref(), Some("bastion"));
        assert!(
            lab.warnings
                .iter()
                .any(|warning| warning.contains("ProxyJump"))
        );
        assert!(
            lab.warnings
                .iter()
                .any(|warning| warning.contains("baddirective"))
        );
    }
}
