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
/// trailing newline, keys in the order the file already had them, and the
/// file's own mode (0600 for one Gate creates, defensively - none of these
/// files holds a Gate credential any more, the relay injects it per request,
/// but they may carry other user config). The atomic write keeps a crash
/// mid-write from corrupting the tool's own file.
pub(crate) fn write_object(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    write_object_with_view(path, settings, crate::config_changes::whole_file)
}

/// [`write_object`], with `view` taking the file's bytes to the part a running
/// process acts on ([`crate::config_changes::write_with_view`]).
pub(crate) fn write_object_with_view(
    path: &Path,
    settings: &Map<String, Value>,
    view: fn(&[u8]) -> Vec<u8>,
) -> Result<()> {
    // Equal values are not a change, however the file is laid out: a file the
    // tool itself wrote with other whitespace would otherwise be rewritten
    // just to reformat it, and that rewrite would record a change to what the
    // running tool loaded. Plain JSON first, so numbers parse as `load_object`
    // read them; JSON5 for OpenClaw's files.
    let current = read_raw(path)?.and_then(|raw| {
        serde_json::from_str::<Value>(&raw)
            .ok()
            .or_else(|| json5::from_str::<Value>(&raw).ok())
    });
    if current.as_ref().and_then(Value::as_object) == Some(settings) {
        return Ok(());
    }
    let mut settings = Value::Object(settings.clone());
    if let Some(current) = &current {
        follow_order(&mut settings, current);
    }
    let mut body = serde_json::to_string_pretty(&settings)
        .with_context(|| format!("serializing {}", path.display()))?;
    body.push('\n');
    crate::config_changes::write_with_view(path, body.as_bytes(), 0o600, view)
        .with_context(|| format!("writing {}", path.display()))
}

/// Put `value`'s keys back in the order `original` has them, at every depth,
/// with keys `original` lacks after them in their own order. Loading keeps a
/// file's order (`serde_json`'s `preserve_order`), but `Map::remove` fills the
/// gap with the last key, so a disconnect that takes Gate's marker out of
/// `settings.json` would otherwise move whatever the tool added after it into
/// its place. Arrays are followed element by element.
fn follow_order(value: &mut Value, original: &Value) {
    match (value, original) {
        (Value::Object(map), Value::Object(original)) => {
            let position = |key: &str| original.keys().position(|k| k == key);
            let mut entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            entries.sort_by_key(|(key, _)| position(key).unwrap_or(usize::MAX));
            for (key, entry) in &mut entries {
                if let Some(original) = original.get(key.as_str()) {
                    follow_order(entry, original);
                }
            }
            *map = entries.into_iter().collect();
        }
        (Value::Array(items), Value::Array(original)) => {
            for (item, original) in items.iter_mut().zip(original) {
                follow_order(item, original);
            }
        }
        _ => {}
    }
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
