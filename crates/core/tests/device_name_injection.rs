//! `x-gate-device-name`: labelling gateway traffic with the machine's name.
//!
//! An integration test for the reason `model_choice_injection.rs` gives - the
//! resolution reads the real preferences file, and the app-support override is
//! process-global.

use std::path::PathBuf;
use std::sync::Mutex;

use gate_connect_core::env;
use gate_connect_core::preferences;
use gate_connect_core::proxy::testing::{
    inject_attribution_for_tests, GATE_DEVICE_NAME_HEADER_NAME,
};

use hyper::header::{HeaderMap, HeaderValue, USER_AGENT};

static LOCK: Mutex<()> = Mutex::new(());

struct TempHome {
    dir: PathBuf,
}

impl TempHome {
    fn set() -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before the epoch")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("gc-device-name-inject-{n}"));
        std::fs::create_dir_all(&dir).expect("create temp app-support dir");
        env::set_app_support_dir_for_tests(Some(dir.clone()));
        preferences::reset_cache_for_tests();
        Self { dir }
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        env::set_app_support_dir_for_tests(None);
        preferences::reset_cache_for_tests();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Headers as a request from a tool would arrive.
fn headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(USER_AGENT, HeaderValue::from_static("claude-cli/1.2.3"));
    h
}

fn device_header(h: &HeaderMap) -> Option<String> {
    h.get(GATE_DEVICE_NAME_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

#[test]
fn the_users_name_for_the_machine_is_sent() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_device_name("Studio Mac").expect("save");

    let mut h = headers();
    inject_attribution_for_tests(&mut h);

    assert_eq!(device_header(&h).as_deref(), Some("Studio Mac"));
}

#[test]
fn an_unnamed_device_sends_no_label() {
    // Skipping the naming step means skipping the header. The Settings row still
    // shows the hostname, but the hostname is a display fallback and stops
    // there: it usually carries a person's name, and a skip that sent it anyway
    // would mean nothing. Such a device is attributed by its install id alone.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();

    let mut h = headers();
    inject_attribution_for_tests(&mut h);

    assert_eq!(device_header(&h), None);
    // The window still has something to show, which is the whole reason the two
    // reads are separate functions.
    assert!(!preferences::device_name().is_empty());
}

#[test]
fn clearing_a_name_stops_sending_it() {
    // The other half of the skip: a user who named the device and then deleted
    // the text is asking to stop labelling their traffic, and must not be left
    // sending the last name they typed.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_device_name("Studio Mac").expect("save");

    let mut named = headers();
    inject_attribution_for_tests(&mut named);
    assert_eq!(device_header(&named).as_deref(), Some("Studio Mac"));

    preferences::set_device_name("   ").expect("save");

    let mut cleared = headers();
    inject_attribution_for_tests(&mut cleared);
    assert_eq!(device_header(&cleared), None);
}

#[test]
fn an_enormous_name_cannot_blow_the_header_block() {
    // A name is free text and both inputs accept a paste, so the bound is the
    // code's job. Truncated rather than dropped: the request must still be
    // attributed, and it must still be a request the gateway will accept.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_device_name(&"M".repeat(64 * 1024)).expect("save");

    let mut h = headers();
    inject_attribution_for_tests(&mut h);

    let sent = device_header(&h).expect("still attributed");
    assert!(sent.len() <= 128, "sent {} bytes", sent.len());
    assert!(sent.starts_with("MMMM"));
}

#[test]
fn a_tool_cannot_label_its_traffic_as_another_machines() {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_device_name("Studio Mac").expect("save");

    let mut h = headers();
    h.insert(
        GATE_DEVICE_NAME_HEADER_NAME,
        HeaderValue::from_static("someone-elses-laptop"),
    );
    inject_attribution_for_tests(&mut h);

    assert_eq!(device_header(&h).as_deref(), Some("Studio Mac"));
}

#[test]
fn a_rename_takes_effect_without_a_restart() {
    // Same contract as the model choice: the read is cached off the request
    // path, and a rename that only applied after a restart would be its own bug
    // report - on Linux the engine lives in the helper daemon, not the window.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_device_name("Before").expect("save");

    let mut first = headers();
    inject_attribution_for_tests(&mut first);
    assert_eq!(device_header(&first).as_deref(), Some("Before"));

    preferences::set_device_name("After").expect("save");

    let mut second = headers();
    inject_attribution_for_tests(&mut second);
    assert_eq!(device_header(&second).as_deref(), Some("After"));
}

#[test]
fn a_name_the_header_codec_rejects_is_left_off() {
    // A rename can be any Unicode ("Gabriel's MacBook" with a curly quote).
    // Attribution's rule applies: left off, never escaped, never a failed
    // request.
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _tmp = TempHome::set();
    preferences::set_device_name("G\u{2019}s MacBook").expect("save");

    let mut h = headers();
    inject_attribution_for_tests(&mut h);

    assert_eq!(device_header(&h), None);
}
