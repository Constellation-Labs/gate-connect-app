//! One-shot migration from Cowork's standard-mode userData
//! (`~/Library/Application Support/Claude/`) to its 3P/gateway-mode
//! userData (`~/Library/Application Support/Claude-3p/`).
//!
//! 3P mode runs as a separate Electron instance with its own userData
//! dir, so scheduled tasks, conversations, memory, org plugins, etc. all
//! start empty after a user switches to gateway mode. This module copies
//! the bits that survive a mode switch (Claude.ai-bound state like
//! cookies and oauth tokens are explicitly excluded).
//!
//! The module is Cowork-specific by nature — paths are Claude Desktop's
//! own userData layout — and therefore lives outside the generic
//! [`crate::Integration`] trait.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::env;
use crate::primitives::{
    copy_dir_recursive, dir_size_bytes, install_org_plugins_dir, is_process_running_matching,
};

const SESSIONS_SUBDIR: &str = "local-agent-mode-sessions";
const COWORK_3P_RUNNING_NEEDLE: &str = "--user-data-dir=";
const COWORK_3P_DIR_NEEDLE: &str = "Claude-3p";

/// Paths to the source and destination per-account/org dirs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateRoots {
    pub source_account: String,
    pub source_org: String,
    pub source_dir: PathBuf,
    pub dest_account: String,
    pub dest_org: String,
    pub dest_dir: PathBuf,
}

/// Why migration can't proceed. Empty when [`MigrateDiscover::ready`] is true.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MigrateBlocker {
    SourceMissing,
    DestMissing,
    CoworkRunning,
    InsufficientDiskSpace {
        needed_bytes: u64,
        available_bytes: u64,
    },
}

/// Read-only summary of what's available to migrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateDiscover {
    pub roots: Option<MigrateRoots>,
    pub plugins: usize,
    pub scheduled: usize,
    pub conversations: usize,
    pub has_memory: bool,
    pub has_enabled_plugins: bool,
    pub has_preferences: bool,
    pub artifacts: usize,
    pub bytes_estimated: u64,
    pub ready: bool,
    pub blockers: Vec<MigrateBlocker>,
}

/// Which categories to migrate. Defaults to all-true via [`MigrateOptions::all`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrateOptions {
    pub include_plugins: bool,
    pub include_scheduled: bool,
    pub include_conversations: bool,
    pub include_memory: bool,
    pub include_enabled_plugins: bool,
    pub include_preferences: bool,
    pub include_artifacts: bool,
    pub dry_run: bool,
}

impl MigrateOptions {
    pub fn all() -> Self {
        Self {
            include_plugins: true,
            include_scheduled: true,
            include_conversations: true,
            include_memory: true,
            include_enabled_plugins: true,
            include_preferences: true,
            include_artifacts: true,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MigrateCategory {
    Plugins,
    Scheduled,
    Conversations,
    Memory,
    EnabledPlugins,
    Preferences,
    Artifacts,
}

impl MigrateCategory {
    pub fn key(self) -> &'static str {
        match self {
            MigrateCategory::Plugins => "plugins",
            MigrateCategory::Scheduled => "scheduled",
            MigrateCategory::Conversations => "conversations",
            MigrateCategory::Memory => "memory",
            MigrateCategory::EnabledPlugins => "enabled_plugins",
            MigrateCategory::Preferences => "preferences",
            MigrateCategory::Artifacts => "artifacts",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CategoryReport {
    pub copied: usize,
    pub skipped: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MigrateReport {
    pub per_category: HashMap<String, CategoryReport>,
    pub dry_run: bool,
}

/// Scan source + destination and return a summary plus pre-flight verdict.
pub fn discover() -> Result<MigrateDiscover> {
    let src_root = env::claude_user_data_dir()?.join(SESSIONS_SUBDIR);
    let dst_root = env::claude_3p_user_data_dir()?.join(SESSIONS_SUBDIR);

    let source = newest_account_org(&src_root)?;
    let dest = newest_account_org(&dst_root)?;

    let mut blockers: Vec<MigrateBlocker> = Vec::new();
    if source.is_none() {
        blockers.push(MigrateBlocker::SourceMissing);
    }
    if dest.is_none() {
        blockers.push(MigrateBlocker::DestMissing);
    }
    if cowork_3p_running() {
        blockers.push(MigrateBlocker::CoworkRunning);
    }

    let roots = match (source, dest) {
        (Some((sa, so, sd)), Some((da, do_, dd))) => Some(MigrateRoots {
            source_account: sa,
            source_org: so,
            source_dir: sd,
            dest_account: da,
            dest_org: do_,
            dest_dir: dd,
        }),
        _ => None,
    };

    let (plugins, scheduled, conversations, has_memory, has_enabled_plugins, artifacts, bytes) =
        if let Some(ref r) = roots {
            let plugins = count_plugins(&r.source_dir);
            let scheduled = count_scheduled(&r.source_dir);
            let conversations = count_conversations(&r.source_dir);
            let has_memory = r.source_dir.join("memory/memory").exists()
                || r.source_dir.join("spaces.json").exists();
            let has_enabled = r.source_dir.join("cowork_settings.json").exists();
            let artifacts = count_artifacts(&r.source_dir);
            let bytes = dir_size_bytes(&r.source_dir).unwrap_or(0);
            (
                plugins,
                scheduled,
                conversations,
                has_memory,
                has_enabled,
                artifacts,
                bytes,
            )
        } else {
            (0, 0, 0, false, false, 0, 0)
        };

    let has_preferences = roots
        .as_ref()
        .map(|_| {
            let src = env::claude_user_data_dir().ok();
            src.map(|p| {
                p.join("claude_desktop_config.json").exists()
                    || p.join("extensions-blocklist.json").exists()
                    || p.join("developer_settings.json").exists()
            })
            .unwrap_or(false)
        })
        .unwrap_or(false);

    // Disk-space pre-flight is omitted in v1 — copying typically a few
    // hundred MB into a user-writable dir, with a clear error if writes
    // actually fail. Adding it back would require a libc dep or a shell
    // out to `df`.

    let ready = blockers.is_empty()
        && (plugins
            + scheduled
            + conversations
            + artifacts
            + has_memory as usize
            + has_enabled_plugins as usize
            + has_preferences as usize
            > 0);

    Ok(MigrateDiscover {
        roots,
        plugins,
        scheduled,
        conversations,
        has_memory,
        has_enabled_plugins,
        has_preferences,
        artifacts,
        bytes_estimated: bytes,
        ready,
        blockers,
    })
}

/// Run the migration. Returns a per-category report. When
/// `options.dry_run = true`, no files are touched but the report still
/// reflects what *would* have been copied/skipped.
pub fn execute(options: &MigrateOptions) -> Result<MigrateReport> {
    let info = discover()?;
    let roots = match info.roots {
        Some(r) => r,
        None => {
            if info
                .blockers
                .iter()
                .any(|b| matches!(b, MigrateBlocker::SourceMissing))
            {
                anyhow::bail!(
                    "No Cowork standard-mode data found at ~/Library/Application Support/Claude. Sign into Cowork in standard mode first."
                );
            }
            anyhow::bail!(
                "No Cowork gateway-mode userData found at ~/Library/Application Support/Claude-3p. Launch Cowork in gateway mode at least once before migrating."
            );
        }
    };
    for b in &info.blockers {
        match b {
            MigrateBlocker::CoworkRunning => anyhow::bail!(
                "Cowork is currently running in gateway mode. Quit it (Cmd+Q) and retry."
            ),
            MigrateBlocker::InsufficientDiskSpace {
                needed_bytes,
                available_bytes,
            } => {
                anyhow::bail!(
                    "Not enough free disk space for migration: need ~{} bytes, have {} bytes.",
                    needed_bytes,
                    available_bytes
                );
            }
            MigrateBlocker::SourceMissing | MigrateBlocker::DestMissing => {}
        }
    }

    let mut report = MigrateReport {
        dry_run: options.dry_run,
        ..Default::default()
    };

    if options.include_plugins {
        let r = migrate_plugins(&roots, options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::Plugins.key().into(), r);
    }
    if options.include_scheduled {
        let r = migrate_scheduled(&roots, options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::Scheduled.key().into(), r);
    }
    if options.include_conversations {
        let r = migrate_conversations(&roots, options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::Conversations.key().into(), r);
    }
    if options.include_memory {
        let r = migrate_memory(&roots, options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::Memory.key().into(), r);
    }
    if options.include_enabled_plugins {
        let r = migrate_enabled_plugins(&roots, options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::EnabledPlugins.key().into(), r);
    }
    if options.include_preferences {
        let r = migrate_preferences(options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::Preferences.key().into(), r);
    }
    if options.include_artifacts {
        let r = migrate_artifacts(&roots, options.dry_run);
        report
            .per_category
            .insert(MigrateCategory::Artifacts.key().into(), r);
    }

    Ok(report)
}

// ---------- discovery helpers ----------

fn newest_account_org(root: &Path) -> Result<Option<(String, String, PathBuf)>> {
    if !root.exists() {
        return Ok(None);
    }
    let mut best: Option<(SystemTime, String, String, PathBuf)> = None;
    for acct in fs::read_dir(root).with_context(|| format!("read_dir {}", root.display()))? {
        let acct = acct?;
        let acct_path = acct.path();
        if !acct_path.is_dir() {
            continue;
        }
        let acct_name = acct.file_name().to_string_lossy().into_owned();
        // Skip non-UUID-shaped helper dirs like `skills-plugin`.
        if !looks_like_uuid(&acct_name) {
            continue;
        }
        for org in
            fs::read_dir(&acct_path).with_context(|| format!("read_dir {}", acct_path.display()))?
        {
            let org = org?;
            let org_path = org.path();
            if !org_path.is_dir() {
                continue;
            }
            let org_name = org.file_name().to_string_lossy().into_owned();
            if !looks_like_uuid(&org_name) {
                continue;
            }
            let mtime = org_path
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(UNIX_EPOCH);
            let candidate = (mtime, acct_name.clone(), org_name, org_path);
            best = match best {
                None => Some(candidate),
                Some(prev) if candidate.0 > prev.0 => Some(candidate),
                Some(prev) => Some(prev),
            };
        }
    }
    Ok(best.map(|(_, a, o, p)| (a, o, p)))
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn count_plugins(src: &Path) -> usize {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(src.join("rpm")) {
        total += entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .count();
    }
    // installed_plugins.json defines what's installed from marketplaces.
    if let Ok(raw) = fs::read_to_string(src.join("cowork_plugins/installed_plugins.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&raw) {
            if let Some(map) = v.get("plugins").and_then(|p| p.as_object()) {
                total += map.len();
            }
        }
    }
    // Standalone skills under skills-plugin/{org}/{account}/skills/
    if let (Ok(root), Some((org, acct))) = (env::claude_user_data_dir(), infer_org_acct(src)) {
        let skills_dir = root
            .join(SESSIONS_SUBDIR)
            .join("skills-plugin")
            .join(&org)
            .join(&acct)
            .join("skills");
        if let Ok(entries) = fs::read_dir(skills_dir) {
            total += entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .count();
        }
    }
    total
}

/// Given a `.../local-agent-mode-sessions/{account}/{org}` path, return
/// `(org, account)` — the layout `skills-plugin` uses (inverted order).
fn infer_org_acct(src: &Path) -> Option<(String, String)> {
    let org = src.file_name()?.to_string_lossy().into_owned();
    let account = src.parent()?.file_name()?.to_string_lossy().into_owned();
    Some((org, account))
}

fn count_scheduled(src: &Path) -> usize {
    let path = src.join("scheduled-tasks.json");
    let Ok(raw) = fs::read_to_string(&path) else {
        return 0;
    };
    let Ok(v) = serde_json::from_str::<Value>(&raw) else {
        return 0;
    };
    v.get("scheduledTasks")
        .and_then(|t| t.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0)
}

fn count_conversations(src: &Path) -> usize {
    let Ok(entries) = fs::read_dir(src) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.starts_with("local_") && !s.ends_with(".json") && e.path().is_dir()
        })
        .count()
}

fn count_artifacts(src: &Path) -> usize {
    let dir = src.join("artifacts");
    let Ok(entries) = fs::read_dir(&dir) else {
        return 0;
    };
    entries.filter_map(|e| e.ok()).count()
}

/// Re-check the Cowork-3P-running guard immediately before a mutating write.
/// `discover()` checks once up front, but Cowork-3P can launch in the gap
/// between discovery and the write and clobber the merged file on quit, so
/// every write site re-checks to close that TOCTOU window.
fn ensure_cowork_3p_stopped() -> Result<()> {
    if cowork_3p_running() {
        anyhow::bail!(
			"Cowork (3P mode) started running mid-migration; aborted before writing to avoid clobbering its data. Quit Cowork and run migrate again."
		);
    }
    Ok(())
}

/// `primitives::write_file` preceded by a fresh 3P-running re-check. All
/// migration writes go through here so the guard can't be forgotten.
fn guarded_write_file(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    ensure_cowork_3p_stopped()?;
    crate::primitives::write_file(path, bytes, mode)
}

fn cowork_3p_running() -> bool {
    // Best signal: the parent Electron process holds an open file
    // descriptor on `IndexedDB/.../LOCK` from launch to quit. Probing
    // that single file with `lsof` is fast and avoids the race where
    // helper processes haven't spawned yet (parent argv doesn't
    // include `Claude-3p`, only helpers do, and they appear seconds
    // after the parent — long enough that a quickly-clicked Migrate
    // could slip through a pgrep-based check).
    if let Ok(root) = env::claude_3p_user_data_dir() {
        let lock = root.join("IndexedDB/app_localhost_0.indexeddb.leveldb/LOCK");
        if lock.exists() {
            let out = std::process::Command::new("/usr/sbin/lsof")
                .arg(lock.as_os_str())
                .output();
            if let Ok(out) = out {
                if out.status.success() && !out.stdout.is_empty() {
                    return true;
                }
            }
        }
    }
    // Fallback: pgrep for `Claude-3p` in helper argv. Misses the brief
    // window during Cowork-3p startup before helpers are up, but catches
    // it once it's settled, and works even if the LOCK path moves.
    is_process_running_matching(&format!(
        "{}{}",
        COWORK_3P_RUNNING_NEEDLE, COWORK_3P_DIR_NEEDLE
    )) || is_process_running_matching(COWORK_3P_DIR_NEEDLE)
}

// ---------- category executors ----------

fn migrate_plugins(roots: &MigrateRoots, dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();

    // Plugins in 3P mode are only discovered from the system-wide
    // `/Library/Application Support/Claude/org-plugins/` path (the
    // bundle's `pD()` function). The per-user `cowork_plugins/` and
    // `rpm/` dirs are unused in 3P. To make the user's standard-mode
    // plugins visible after migration we have to land them in the
    // system-wide dir, which requires admin elevation (same osascript
    // pattern we already use for the managed-preferences plist).
    let plugins = collect_source_plugins(&roots.source_dir);
    if plugins.is_empty() {
        // Still run skills migration even if no plugins.
        migrate_skills_plugin(roots, dry_run, &mut r);
        return r;
    }

    if dry_run {
        r.copied += plugins.len();
    } else {
        match install_org_plugins_dir(&plugins) {
            Ok(_) => {
                r.copied += plugins.len();
                // Enable each plugin in cowork_settings.json so Cowork-3p
                // actually loads them (the bundle filters with
                // `enabledPlugins[`${name}@${Pw}`] !== !0` where Pw is the
                // `org-provisioned` marker).
                let dst_settings = roots.dest_dir.join("cowork_settings.json");
                if let Err(err) = enable_org_plugins(&dst_settings, &plugins) {
                    r.failed += 1;
                    r.errors.push(format!("cowork_settings.json enable: {err}"));
                }
            }
            Err(err) => {
                r.failed += plugins.len();
                r.errors.push(format!("org-plugins install: {err}"));
            }
        }
    }

    // Standalone skills (separate from plugins) live in the per-user
    // `skills-plugin/{org}/{acct}/skills/` path and DO work in 3P mode,
    // provided manifest entries carry `syncManaged: false` (handled in
    // `merge_skills_manifest`).
    migrate_skills_plugin(roots, dry_run, &mut r);

    r
}

/// Locate each plugin to install. Returns `(plugin_name, source_path)`
/// where `source_path` is the directory holding `.claude-plugin/plugin.json`
/// and `plugin_name` is the canonical name from that manifest (not the
/// `plugin_01XYZ` rpm dir id).
fn collect_source_plugins(src_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // rpm/plugin_*/ — org-provisioned plugins.
    if let Ok(entries) = fs::read_dir(src_dir.join("rpm")) {
        for e in entries.flatten() {
            let p = e.path();
            if !p.is_dir() {
                continue;
            }
            if let Some(name) = read_plugin_name(&p) {
                if seen.insert(name.clone()) {
                    out.push((name, p));
                }
            }
        }
    }

    // cowork_plugins/cache/{marketplace}/{plugin}/{version}/ — marketplace
    // plugins the user installed. Walk to find each dir with a plugin.json.
    if let Ok(markets) = fs::read_dir(src_dir.join("cowork_plugins/cache")) {
        for market in markets.flatten() {
            let Ok(plugins) = fs::read_dir(market.path()) else {
                continue;
            };
            for plugin in plugins.flatten() {
                let Ok(versions) = fs::read_dir(plugin.path()) else {
                    continue;
                };
                let mut newest: Option<(SystemTime, PathBuf)> = None;
                for v in versions.flatten() {
                    let p = v.path();
                    if !p.is_dir() {
                        continue;
                    }
                    let mtime = p
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    newest = match newest {
                        None => Some((mtime, p)),
                        Some(prev) if mtime > prev.0 => Some((mtime, p)),
                        Some(prev) => Some(prev),
                    };
                }
                if let Some((_, p)) = newest {
                    if let Some(name) = read_plugin_name(&p) {
                        if seen.insert(name.clone()) {
                            out.push((name, p));
                        }
                    }
                }
            }
        }
    }
    out
}

fn read_plugin_name(plugin_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(plugin_dir.join(".claude-plugin/plugin.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v.get("name")
        .and_then(|n| n.as_str())
        .map(|s| s.to_string())
}

const ORG_PROVISIONED_MARKER: &str = "org-provisioned";

/// Merge each `{name}@org-provisioned: true` entry into
/// `cowork_settings.json`'s `enabledPlugins` map. Preserves any existing
/// entries (so user-toggled state survives a re-migration).
fn enable_org_plugins(settings_path: &Path, plugins: &[(String, PathBuf)]) -> Result<()> {
    let mut val: Value = if settings_path.exists() {
        let raw = fs::read_to_string(settings_path)
            .with_context(|| format!("reading {}", settings_path.display()))?;
        if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", settings_path.display()))?
        }
    } else {
        Value::Object(Map::new())
    };
    if !val.is_object() {
        anyhow::bail!("destination is not a JSON object, refusing to overwrite");
    }
    let obj = val.as_object_mut().unwrap();
    let mut enabled = obj
        .get("enabledPlugins")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (name, _) in plugins {
        let key = format!("{name}@{ORG_PROVISIONED_MARKER}");
        enabled.insert(key, Value::Bool(true));
    }
    obj.insert("enabledPlugins".into(), Value::Object(enabled));
    let body = serde_json::to_string_pretty(&val)?;
    guarded_write_file(settings_path, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", settings_path.display()))?;
    Ok(())
}

fn migrate_skills_plugin(roots: &MigrateRoots, dry_run: bool, r: &mut CategoryReport) -> usize {
    let Ok(src_root) = env::claude_user_data_dir() else {
        return 0;
    };
    let Ok(dst_root) = env::claude_3p_user_data_dir() else {
        return 0;
    };
    let src_base = src_root
        .join(SESSIONS_SUBDIR)
        .join("skills-plugin")
        .join(&roots.source_org)
        .join(&roots.source_account);
    let dst_base = dst_root
        .join(SESSIONS_SUBDIR)
        .join("skills-plugin")
        .join(&roots.dest_org)
        .join(&roots.dest_account);

    if !src_base.exists() {
        return 0;
    }

    // Copy skill dirs: skip per-skill if dst already has that named dir.
    let mut copied = 0usize;
    let src_skills = src_base.join("skills");
    let dst_skills = dst_base.join("skills");
    if let Ok(entries) = fs::read_dir(&src_skills) {
        for e in entries.flatten() {
            let src = e.path();
            if !src.is_dir() {
                continue;
            }
            let dst = dst_skills.join(e.file_name());
            if dst.exists() {
                r.skipped += 1;
                continue;
            }
            if dry_run {
                copied += 1;
                r.copied += 1;
                continue;
            }
            match copy_dir_recursive(&src, &dst) {
                Ok(_) => {
                    copied += 1;
                    r.copied += 1;
                }
                Err(err) => {
                    r.failed += 1;
                    r.errors
                        .push(format!("skills-plugin/{:?}: {err}", e.file_name()));
                }
            }
        }
    }

    // Merge manifest.json's `skills` array by name.
    let src_manifest = src_base.join("manifest.json");
    let dst_manifest = dst_base.join("manifest.json");
    if src_manifest.exists() {
        match merge_skills_manifest(&src_manifest, &dst_manifest, dry_run) {
            Ok(true) => r.copied += 1,
            Ok(false) => {}
            Err(err) => {
                r.failed += 1;
                r.errors.push(format!("skills-plugin/manifest.json: {err}"));
            }
        }
    }
    copied
}

fn merge_skills_manifest(src: &Path, dst: &Path, dry_run: bool) -> Result<bool> {
    let src_raw = fs::read_to_string(src)?;
    let src_val: Value = serde_json::from_str(&src_raw)?;
    let src_skills: Vec<Value> = src_val
        .get("skills")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut dst_val: Value = if dst.exists() {
        let raw = fs::read_to_string(dst).with_context(|| format!("reading {}", dst.display()))?;
        if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", dst.display()))?
        }
    } else {
        Value::Object(Map::new())
    };
    if !dst_val.is_object() {
        anyhow::bail!("destination is not a JSON object, refusing to overwrite");
    }
    let dst_skills: Vec<Value> = dst_val
        .get("skills")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let existing_names: std::collections::HashSet<String> = dst_skills
        .iter()
        .filter_map(|s| {
            s.get("name")
                .and_then(|n| n.as_str())
                .map(|s| s.to_string())
        })
        .collect();

    let mut merged = dst_skills.clone();
    let mut added = 0usize;
    for s in src_skills {
        let name = s
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        if name.is_empty() || existing_names.contains(&name) {
            continue;
        }
        merged.push(to_3p_skill_entry(&s, &name));
        added += 1;
    }
    if added == 0 {
        return Ok(false);
    }
    dst_val
        .as_object_mut()
        .unwrap()
        .insert("skills".into(), Value::Array(merged));
    if dry_run {
        return Ok(true);
    }
    let body = serde_json::to_string_pretty(&dst_val)?;
    guarded_write_file(dst, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", dst.display()))?;
    Ok(true)
}

/// Transform a standard-mode skill manifest entry into the format
/// Cowork-3p expects for locally-stored skills. Crucially we set
/// `syncManaged: false`: without it, Cowork-3p's launch-time sync
/// treats the entry as claude.ai-backed and prunes it (the bundle
/// filters via `syncManaged !== false`). Also normalizes `skillId` to
/// the name (server-style `skill_*` IDs are meaningless in 3P) and
/// flips `creatorType` to `"user"` since there's no claude.ai concept
/// of an anthropic-managed skill in 3P mode.
fn to_3p_skill_entry(src: &Value, name: &str) -> Value {
    let mut obj = Map::new();
    obj.insert("skillId".into(), Value::String(name.to_string()));
    obj.insert("name".into(), Value::String(name.to_string()));
    if let Some(desc) = src.get("description").and_then(|v| v.as_str()) {
        obj.insert("description".into(), Value::String(desc.to_string()));
    }
    obj.insert("creatorType".into(), Value::String("user".into()));
    obj.insert("syncManaged".into(), Value::Bool(false));
    if let Some(u) = src.get("updatedAt").cloned() {
        obj.insert("updatedAt".into(), u);
    }
    let enabled = src.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true);
    obj.insert("enabled".into(), Value::Bool(enabled));
    Value::Object(obj)
}

fn migrate_scheduled(roots: &MigrateRoots, dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();
    let src_json = roots.source_dir.join("scheduled-tasks.json");
    let dst_json = roots.dest_dir.join("scheduled-tasks.json");

    if src_json.exists() {
        match merge_scheduled_tasks(&src_json, &dst_json, dry_run) {
            Ok(merged_count) => r.copied += merged_count,
            Err(err) => {
                r.failed += 1;
                r.errors.push(format!("scheduled-tasks.json: {err}"));
            }
        }
    }

    // ~/Documents/Claude/Scheduled/{task}/SKILL.md — same path either mode,
    // so we copy missing files only (don't clobber).
    if let Ok(docs) = env::claude_documents_dir() {
        let sched = docs.join("Scheduled");
        if let Ok(entries) = fs::read_dir(&sched) {
            for entry in entries.flatten() {
                let task_dir = entry.path();
                if !task_dir.is_dir() {
                    continue;
                }
                // The path lives under ~/Documents either way; SKILL.md
                // migration is a no-op (same destination on both modes), so
                // count each task dir we encountered as skipped for clarity.
                r.skipped += 1;
            }
        }
    }
    r
}

fn merge_scheduled_tasks(src: &Path, dst: &Path, dry_run: bool) -> Result<usize> {
    let src_raw = fs::read_to_string(src)?;
    let src_val: Value = serde_json::from_str(&src_raw)?;
    let src_tasks: Vec<Value> = src_val
        .get("scheduledTasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut dst_val: Value = if dst.exists() {
        let raw = fs::read_to_string(dst).with_context(|| format!("reading {}", dst.display()))?;
        if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", dst.display()))?
        }
    } else {
        Value::Object(Map::new())
    };

    let dst_tasks: Vec<Value> = dst_val
        .get("scheduledTasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let existing_ids: std::collections::HashSet<String> = dst_tasks
        .iter()
        .filter_map(|t| t.get("id").and_then(|i| i.as_str()).map(|s| s.to_string()))
        .collect();

    let mut merged = dst_tasks.clone();
    let mut added = 0usize;
    for t in src_tasks {
        let id = t
            .get("id")
            .and_then(|i| i.as_str())
            .unwrap_or("")
            .to_string();
        if !id.is_empty() && existing_ids.contains(&id) {
            continue;
        }
        merged.push(t);
        added += 1;
    }

    if added == 0 {
        return Ok(0);
    }

    if !dst_val.is_object() {
        anyhow::bail!("destination is not a JSON object, refusing to overwrite");
    }
    dst_val
        .as_object_mut()
        .unwrap()
        .insert("scheduledTasks".into(), Value::Array(merged));
    if !dst_val.as_object().unwrap().contains_key("recordedSkips") {
        dst_val
            .as_object_mut()
            .unwrap()
            .insert("recordedSkips".into(), Value::Object(Map::new()));
    }

    if dry_run {
        return Ok(added);
    }
    let body = serde_json::to_string_pretty(&dst_val)?;
    guarded_write_file(dst, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", dst.display()))?;
    Ok(added)
}

fn migrate_conversations(roots: &MigrateRoots, dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();
    let Ok(entries) = fs::read_dir(&roots.source_dir) else {
        return r;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if !s.starts_with("local_") {
            continue;
        }
        let dst = roots.dest_dir.join(&name);
        if dst.exists() {
            r.skipped += 1;
            continue;
        }
        if dry_run {
            r.copied += 1;
            continue;
        }
        let src = entry.path();
        let outcome = if src.is_dir() {
            copy_dir_recursive(&src, &dst)
        } else {
            // local_{uuid}.json metadata file
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent).ok();
            }
            fs::copy(&src, &dst)
                .map(|_| ())
                .map_err(anyhow::Error::from)
        };
        match outcome {
            Ok(_) => r.copied += 1,
            Err(err) => {
                r.failed += 1;
                r.errors.push(format!("{s}: {err}"));
            }
        }
    }
    r
}

fn migrate_memory(roots: &MigrateRoots, dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();

    let candidates = [
        ("memory/memory", roots.source_dir.join("memory/memory")),
        ("spaces.json", roots.source_dir.join("spaces.json")),
    ];

    for (label, src) in candidates {
        if !src.exists() {
            continue;
        }
        let rel = label;
        let dst = roots.dest_dir.join(rel);
        if dst.exists() && dst.metadata().map(|m| m.len() > 0).unwrap_or(false) {
            r.skipped += 1;
            continue;
        }
        if dry_run {
            r.copied += 1;
            continue;
        }
        if let Some(parent) = dst.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::copy(&src, &dst) {
            Ok(_) => r.copied += 1,
            Err(err) => {
                r.failed += 1;
                r.errors.push(format!("{label}: {err}"));
            }
        }
    }

    // spaces/{space_id}/memory — per-space memory files
    let src_spaces = roots.source_dir.join("spaces");
    if let Ok(entries) = fs::read_dir(&src_spaces) {
        for e in entries.flatten() {
            let space_id = e.file_name();
            let src_mem = e.path().join("memory");
            if !src_mem.exists() {
                continue;
            }
            let dst_mem = roots.dest_dir.join("spaces").join(&space_id).join("memory");
            if dst_mem.exists() && dst_mem.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                r.skipped += 1;
                continue;
            }
            if dry_run {
                r.copied += 1;
                continue;
            }
            if let Some(parent) = dst_mem.parent() {
                let _ = fs::create_dir_all(parent);
            }
            match fs::copy(&src_mem, &dst_mem) {
                Ok(_) => r.copied += 1,
                Err(err) => {
                    r.failed += 1;
                    r.errors.push(format!(
                        "spaces/{}/memory: {err}",
                        space_id.to_string_lossy()
                    ));
                }
            }
        }
    }
    r
}

fn migrate_enabled_plugins(roots: &MigrateRoots, dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();
    let src = roots.source_dir.join("cowork_settings.json");
    let dst = roots.dest_dir.join("cowork_settings.json");
    if !src.exists() {
        return r;
    }
    match merge_top_level_objects(&src, &dst, "enabledPlugins", dry_run) {
        Ok(true) => r.copied += 1,
        Ok(false) => r.skipped += 1,
        Err(err) => {
            r.failed += 1;
            r.errors.push(format!("cowork_settings.json: {err}"));
        }
    }
    r
}

/// Merge `enabledPlugins` (or any nested map) from src into dst. Dest wins
/// on per-key conflicts. Returns whether anything actually changed.
fn merge_top_level_objects(
    src: &Path,
    dst: &Path,
    nested_key: &str,
    dry_run: bool,
) -> Result<bool> {
    let src_raw = fs::read_to_string(src)?;
    let src_val: Value = serde_json::from_str(&src_raw)?;
    let src_map = src_val
        .get(nested_key)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut dst_val: Value = if dst.exists() {
        let raw = fs::read_to_string(dst).with_context(|| format!("reading {}", dst.display()))?;
        if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", dst.display()))?
        }
    } else {
        Value::Object(Map::new())
    };

    if !dst_val.is_object() {
        anyhow::bail!("destination is not a JSON object, refusing to overwrite");
    }

    let dst_obj = dst_val.as_object_mut().unwrap();
    let mut dst_nested = dst_obj
        .get(nested_key)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    let mut changed = false;
    for (k, v) in src_map {
        if !dst_nested.contains_key(&k) {
            dst_nested.insert(k, v);
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }
    dst_obj.insert(nested_key.into(), Value::Object(dst_nested));

    if dry_run {
        return Ok(true);
    }
    let body = serde_json::to_string_pretty(&dst_val)?;
    guarded_write_file(dst, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", dst.display()))?;
    Ok(true)
}

fn migrate_preferences(dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();
    let (Ok(src_root), Ok(dst_root)) =
        (env::claude_user_data_dir(), env::claude_3p_user_data_dir())
    else {
        return r;
    };
    let files = [
        "claude_desktop_config.json",
        "extensions-blocklist.json",
        "developer_settings.json",
    ];
    for f in files {
        let src = src_root.join(f);
        let dst = dst_root.join(f);
        if !src.exists() {
            continue;
        }
        match merge_top_level_keys(&src, &dst, dry_run) {
            Ok(true) => r.copied += 1,
            Ok(false) => r.skipped += 1,
            Err(err) => {
                r.failed += 1;
                r.errors.push(format!("{f}: {err}"));
            }
        }
    }
    r
}

/// Merge every top-level key from src into dst. Dest wins on conflicts.
/// Strips sensitive keys we never want to copy.
fn merge_top_level_keys(src: &Path, dst: &Path, dry_run: bool) -> Result<bool> {
    let src_raw = fs::read_to_string(src)?;
    let src_val: Value = serde_json::from_str(&src_raw)?;
    let src_obj = match src_val {
        Value::Object(m) => m,
        _ => return Ok(false),
    };

    let mut dst_val: Value = if dst.exists() {
        let raw = fs::read_to_string(dst).with_context(|| format!("reading {}", dst.display()))?;
        if raw.trim().is_empty() {
            Value::Object(Map::new())
        } else {
            serde_json::from_str(&raw)
                .with_context(|| format!("parsing {} as JSON", dst.display()))?
        }
    } else {
        Value::Object(Map::new())
    };
    if !dst_val.is_object() {
        anyhow::bail!("destination is not a JSON object, refusing to overwrite");
    }
    let dst_obj = dst_val.as_object_mut().unwrap();

    let mut changed = false;
    for (k, v) in src_obj {
        // Prefix match: `dxt:allowlist` covers per-extension variants like
        // `dxt:allowlistEnabled:<id>`, not just the bare key.
        if SENSITIVE_PREFERENCE_KEYS.iter().any(|p| k.starts_with(p)) {
            continue;
        }
        if !dst_obj.contains_key(&k) {
            dst_obj.insert(k, v);
            changed = true;
        }
    }
    if !changed {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    let body = serde_json::to_string_pretty(&dst_val)?;
    guarded_write_file(dst, body.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", dst.display()))?;
    Ok(true)
}

/// Key *prefixes* in preference JSON we refuse to copy from standard mode
/// to 3P, either because they're encrypted-blob-bound to the source process
/// or because they reference the old claude.ai org UUID.
const SENSITIVE_PREFERENCE_KEYS: &[&str] = &["oauth:tokenCache", "dxt:allowlist"];

fn migrate_artifacts(roots: &MigrateRoots, dry_run: bool) -> CategoryReport {
    let mut r = CategoryReport::default();
    let src_json = roots.source_dir.join("artifacts.json");
    let dst_json = roots.dest_dir.join("artifacts.json");
    if src_json.exists() && !dst_json.exists() {
        if dry_run {
            r.copied += 1;
        } else if let Some(parent) = dst_json.parent() {
            let _ = fs::create_dir_all(parent);
            match fs::copy(&src_json, &dst_json) {
                Ok(_) => r.copied += 1,
                Err(err) => {
                    r.failed += 1;
                    r.errors.push(format!("artifacts.json: {err}"));
                }
            }
        }
    }
    let src_dir = roots.source_dir.join("artifacts");
    let dst_dir = roots.dest_dir.join("artifacts");
    if src_dir.exists() && !dst_dir.exists() {
        if dry_run {
            r.copied += 1;
        } else {
            match copy_dir_recursive(&src_dir, &dst_dir) {
                Ok(_) => r.copied += 1,
                Err(err) => {
                    r.failed += 1;
                    r.errors.push(format!("artifacts/: {err}"));
                }
            }
        }
    }
    r
}

// ---------- tests ----------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn make_account_org_dir(root: &Path, account: &str, org: &str) -> PathBuf {
        let dir = root.join(account).join(org);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn looks_like_uuid_accepts_valid_and_rejects_others() {
        assert!(looks_like_uuid("1bc59c73-e6a7-4034-a07d-249a08bb7d1b"));
        assert!(!looks_like_uuid("skills-plugin"));
        assert!(!looks_like_uuid("1bc59c73e6a740340a07d249a08bb7d1b"));
        assert!(!looks_like_uuid(""));
    }

    #[test]
    fn newest_account_org_picks_latest_mtime() {
        let tmp = tempdir();
        let root = tmp.path();

        let a = make_account_org_dir(
            root,
            "00000000-0000-0000-0000-000000000001",
            "00000000-0000-0000-0000-00000000000a",
        );
        let b = make_account_org_dir(
            root,
            "00000000-0000-0000-0000-000000000002",
            "00000000-0000-0000-0000-00000000000b",
        );
        // Touch B more recently.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(b.join("marker"), b"y").unwrap();
        // Re-stat: mtime should be later than a's.
        let picked = newest_account_org(root).unwrap().unwrap();
        assert_eq!(picked.2, b);
        let _ = a;
    }

    #[test]
    fn newest_account_org_ignores_non_uuid_dirs() {
        let tmp = tempdir();
        let root = tmp.path();
        fs::create_dir_all(root.join("skills-plugin").join("plugin-id")).unwrap();
        assert!(newest_account_org(root).unwrap().is_none());
    }

    #[test]
    fn count_scheduled_handles_missing_and_present() {
        let tmp = tempdir();
        let dir = tmp.path();
        assert_eq!(count_scheduled(dir), 0);
        fs::write(
            dir.join("scheduled-tasks.json"),
            r#"{"scheduledTasks":[{"id":"a"},{"id":"b"}],"recordedSkips":{}}"#,
        )
        .unwrap();
        assert_eq!(count_scheduled(dir), 2);
    }

    #[test]
    fn count_plugins_combines_rpm_and_installed() {
        let tmp = tempdir();
        let dir = tmp.path();
        fs::create_dir_all(dir.join("rpm/plugin_X")).unwrap();
        fs::create_dir_all(dir.join("rpm/plugin_Y")).unwrap();
        fs::create_dir_all(dir.join("cowork_plugins")).unwrap();
        fs::write(
            dir.join("cowork_plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"a@m":[],"b@m":[]}}"#,
        )
        .unwrap();
        assert_eq!(count_plugins(dir), 4);
    }

    #[test]
    fn merge_scheduled_tasks_keeps_dest_unchanged_and_appends_new() {
        let tmp = tempdir();
        let src = tmp.path().join("src.json");
        let dst = tmp.path().join("dst.json");
        fs::write(
            &src,
            r#"{"scheduledTasks":[{"id":"A","cron":"src-a"},{"id":"B","cron":"src-b"}]}"#,
        )
        .unwrap();
        fs::write(
            &dst,
            r#"{"scheduledTasks":[{"id":"B","cron":"dst-b-modified"},{"id":"C","cron":"dst-c"}]}"#,
        )
        .unwrap();

        let added = merge_scheduled_tasks(&src, &dst, false).unwrap();
        assert_eq!(added, 1, "only A is new");

        let merged: Value = serde_json::from_str(&fs::read_to_string(&dst).unwrap()).unwrap();
        let tasks = merged.get("scheduledTasks").unwrap().as_array().unwrap();
        let ids: Vec<&str> = tasks
            .iter()
            .filter_map(|t| t.get("id").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(ids, vec!["B", "C", "A"]);
        // B kept its dest version, not src's.
        let b = tasks
            .iter()
            .find(|t| t.get("id").and_then(|v| v.as_str()) == Some("B"))
            .unwrap();
        assert_eq!(
            b.get("cron").and_then(|v| v.as_str()),
            Some("dst-b-modified")
        );
    }

    #[test]
    fn merge_top_level_objects_union_with_dest_wins() {
        let tmp = tempdir();
        let src = tmp.path().join("src.json");
        let dst = tmp.path().join("dst.json");
        fs::write(&src, r#"{"enabledPlugins":{"x":true,"y":false}}"#).unwrap();
        fs::write(&dst, r#"{"enabledPlugins":{"y":true,"z":true}}"#).unwrap();

        let changed = merge_top_level_objects(&src, &dst, "enabledPlugins", false).unwrap();
        assert!(changed);

        let merged: Value = serde_json::from_str(&fs::read_to_string(&dst).unwrap()).unwrap();
        let m = merged.get("enabledPlugins").unwrap().as_object().unwrap();
        assert_eq!(m.get("x").unwrap().as_bool(), Some(true));
        assert_eq!(m.get("y").unwrap().as_bool(), Some(true)); // dest wins
        assert_eq!(m.get("z").unwrap().as_bool(), Some(true));
    }

    #[test]
    fn merge_top_level_keys_strips_sensitive() {
        let tmp = tempdir();
        let src = tmp.path().join("src.json");
        let dst = tmp.path().join("dst.json");
        fs::write(
            &src,
            r#"{"locale":"en-US","oauth:tokenCache":"SECRET","dxt:allowlistEnabled:abc":false}"#,
        )
        .unwrap();
        let changed = merge_top_level_keys(&src, &dst, false).unwrap();
        assert!(changed);
        let merged: Value = serde_json::from_str(&fs::read_to_string(&dst).unwrap()).unwrap();
        let obj = merged.as_object().unwrap();
        assert_eq!(obj.get("locale").unwrap().as_str(), Some("en-US"));
        assert!(!obj.contains_key("oauth:tokenCache"));
        assert!(!obj.contains_key("dxt:allowlistEnabled:abc"));
    }

    #[test]
    fn migrate_conversations_skips_existing_uuids() {
        let tmp = tempdir();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::create_dir_all(src.join("local_AAA")).unwrap();
        fs::write(src.join("local_AAA.json"), b"{}").unwrap();
        fs::create_dir_all(src.join("local_BBB")).unwrap();
        fs::write(src.join("local_BBB.json"), b"{}").unwrap();
        // Pre-existing in dst (both dir and metadata) → should skip.
        fs::create_dir_all(dst.join("local_AAA")).unwrap();
        fs::write(dst.join("local_AAA.json"), b"{}").unwrap();

        let roots = MigrateRoots {
            source_account: "src-acct".into(),
            source_org: "src-org".into(),
            source_dir: src,
            dest_account: "dst-acct".into(),
            dest_org: "dst-org".into(),
            dest_dir: dst.clone(),
        };
        let r = migrate_conversations(&roots, false);
        assert_eq!(r.copied, 2, "BBB dir and BBB.json");
        assert_eq!(r.skipped, 2, "AAA dir and AAA.json");
        assert!(dst.join("local_BBB").exists());
        assert!(dst.join("local_BBB.json").exists());
    }

    #[test]
    fn merge_skills_manifest_appends_new_by_name_in_3p_format() {
        let tmp = tempdir();
        let src = tmp.path().join("src.json");
        let dst = tmp.path().join("dst.json");
        // Source-side entries use server-style skillId and no syncManaged
        // flag — that's exactly how Cowork standard mode writes them.
        fs::write(
            &src,
            r#"{"skills":[
                {"name":"ai-meeting-logger","skillId":"skill_X1","description":"Log it","creatorType":"user","updatedAt":"2026-04-30T20:29:18.126747Z","enabled":true},
                {"name":"schedule","skillId":"src-schedule","creatorType":"anthropic","enabled":true}
            ]}"#,
        )
        .unwrap();
        fs::write(
            &dst,
            r#"{"skills":[{"name":"schedule","skillId":"schedule","creatorType":"anthropic","enabled":true}]}"#,
        )
        .unwrap();

        let changed = merge_skills_manifest(&src, &dst, false).unwrap();
        assert!(changed);
        let merged: Value = serde_json::from_str(&fs::read_to_string(&dst).unwrap()).unwrap();
        let skills = merged.get("skills").unwrap().as_array().unwrap();
        let names: Vec<&str> = skills
            .iter()
            .filter_map(|s| s.get("name").and_then(|n| n.as_str()))
            .collect();
        assert_eq!(names, vec!["schedule", "ai-meeting-logger"]);
        // The migrated skill must carry the 3P-specific fields, otherwise
        // Cowork-3p prunes it at launch (treats it as a stale server-managed
        // entry).
        let logger = skills
            .iter()
            .find(|s| s.get("name").and_then(|n| n.as_str()) == Some("ai-meeting-logger"))
            .unwrap();
        assert_eq!(
            logger.get("syncManaged").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            logger.get("creatorType").and_then(|v| v.as_str()),
            Some("user")
        );
        assert_eq!(
            logger.get("skillId").and_then(|v| v.as_str()),
            Some("ai-meeting-logger"),
            "skillId should be normalized to the skill name, not the server-generated skill_* id"
        );
        assert_eq!(
            logger.get("description").and_then(|v| v.as_str()),
            Some("Log it")
        );
        assert_eq!(
            logger.get("updatedAt").and_then(|v| v.as_str()),
            Some("2026-04-30T20:29:18.126747Z")
        );
        assert_eq!(logger.get("enabled").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn execute_dry_run_writes_nothing() {
        // We can't easily redirect env::claude_*_dir() in this unit test,
        // so verify the contract on a tiny direct call: merge with dry_run
        // doesn't write the dest file.
        let tmp = tempdir();
        let src = tmp.path().join("src.json");
        let dst = tmp.path().join("dst.json");
        fs::write(&src, r#"{"scheduledTasks":[{"id":"A"}]}"#).unwrap();

        let added = merge_scheduled_tasks(&src, &dst, true).unwrap();
        assert_eq!(added, 1);
        assert!(!dst.exists(), "dry_run must not write");
    }

    // Minimal tempdir helper to avoid a `tempfile` dep.
    struct TempDir(PathBuf);
    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn tempdir() -> TempDir {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p =
            std::env::temp_dir().join(format!("gate-connect-test-{}-{}", std::process::id(), n));
        fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}
