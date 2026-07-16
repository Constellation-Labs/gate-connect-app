//! Build script: make the `option_env!`-baked Cognito OAuth config
//! (see `src/oauth.rs`) cache-correct. Cargo does not track env vars read via
//! `option_env!`, so without this a restored `target/` can mask a changed
//! value and ship a stale (or absent) bake. Declaring the vars forces a
//! recompile whenever any of them changes.
fn main() {
    for var in [
        "GATE_COGNITO_HOSTED_DOMAIN",
        "GATE_COGNITO_CLIENT_ID",
        "GATE_COGNITO_SCOPES",
        "GATE_COGNITO_HOSTED_DOMAIN_STAGING",
        "GATE_COGNITO_CLIENT_ID_STAGING",
        "GATE_COGNITO_SCOPES_STAGING",
    ] {
        println!("cargo:rerun-if-env-changed={var}");
    }
}
