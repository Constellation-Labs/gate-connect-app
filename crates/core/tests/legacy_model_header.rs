//! `x-gate-model` is retired but still stripped, on every path to the gateway.
//!
//! Gate Connect no longer sends it - the chosen models live in the tool's own
//! config - but a gateway that predates that change still rewrites the served
//! model from it. A local process that set it itself could therefore pick a
//! paid model on the user's behalf, so every hop that reaches the gateway must
//! remove it (review on #382: deleting `model_choice_injection.rs` had taken
//! these assertions with it).
//!
//! The MITM engine's rewrite is exercised through its test seam; the relay's
//! catalog and Gate models routes end to end, against a mock gateway.

use gate_connect_core::proxy::testing::{
    apply_rewrite_for_tests, inject_attribution_for_tests, GATE_MODEL_HEADER_NAME,
};
use hyper::header::{HeaderMap, HeaderValue};

#[test]
fn attribution_strips_a_caller_set_model_header() {
    let mut h = HeaderMap::new();
    h.insert(
        GATE_MODEL_HEADER_NAME,
        HeaderValue::from_static("anthropic/claude-opus-5"),
    );
    h.insert(
        "user-agent",
        HeaderValue::from_static("codex_cli_rs/0.159.0"),
    );
    inject_attribution_for_tests(&mut h);
    assert!(h.get(GATE_MODEL_HEADER_NAME).is_none());
}

#[test]
fn the_engine_rewrite_strips_it_too() {
    let mut req = hyper::Request::builder()
        .method("POST")
        .uri("https://api.anthropic.com/v1/messages")
        .header(GATE_MODEL_HEADER_NAME, "openai/gpt-5.6-luna")
        .header("user-agent", "claude-cli/2.1.285")
        .body(())
        .unwrap();
    let gateway: hyper::Uri = "https://gw.example.com".parse().unwrap();
    apply_rewrite_for_tests(
        &mut req,
        &gateway,
        "https://api.anthropic.com",
        "sk-gw-test",
    )
    .expect("rewrite");
    assert!(req.headers().get(GATE_MODEL_HEADER_NAME).is_none());
    assert_eq!(
        req.headers()
            .get("x-gate-upstream-url")
            .and_then(|v| v.to_str().ok()),
        Some("https://api.anthropic.com"),
        "an ordinary BYOK rewrite otherwise"
    );
}
