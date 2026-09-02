//! Shared JSON config-file plumbing for the integrations that do the same
//! read-modify-write dance on a tool's settings file (Claude Code's
//! `settings.json`, OpenCode's `opencode.json`, OpenClaw's `openclaw.json`).
//! One copy so the conventions - empty-file-reads-as-absent, top-level must
//! be an object, 0600 atomic writes with a trailing newline - are a single
//! decision instead of three drifting ones.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{Map, Value};

/// Load a JSON object config file. `None` when the file is missing or blank
/// (both mean "the tool has no config yet"); an error when it exists but its
/// top level is not an object - overwriting an unrecognized shape would
/// destroy user config that disconnect could never restore.
pub(crate) fn load_object(path: &Path) -> Result<Option<Map<String, Value>>> {
    let Some(raw) = read_raw(path)? else {
        return Ok(None);
    };
    let value: Value = serde_json::from_str(&raw)
        .with_context(|| format!("parsing {} as JSON", path.display()))?;
    require_object(value, path)
}

/// Like [`load_object`], parsed as JSON5 (comments + trailing commas), for
/// tools whose config format allows them (OpenClaw). Note the asymmetry the
/// caller accepts: writes go back out as plain JSON, so comments in a
/// hand-edited file do not survive a connect.
pub(crate) fn load_object_json5(path: &Path) -> Result<Option<Map<String, Value>>> {
    let Some(raw) = read_raw(path)? else {
        return Ok(None);
    };
    let value: Value =
        json5::from_str(&raw).with_context(|| format!("parsing {} as JSON5", path.display()))?;
    require_object(value, path)
}

/// Serialize and atomically write a JSON object config: pretty-printed,
/// trailing newline, 0600. 0600 defensively - none of these files holds a
/// Gate credential any more (the relay injects it per request) but they may
/// carry other user config; the atomic write keeps a crash mid-write from
/// corrupting the tool's own file.
pub(crate) fn write_object(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    let mut body = serde_json::to_string_pretty(settings)
        .with_context(|| format!("serializing {}", path.display()))?;
    body.push('\n');
    crate::primitives::write_file(path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

/// The object under `key`, inserting an empty one when absent - or when the
/// existing value is not an object, which this REPLACES. A caller that must
/// not clobber a malformed hand-edited value has to guard before calling
/// (see claude_code's `reject_non_object_env`).
pub(crate) fn ensure_object<'a>(
    parent: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    if !matches!(parent.get(key), Some(Value::Object(_))) {
        parent.insert(key.into(), Value::Object(Map::new()));
    }
    parent
        .get_mut(key)
        .and_then(|v| v.as_object_mut())
        .expect("just inserted an object")
}

fn read_raw(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(raw))
}

fn require_object(value: Value, path: &Path) -> Result<Option<Map<String, Value>>> {
    match value {
        Value::Object(m) => Ok(Some(m)),
        _ => anyhow::bail!("{} top level must be a JSON object", path.display()),
    }
}
