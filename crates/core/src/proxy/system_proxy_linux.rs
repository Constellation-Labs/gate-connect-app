//! Linux system HTTP/HTTPS proxy wiring via a user-scoped systemd
//! `environment.d` drop-in (`~/.config/environment.d/gate-proxy.conf`). Enabling
//! writes `http_proxy`/`https_proxy` (+ upper-case aliases) pointing at our
//! loopback engine, a `no_proxy` that keeps loopback traffic off the proxy, and
//! `NODE_EXTRA_CA_CERTS` pointing at our CA so Node-based CLIs (e.g. Claude
//! Code) - which ship their own bundle and ignore the system trust store -
//! accept the engine's minted leaf certs. Disabling deletes the file again.
//!
//! Why `environment.d` and not `/etc/environment`:
//!
//! - **No root.** The drop-in lives in the user's home, so enable/disable are
//!   unprivileged - no `pkexec`/polkit prompt, and no all-or-nothing privileged
//!   write that strands the toggle when polkit is unavailable. (Trusting the CA
//!   still needs root; that's a separate, one-time step in [`super::ca`].)
//! - **Transient by ownership.** We own the whole file, so "off" is a plain
//!   delete and a stale drop-in never lingers root-owned in a shared file.
//!
//! `systemd --user` reads `environment.d` at login and applies it to the
//! graphical session, so the variables reach GUI apps started afterwards *and*
//! command-line shells spawned from the session. On its own that only affects
//! **new** login sessions, which would force a logout. To avoid that, enabling
//! also pushes the same variables into the *running* session via
//! `dbus-update-activation-environment --systemd`, which updates the D-Bus
//! activation environment and the `systemd --user` manager that modern desktops
//! use to launch apps - so a tool relaunched after enabling picks up the proxy
//! immediately, no logout. That push is best-effort: with no session bus, or on
//! a pure non-systemd session (rare on modern Ubuntu/GNOME), it's a no-op and
//! the drop-in still applies at the next login. Either way, already-running
//! processes keep their environment until relaunched - nothing can change that.
//!
//! Second channel: **GNOME's own proxy setting**
//! (`org.gnome.system.proxy` via `gsettings`). The variables above can only
//! reach a process at launch, which makes "off" invisible to anything already
//! running: a browser started while routing was on keeps its `https_proxy`
//! pointer for its whole life, and once the engine stops answering, every page
//! it loads is a proxy error. GNOME's keys are re-read live (Firefox and
//! everything else on GLib's proxy resolver watch them), so enabling and
//! disabling both take effect *now* for GUI apps. Those keys belong to the user
//! rather than to us - a hand-configured corporate proxy lives there - so
//! enable captures them and off puts them back; see [`GNOME_KEYS`].
//!
//! Pairs with a *stable* engine port (persisted via [`load_port`]/[`save_port`]):
//! a session freezes the proxy pointer at login, so the engine must come back on
//! the same port across restarts or that frozen pointer dangles at a dead port.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::env;

/// Basename of our user-scoped systemd environment drop-in.
const DROPIN_NAME: &str = "gate-proxy.conf";

/// Path to our `environment.d` drop-in: `$XDG_CONFIG_HOME/environment.d/gate-proxy.conf`
/// (i.e. `~/.config/environment.d/gate-proxy.conf`).
fn dropin_path() -> Result<PathBuf> {
    Ok(dirs::config_dir()
        .context("could not resolve user config directory")?
        .join("environment.d")
        .join(DROPIN_NAME))
}

/// State recorded on enable, so disable can undo exactly what enable did.
///
/// The drop-in half needs nothing captured - we own that file outright, so "off"
/// is a plain delete - and its existence on disk is what tells
/// [`super::manager`] a previous session left the proxy on (crash reconcile).
/// GNOME's proxy keys are the opposite: they are the user's, so their prior
/// values have to be carried across the toggle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxySnapshot {
    /// Whether our drop-in was already present when we snapshotted (i.e. an
    /// earlier unclean session left it behind).
    pub block_present: bool,
    /// GNOME's proxy keys as they were before we first pointed them at the
    /// engine, captured by [`snapshot`] and written back by [`gsettings_off`].
    /// `None` on a session without `gsettings` (KDE, a bare WM) and on
    /// snapshots written before this field existed - both mean "switch the
    /// proxy mode off" rather than "restore".
    #[serde(default)]
    pub gnome_proxy: Option<Vec<GsettingsEntry>>,
}

/// One GNOME proxy key with its value as literal GVariant text, exactly as
/// `gsettings get` printed it. Kept verbatim so `gsettings set` can hand it
/// straight back: the printed form round-trips for every type we touch (strings
/// come back quoted, ports bare, `ignore-hosts` as a `['a', 'b']` array), so
/// nothing here has to model GVariant types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GsettingsEntry {
    pub schema: String,
    pub key: String,
    pub value: String,
}

fn snapshot_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?
        .join("proxy")
        .join("system-proxy.snapshot.json"))
}

/// The engine's chosen loopback port persists (via [`super::port_persist`])
/// so it can be reused across restarts (keeping a frozen session's proxy
/// pointer valid). No PAC port here: Linux wires env-var proxies straight at
/// the engine.
pub fn load_port() -> Result<Option<u16>> {
    super::port_persist::load("port")
}

/// Persist the engine port for reuse on the next run (see [`load_port`]).
pub fn save_port(port: u16) -> Result<()> {
    super::port_persist::save("port", port)
}

/// Cross-process lock serializing enable/disable, so the app and the CLI can't
/// interleave the snapshot / drop-in / port writes (see [`super::flock`]).
pub fn op_lock_path() -> Result<PathBuf> {
    Ok(env::app_support_dir()?.join("proxy").join("op.lock"))
}

/// Whether our drop-in currently exists on disk.
fn dropin_present() -> Result<bool> {
    Ok(dropin_path()?.exists())
}

/// The proxy-related environment variables we manage, in a stable order.
/// Enabling sets them (drop-in + live push); disabling blanks them in the
/// running session. Shared with the macOS and Windows exports so the three
/// platforms can't drift (see [`super::proxy_env`]).
const PROXY_VARS: [&str; 7] = super::proxy_env::VARS_CASE_SENSITIVE;

/// The name/value pairs for an *enabled* proxy pointing at `127.0.0.1:port`,
/// keyed by [`PROXY_VARS`]. Consumed by both [`build_dropin`] and the live
/// session push in [`enable`].
fn proxy_env(port: u16) -> Result<Vec<(&'static str, String)>> {
    super::proxy_env::case_sensitive(port)
}

/// Build the drop-in body from name/value pairs. systemd `environment.d` parses
/// `KEY=VALUE` lines (not shell), so a value may contain spaces; we double-quote
/// the CA path anyway for clarity and to stay safe if a consumer ever sources it
/// more strictly.
fn build_dropin(assignments: &[(&'static str, String)]) -> String {
    let mut body =
        String::from("# Managed by Gate Connect - do not edit. Removed when the proxy is off.\n");
    for (key, value) in assignments {
        if *key == "NODE_EXTRA_CA_CERTS" {
            body.push_str(&format!("{key}=\"{value}\"\n"));
        } else {
            body.push_str(&format!("{key}={value}\n"));
        }
    }
    body
}

/// True when a test seam has redirected this process's per-user paths.
///
/// The drop-in, the snapshot and the port file all move with that seam; the
/// login session does not. Its environment and GNOME's dconf database sit
/// outside the redirected filesystem, so a unit test calling [`enable`] would
/// reconfigure the developer's own desktop and leave it that way. Every
/// session-facing side effect below is skipped under the seam.
fn session_effects_suppressed() -> bool {
    crate::env::test_seam("GATE_CONNECT_TEST_HOME").is_some()
}

/// Run a session-facing command for its effect only: silent on success, silent
/// when the binary simply is not there (a desktop that ships neither systemd nor
/// GNOME), a diagnostic on anything else. Never fails the caller - the drop-in
/// and the GNOME keys are the durable state, and these commands only carry "on"
/// and "off" into the session that is already running.
fn run_best_effort(program: &str, args: &[&str]) {
    match std::process::Command::new(program).args(args).output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => eprintln!(
            "[gate] {program} {} exited {}: {}",
            args.first().copied().unwrap_or(""),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // tool absent; fine
        Err(e) => eprintln!("[gate] could not run {program}: {e}"),
    }
}

/// Push proxy variable assignments into the *running* login session so tools
/// launched (or relaunched) now pick them up without waiting for the next
/// login. `dbus-update-activation-environment --systemd` updates both the D-Bus
/// activation environment and the `systemd --user` manager that modern desktops
/// use to spawn apps. Best-effort: no session bus, or a desktop that doesn't
/// ship the tool, just means the `environment.d` drop-in applies at next login
/// instead. Already-running processes keep their old environment until
/// relaunched - nothing can change that.
fn push_to_session(assignments: &[(&'static str, String)]) {
    if session_effects_suppressed() {
        return;
    }
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return; // no graphical session bus to update; drop-in covers next login
    }
    let mut cmd = std::process::Command::new("dbus-update-activation-environment");
    cmd.arg("--systemd");
    for (key, value) in assignments {
        cmd.arg(format!("{key}={value}"));
    }
    match cmd.output() {
        Ok(out) if out.status.success() => {}
        Ok(out) => eprintln!(
            "[gate] live proxy env push exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // tool absent; fine
        Err(e) => eprintln!("[gate] could not run dbus-update-activation-environment: {e}"),
    }
}

/// Take our proxy variables *out* of the running login session.
///
/// `dbus-update-activation-environment` can only assign, so the only way "off"
/// used to reach the session was a push of empty values - which left
/// `http_proxy=` (present, but empty) in the `systemd --user` manager that GNOME
/// launches desktop apps from. `systemctl --user unset-environment` genuinely
/// removes them, so an app started after "off" inherits no proxy pointer at all
/// rather than an empty one. The blanking push stays for the D-Bus activation
/// environment, which has no unset and which non-systemd activation still reads.
///
/// Best-effort, and - like the enable-time push - powerless over processes that
/// are already running: those keep the pointer they launched with until they are
/// restarted. Reaching *them* is what [`gsettings_off`] is for.
fn unset_from_session() {
    if !session_effects_suppressed() {
        let mut args = vec!["--user", "unset-environment"];
        args.extend(PROXY_VARS);
        run_best_effort("systemctl", &args);
    }
    let cleared: Vec<(&'static str, String)> = PROXY_VARS
        .into_iter()
        .map(|key| (key, String::new()))
        .collect();
    push_to_session(&cleared);
}

/// The GNOME proxy keys we overwrite while routing is on, and therefore the ones
/// [`snapshot`] captures beforehand and [`gsettings_off`] writes back.
///
/// Why this channel exists at all: see the second-channel paragraph in the
/// module docs. The engine only speaks HTTP and HTTPS, so we set exactly those
/// two protocols and leave `use-same-proxy`, `ftp` and `socks` as the user has
/// them.
///
/// `mode` comes first so the off path stops routing before it moves the host and
/// port back underneath it; [`gsettings_apply`] writes them in the opposite
/// order, for the same reason.
const GNOME_KEYS: [(&str, &str); 6] = [
    ("org.gnome.system.proxy", "mode"),
    ("org.gnome.system.proxy", "ignore-hosts"),
    ("org.gnome.system.proxy.http", "host"),
    ("org.gnome.system.proxy.http", "port"),
    ("org.gnome.system.proxy.https", "host"),
    ("org.gnome.system.proxy.https", "port"),
];

/// The engine's loopback host as GVariant text - the same `127.0.0.1` the
/// variables point at (see [`super::proxy_env`]).
const ENGINE_HOST_GVARIANT: &str = "'127.0.0.1'";

/// Read one key, or `None` if `gsettings` or the schema is missing (i.e. not a
/// GNOME session).
fn gsettings_get(schema: &str, key: &str) -> Option<String> {
    let out = std::process::Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Capture [`GNOME_KEYS`] so the off path can put them back verbatim.
///
/// `None` when this is not a GNOME session, or when any single key won't read: a
/// partial record would restore some keys and leave others pointing at an engine
/// that has stopped answering, which is worse than falling back to mode-off.
fn gsettings_capture() -> Option<Vec<GsettingsEntry>> {
    if session_effects_suppressed() {
        return None;
    }
    let mut entries = Vec::with_capacity(GNOME_KEYS.len());
    for (schema, key) in GNOME_KEYS {
        entries.push(GsettingsEntry {
            schema: schema.to_string(),
            key: key.to_string(),
            value: gsettings_get(schema, key)?,
        });
    }
    Some(entries)
}

/// Render `no_proxy`'s host list as the GVariant string array `ignore-hosts`
/// wants, so both channels exempt exactly the same hosts. The hosts are our own
/// literals, so there are no quotes to escape.
fn gvariant_string_array(hosts: &str) -> String {
    let quoted: Vec<String> = hosts
        .split(',')
        .map(|host| format!("'{}'", host.trim()))
        .collect();
    format!("[{}]", quoted.join(", "))
}

/// Point GNOME's proxy setting at the loopback engine, so apps that are already
/// running follow the toggle instead of waiting to be relaunched. Best-effort: a
/// session without `gsettings` is still routed by the drop-in, so nothing here
/// may fail [`enable`].
fn gsettings_apply(port: u16) {
    if session_effects_suppressed() {
        return;
    }
    let ignore = gvariant_string_array(super::proxy_env::NO_PROXY_VALUE);
    let port = port.to_string();
    // `mode` last: until it flips to manual the host and port keys are inert, so
    // nothing can route at a half-written pointer.
    for (schema, key, value) in [
        ("org.gnome.system.proxy", "ignore-hosts", ignore.as_str()),
        ("org.gnome.system.proxy.http", "host", ENGINE_HOST_GVARIANT),
        ("org.gnome.system.proxy.http", "port", port.as_str()),
        ("org.gnome.system.proxy.https", "host", ENGINE_HOST_GVARIANT),
        ("org.gnome.system.proxy.https", "port", port.as_str()),
        ("org.gnome.system.proxy", "mode", "'manual'"),
    ] {
        run_best_effort("gsettings", &["set", schema, key, value]);
    }
}

/// Put GNOME's proxy keys back as [`gsettings_capture`] found them, or - with
/// nothing recorded - just switch the proxy mode off.
///
/// Restoring rather than blanket-clearing because these keys are the user's:
/// someone with a hand-configured proxy would otherwise lose it the first time
/// they toggled ours.
fn gsettings_off(prior: Option<&[GsettingsEntry]>) {
    if session_effects_suppressed() {
        return;
    }
    match prior {
        // Recorded `mode` first (see [`GNOME_KEYS`]), so routing stops before
        // the host and port move back.
        Some(entries) => {
            for entry in entries {
                run_best_effort(
                    "gsettings",
                    &["set", &entry.schema, &entry.key, &entry.value],
                );
            }
        }
        // Nothing recorded: a pre-upgrade snapshot, a non-GNOME session, or a
        // capture that failed. The mode is what makes the host and port live, so
        // clearing it stops routing and leaves the rest for the user to inspect.
        None => run_best_effort(
            "gsettings",
            &["set", "org.gnome.system.proxy", "mode", "'none'"],
        ),
    }
}

/// Note whether our drop-in is currently present, and what GNOME's proxy keys
/// held before we touched them. Non-privileged.
pub fn snapshot() -> Result<ProxySnapshot> {
    let block_present = dropin_present()?;
    // With our drop-in already on disk, "prior" means what an *earlier* session
    // recorded, never what `gsettings` says now: reading now would record our
    // own manual pointer as the user's setting and make every later restore
    // leave the machine routed at a port that is about to die. This is the
    // normal case rather than a corner - the startup re-honor calls `enable`,
    // and so `snapshot`, with the crashed session's wiring still in place.
    let gnome_proxy = if block_present {
        load_snapshot()
            .unwrap_or(None)
            .and_then(|previous| previous.gnome_proxy)
    } else {
        gsettings_capture()
    };
    Ok(ProxySnapshot {
        block_present,
        gnome_proxy,
    })
}

pub fn save_snapshot(snapshot: &ProxySnapshot) -> Result<()> {
    let path = snapshot_path()?;
    let raw = serde_json::to_string_pretty(snapshot).context("serializing proxy snapshot")?;
    // Atomic write (handles parent dirs too): a torn snapshot would make
    // disable/reconcile fall back to force-off instead of an exact restore.
    crate::primitives::write_file(&path, raw.as_bytes(), 0o600)
        .with_context(|| format!("writing {}", path.display()))
}

pub fn load_snapshot() -> Result<Option<ProxySnapshot>> {
    let path = snapshot_path()?;
    match fs::read_to_string(&path) {
        Ok(raw) => {
            Ok(Some(serde_json::from_str(&raw).with_context(|| {
                format!("parsing {} as JSON", path.display())
            })?))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn clear_snapshot() -> Result<()> {
    let path = snapshot_path()?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

/// The proxy URL currently exported to the environment, read back from the
/// drop-in rather than from anything we remember writing.
///
/// The drop-in is the readback source even though [`gsettings_apply`] mirrors
/// the same pointer into GNOME: the file is ours alone, while GNOME's keys are
/// the user's and may hold their proxy rather than ours. There is no PAC on this
/// platform either way (see `ENV_CHANNEL_SEPARABLE`).
pub fn exported_proxy() -> Result<Option<String>> {
    let path = dropin_path()?;
    let body = match fs::read_to_string(&path) {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    Ok(body
        .lines()
        .find_map(|l| l.trim().strip_prefix("HTTPS_PROXY="))
        .map(|v| v.trim().trim_matches('"').to_string()))
}

/// Still false now that [`gsettings_apply`] drives GNOME's setting as well. That
/// channel reaches GUI apps, but the tools this product exists to route are CLIs
/// on Node, Bun and Python, and they read the variables and nothing else - so
/// declining the variables would still mean declining routing. macOS and Windows
/// can separate the two because their OS proxy setting is a PAC that every
/// platform HTTP stack honours.
pub const ENV_CHANNEL_SEPARABLE: bool = false;

/// Point the system proxy at the loopback engine: write our `environment.d`
/// drop-in (applied to future login sessions) and push the same variables into
/// the running session so a tool relaunched now picks them up without a logout.
/// Unprivileged (user's home).
pub fn enable(port: u16) -> Result<()> {
    let assignments = proxy_env(port)?;
    let path = dropin_path()?;
    crate::primitives::write_file(&path, build_dropin(&assignments).as_bytes(), 0o644)
        .with_context(|| format!("writing {}", path.display()))?;
    push_to_session(&assignments);
    // GNOME's own setting, so apps that are already running pick the proxy up
    // (see [`GNOME_KEYS`]). Last and best-effort: the drop-in above is the part
    // that must not fail.
    gsettings_apply(port);
    Ok(())
}

/// Turn routing off, putting GNOME's proxy keys back to the values [`snapshot`]
/// captured. The drop-in half needs no captured state - we own that file
/// outright, so "off" there is a plain delete. Unprivileged.
pub fn restore(snapshot: &ProxySnapshot) -> Result<()> {
    off(snapshot.gnome_proxy.as_deref())
}

/// Turn routing off with no snapshot in hand. Fail-safe used when none is
/// available, so a dead engine never strands new shells at an unreachable proxy.
/// Still restores GNOME's keys exactly when an earlier session left a record of
/// them on disk; without one it only clears the proxy mode. Unprivileged.
pub fn force_off() -> Result<()> {
    let recorded = load_snapshot()
        .unwrap_or(None)
        .and_then(|snapshot| snapshot.gnome_proxy);
    off(recorded.as_deref())
}

/// Shared body of [`restore`] and [`force_off`]: delete the drop-in, take the
/// variables out of the running session, and hand GNOME's keys back.
fn off(prior_gnome: Option<&[GsettingsEntry]>) -> Result<()> {
    let path = dropin_path()?;
    // Evidence that we are the ones routing: our drop-in is still on disk.
    let was_routing = path.exists();
    let result = match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    };
    // Blank the vars in the running session too, so tools launched after turning
    // the proxy off stop routing through a possibly-dead engine without waiting
    // for the next login. (Already-running processes keep them until relaunched.)
    unset_from_session();
    // GNOME's keys only when there is evidence we wrote them: the drop-in was
    // still there, or a session recorded the prior values. App exit runs this
    // path unconditionally, including when the proxy was never on, and a blanket
    // mode 'none' would flatten a proxy the user configured themselves.
    if was_routing || prior_gnome.is_some() {
        gsettings_off(prior_gnome);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_temp_env<T>(f: impl FnOnce() -> T) -> T {
        // These tests mutate process-global state (XDG_CONFIG_HOME and the
        // GATE_CONNECT_TEST_HOME data-dir seam) and share a single temp dir, so
        // they must not run concurrently: otherwise one test's teardown
        // `remove_dir_all` races another's writes and the atomic rename fails
        // with ENOENT. Serialize them - and not only against each other: this
        // sets `GATE_CONNECT_TEST_HOME` for the whole process, so the exclusion
        // has to cover every test that reads a per-user path, not just this
        // module's. See `crate::env::path_env_lock`.
        let _lock = crate::env::path_env_lock();

        // `dropin_path` keys off `dirs::config_dir()` (XDG_CONFIG_HOME), and the
        // snapshot/port paths off `app_support_dir`; point both at a throwaway
        // dir so the test never touches the real user config.
        let tmp = std::env::temp_dir().join(format!("gate-proxy-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        std::env::set_var("XDG_CONFIG_HOME", tmp.join("config"));
        // Redirect the data dir via the GATE_CONNECT_TEST_HOME seam, which
        // `app_support_dir` consults before the mutex override - a mutex
        // override would be silently bypassed when the env var is already set in
        // the environment (CI). Save and restore any ambient value rather than
        // clearing it, since this runs in-process alongside the other unit tests.
        let prev_home = std::env::var_os("GATE_CONNECT_TEST_HOME");
        std::env::set_var("GATE_CONNECT_TEST_HOME", &tmp);
        let out = f();
        match prev_home {
            Some(v) => std::env::set_var("GATE_CONNECT_TEST_HOME", v),
            None => std::env::remove_var("GATE_CONNECT_TEST_HOME"),
        }
        let _ = fs::remove_dir_all(&tmp);
        out
    }

    #[test]
    fn enable_writes_dropin_then_force_off_removes_it() {
        with_temp_env(|| {
            assert!(!dropin_present().unwrap());
            enable(41234).unwrap();
            assert!(dropin_present().unwrap());
            let body = fs::read_to_string(dropin_path().unwrap()).unwrap();
            assert!(body.contains("http_proxy=http://127.0.0.1:41234"));
            assert!(body.contains("HTTPS_PROXY=http://127.0.0.1:41234"));
            // CA path is double-quoted so an embedded space is safe.
            assert!(body.contains("NODE_EXTRA_CA_CERTS=\""));
            force_off().unwrap();
            assert!(!dropin_present().unwrap());
        });
    }

    #[test]
    fn force_off_is_noop_without_dropin() {
        with_temp_env(|| {
            assert!(force_off().is_ok());
        });
    }

    #[test]
    fn gnome_keys_start_with_mode_so_off_stops_routing_first() {
        assert_eq!(GNOME_KEYS[0], ("org.gnome.system.proxy", "mode"));
    }

    #[test]
    fn ignore_hosts_renders_no_proxy_as_a_gvariant_array() {
        assert_eq!(
            gvariant_string_array("localhost,127.0.0.1,::1"),
            "['localhost', '127.0.0.1', '::1']"
        );
    }

    #[test]
    fn ignore_hosts_covers_every_no_proxy_host() {
        // The two channels have to exempt the same hosts: a host the variables
        // keep off the proxy but GNOME sends through it is a loop (OpenCode's
        // TUI talking to its own local server) that only GUI-launched tools hit.
        let rendered = gvariant_string_array(super::super::proxy_env::NO_PROXY_VALUE);
        for host in super::super::proxy_env::NO_PROXY_VALUE.split(',') {
            assert!(
                rendered.contains(&format!("'{host}'")),
                "{rendered} is missing {host}"
            );
        }
    }
}
