//! The gateway's harness manifest, vendored, and the checks that hold this
//! app's own harness lists to it.
//!
//! `vendor/harnesses.json` is a byte copy of the gateway repo's manifest: the
//! one list of every harness Gate supports and what it promises for each. The
//! two repositories ship separately, so the copy is taken by hand
//! (`ci/vendor-manifest.sh`) with its SHA-256 recorded beside it, and read with
//! `include_str!`: nothing loads it from disk or the network at run time.
//!
//! This app keeps three harness lists of its own, and each check below compares
//! one with the manifest **in both directions**, the way the gateway's own
//! `check.mjs` does for its surfaces:
//!
//! - [`check_routing`]: each integration's [`Mechanism`] and Gate models
//!   support against the entry's `routing`.
//! - [`check_domain_slugs`]: the proxy catalog's domain slugs against the
//!   `routing.proxy_domain` of the manifest's Works entries.
//! - [`check_stamping_names`]: the slugs the request-stamping table emits
//!   against every entry's `surfaces.client_tool_slug`.
//!
//! Each returns every [`Drift`] it finds, naming the harness and the missing
//! piece, rather than stopping at the first. The checks are pure functions over
//! plain data so a fixture can prove each variant fires; the tests at the
//! bottom run them on the real vendored copy and the real lists.
//!
//! The types are permissive on purpose (no `deny_unknown_fields`): a field the
//! gateway adds to its schema must not break this build, only a field a check
//! reads.

use serde::Deserialize;

use crate::registry::Mechanism;

/// The vendored manifest, exactly as copied.
pub const MANIFEST_JSON: &str = include_str!("../vendor/harnesses.json");

/// The SHA-256 recorded when the manifest was vendored, as lower-case hex.
pub const MANIFEST_SHA256: &str = include_str!("../vendor/harnesses.json.sha256");

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub harnesses: Vec<Harness>,
}

#[derive(Debug, Deserialize)]
pub struct Harness {
    pub id: String,
    /// `certified`, `works` or `unsupported`.
    pub tier: String,
    pub routing: Routing,
    pub surfaces: Surfaces,
}

#[derive(Debug, Deserialize)]
pub struct Routing {
    /// `relay`, `proxy-engine`, `manual` or `none`.
    pub mechanism: String,
    pub proxy_domain: Option<String>,
    /// Optional in the schema; absent means false.
    #[serde(default)]
    pub gate_models: bool,
}

#[derive(Debug, Deserialize)]
pub struct Surfaces {
    pub connect: Option<Surface<Connect>>,
    pub client_tool_slug: Option<Surface<String>>,
}

#[derive(Debug, Deserialize)]
pub struct Connect {
    pub mechanism: String,
}

/// A surface field is three-state in the manifest: a value, `null`, or the
/// not-applicable marker `{"na": "<reason>"}`. `null` is the `None` around it.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Surface<T> {
    NotApplicable { na: String },
    Value(T),
}

impl<T> Surface<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Surface::Value(v) => Some(v),
            Surface::NotApplicable { .. } => None,
        }
    }
}

/// Parse the vendored manifest.
pub fn load() -> anyhow::Result<Manifest> {
    Ok(serde_json::from_str(MANIFEST_JSON)?)
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// One disagreement between this app and the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Drift {
    /// An integration routes one way and the manifest says another.
    /// `field` is the manifest key that disagrees.
    Routing {
        harness: String,
        field: &'static str,
        manifest: String,
        code: &'static str,
    },
    /// The manifest and the integration disagree on Gate models support.
    GateModels {
        harness: String,
        manifest: bool,
        code: bool,
    },
    /// An integration ships with no manifest entry naming it.
    IntegrationNotInManifest { tool: String },
    /// The manifest names a Gate Connect integration this app does not ship.
    ManifestIntegrationMissing { harness: String },
    /// The proxy catalog has a domain no Works entry names: either no entry
    /// names it, or the one that does promises less than Works.
    DomainNotInManifest { slug: String },
    /// A manifest entry names a domain the proxy catalog does not have.
    DomainNotInCatalog { harness: String, slug: String },
    /// The stamping table emits a slug no manifest entry names.
    StampNotInManifest { slug: String },
    /// A manifest entry names a slug the stamping table never emits.
    StampMissing { harness: String, slug: String },
}

/// What [`check_routing`] needs to know about one integration.
#[derive(Debug, Clone)]
pub struct ToolRouting {
    pub slug: String,
    pub mechanism: Mechanism,
    pub gate_models: bool,
}

impl ToolRouting {
    /// Every integration in [`crate::registry::registry`].
    pub fn from_registry() -> Vec<ToolRouting> {
        crate::registry::registry()
            .iter()
            .map(|i| ToolRouting {
                slug: i.id().slug().to_string(),
                mechanism: i.mechanism(),
                gate_models: i.supports_gate_models(),
            })
            .collect()
    }
}

/// The manifest's name for a [`Mechanism`], or `None` for the environment
/// channel, which is a mechanism rather than a harness and has no entry:
/// `env-proxy` exports the proxy variables for whatever reads them.
fn manifest_mechanism(mechanism: Mechanism) -> Option<&'static str> {
    match mechanism {
        Mechanism::Relay => Some("relay"),
        Mechanism::ForwardProxy => Some("proxy-engine"),
        Mechanism::Environment => None,
    }
}

/// Each integration against its manifest entry: `routing.mechanism`,
/// `surfaces.connect.mechanism` and `routing.gate_models`. Both directions: an
/// integration with no entry, and an entry naming an integration (a non-null,
/// applicable `surfaces.connect`) this app does not ship.
pub fn check_routing(manifest: &Manifest, tools: &[ToolRouting]) -> Vec<Drift> {
    let mut drift = Vec::new();
    for tool in tools {
        let Some(code) = manifest_mechanism(tool.mechanism) else {
            continue;
        };
        let Some(entry) = manifest.harnesses.iter().find(|h| h.id == tool.slug) else {
            drift.push(Drift::IntegrationNotInManifest {
                tool: tool.slug.clone(),
            });
            continue;
        };
        if entry.routing.mechanism != code {
            drift.push(Drift::Routing {
                harness: entry.id.clone(),
                field: "routing.mechanism",
                manifest: entry.routing.mechanism.clone(),
                code,
            });
        }
        match entry.surfaces.connect.as_ref().and_then(Surface::value) {
            Some(connect) if connect.mechanism != code => drift.push(Drift::Routing {
                harness: entry.id.clone(),
                field: "surfaces.connect.mechanism",
                manifest: connect.mechanism.clone(),
                code,
            }),
            Some(_) => {}
            None => drift.push(Drift::Routing {
                harness: entry.id.clone(),
                field: "surfaces.connect",
                manifest: "no integration".to_string(),
                code,
            }),
        }
        if entry.routing.gate_models != tool.gate_models {
            drift.push(Drift::GateModels {
                harness: entry.id.clone(),
                manifest: entry.routing.gate_models,
                code: tool.gate_models,
            });
        }
    }
    for entry in &manifest.harnesses {
        let names_integration = entry
            .surfaces
            .connect
            .as_ref()
            .and_then(Surface::value)
            .is_some();
        if names_integration && !tools.iter().any(|t| t.slug == entry.id) {
            drift.push(Drift::ManifestIntegrationMissing {
                harness: entry.id.clone(),
            });
        }
    }
    drift
}

/// The proxy catalog's domain slugs against the manifest's domains.
///
/// A catalog domain has to be named by a **Works** entry, not merely by some
/// entry: a domain the app routes is a support claim, and an entry that names it
/// at Unsupported would be the app routing something the gateway promises
/// nothing for. The other direction holds for every entry, whatever its tier: a
/// manifest domain the catalog does not have is a claim the app cannot honour.
pub fn check_domain_slugs(manifest: &Manifest, catalog: &[&str]) -> Vec<Drift> {
    let mut drift = Vec::new();
    for &slug in catalog {
        let named = manifest
            .harnesses
            .iter()
            .any(|h| h.tier == "works" && h.routing.proxy_domain.as_deref() == Some(slug));
        if !named {
            drift.push(Drift::DomainNotInManifest {
                slug: slug.to_string(),
            });
        }
    }
    for entry in &manifest.harnesses {
        if let Some(slug) = entry.routing.proxy_domain.as_deref() {
            if !catalog.contains(&slug) {
                drift.push(Drift::DomainNotInCatalog {
                    harness: entry.id.clone(),
                    slug: slug.to_string(),
                });
            }
        }
    }
    drift
}

/// The slugs the stamping table emits against every applicable
/// `surfaces.client_tool_slug`.
pub fn check_stamping_names(manifest: &Manifest, stamped: &[&str]) -> Vec<Drift> {
    let named: Vec<(&str, &str)> = manifest
        .harnesses
        .iter()
        .filter_map(|h| {
            let slug = h.surfaces.client_tool_slug.as_ref()?.value()?;
            Some((h.id.as_str(), slug.as_str()))
        })
        .collect();
    let mut drift = Vec::new();
    for &slug in stamped {
        if !named.iter().any(|(_, s)| *s == slug) {
            drift.push(Drift::StampNotInManifest {
                slug: slug.to_string(),
            });
        }
    }
    for (harness, slug) in named {
        if !stamped.contains(&slug) {
            drift.push(Drift::StampMissing {
                harness: harness.to_string(),
                slug: slug.to_string(),
            });
        }
    }
    drift
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The slugs the stamping table emits, read from the table itself.
    fn stamped_slugs() -> Vec<&'static str> {
        crate::proxy::CLIENT_TOOL_NEEDLES
            .iter()
            .map(|(_, client)| client.slug())
            .collect()
    }

    fn catalog_slugs() -> Vec<String> {
        crate::proxy::default_domains()
            .into_iter()
            .map(|d| d.slug)
            .collect()
    }

    // --- the real vendored copy against the real lists

    /// AC1: a hand edit of the vendored copy without re-vendoring fails here.
    #[test]
    fn the_vendored_manifest_matches_its_recorded_checksum() {
        assert_eq!(
            sha256_hex(MANIFEST_JSON.as_bytes()),
            MANIFEST_SHA256.trim(),
            "vendor/harnesses.json was changed without its checksum; \
             re-vendor with ci/vendor-manifest.sh instead of editing it"
        );
    }

    #[test]
    fn the_vendored_manifest_parses() {
        let manifest = load().expect("vendor/harnesses.json does not parse");
        assert_eq!(manifest.schema_version, 1);
        assert!(!manifest.harnesses.is_empty());
    }

    #[test]
    fn every_integration_routes_the_way_the_manifest_says() {
        let drift = check_routing(&load().unwrap(), &ToolRouting::from_registry());
        assert_eq!(drift, vec![], "routing drift against the manifest");
    }

    /// AC4: a proxy domain slug with no manifest entry fails here.
    #[test]
    fn every_proxy_domain_is_in_the_manifest() {
        let slugs = catalog_slugs();
        let slugs: Vec<&str> = slugs.iter().map(String::as_str).collect();
        let drift = check_domain_slugs(&load().unwrap(), &slugs);
        assert_eq!(drift, vec![], "proxy domain drift against the manifest");
    }

    /// AC2: renaming a slug in the stamping table without the manifest fails here.
    #[test]
    fn every_stamped_name_is_in_the_manifest() {
        let drift = check_stamping_names(&load().unwrap(), &stamped_slugs());
        assert_eq!(drift, vec![], "stamping drift against the manifest");
    }

    // --- each drift variant fires, on fixtures

    fn fixture() -> Manifest {
        serde_json::from_str(
            r#"{
              "schema_version": 1,
              "a_field_this_app_does_not_read": true,
              "harnesses": [
                { "id": "relay-tool", "tier": "certified",
                  "routing": { "mechanism": "relay", "proxy_domain": null, "gate_models": true },
                  "surfaces": { "connect": { "mechanism": "relay", "source": "x.rs" },
                                "client_tool_slug": "relay-tool" } },
                { "id": "engine-tool", "tier": "certified",
                  "routing": { "mechanism": "proxy-engine", "proxy_domain": null },
                  "surfaces": { "connect": { "mechanism": "proxy-engine", "source": "y.rs" },
                                "client_tool_slug": "engine-tool" } },
                { "id": "desktop-app", "tier": "works",
                  "routing": { "mechanism": "proxy-engine", "proxy_domain": "example" },
                  "surfaces": { "connect": { "na": "carried by the domain toggle" },
                                "client_tool_slug": { "na": "no integration" } } },
                { "id": "manual-tool", "tier": "unsupported",
                  "routing": { "mechanism": "manual", "proxy_domain": null },
                  "surfaces": { "connect": null, "client_tool_slug": null } }
              ]
            }"#,
        )
        .unwrap()
    }

    fn tool(slug: &str, mechanism: Mechanism, gate_models: bool) -> ToolRouting {
        ToolRouting {
            slug: slug.to_string(),
            mechanism,
            gate_models,
        }
    }

    fn agreeing_tools() -> Vec<ToolRouting> {
        vec![
            tool("relay-tool", Mechanism::Relay, true),
            tool("engine-tool", Mechanism::ForwardProxy, false),
            tool("env-proxy", Mechanism::Environment, false),
        ]
    }

    #[test]
    fn a_fixture_that_agrees_reports_no_drift() {
        let m = fixture();
        assert_eq!(check_routing(&m, &agreeing_tools()), vec![]);
        assert_eq!(check_domain_slugs(&m, &["example"]), vec![]);
        assert_eq!(
            check_stamping_names(&m, &["relay-tool", "engine-tool"]),
            vec![]
        );
    }

    #[test]
    fn a_changed_mechanism_is_routing_drift_on_both_fields() {
        let mut tools = agreeing_tools();
        tools[1].mechanism = Mechanism::Relay;
        let drift = check_routing(&fixture(), &tools);
        assert_eq!(
            drift,
            vec![
                Drift::Routing {
                    harness: "engine-tool".into(),
                    field: "routing.mechanism",
                    manifest: "proxy-engine".into(),
                    code: "relay",
                },
                Drift::Routing {
                    harness: "engine-tool".into(),
                    field: "surfaces.connect.mechanism",
                    manifest: "proxy-engine".into(),
                    code: "relay",
                },
            ]
        );
    }

    #[test]
    fn gate_models_support_must_agree_both_ways() {
        let mut tools = agreeing_tools();
        tools[0].gate_models = false;
        tools[1].gate_models = true;
        assert_eq!(
            check_routing(&fixture(), &tools),
            vec![
                Drift::GateModels {
                    harness: "relay-tool".into(),
                    manifest: true,
                    code: false,
                },
                Drift::GateModels {
                    harness: "engine-tool".into(),
                    manifest: false,
                    code: true,
                },
            ]
        );
    }

    #[test]
    fn an_integration_with_no_entry_and_an_entry_with_no_integration_both_fail() {
        let mut tools = agreeing_tools();
        tools.remove(1);
        tools.push(tool("new-tool", Mechanism::Relay, false));
        assert_eq!(
            check_routing(&fixture(), &tools),
            vec![
                Drift::IntegrationNotInManifest {
                    tool: "new-tool".into()
                },
                Drift::ManifestIntegrationMissing {
                    harness: "engine-tool".into()
                },
            ]
        );
    }

    #[test]
    fn an_integration_the_manifest_does_not_route_through_gate_connect_fails() {
        let mut tools = agreeing_tools();
        tools.push(tool("manual-tool", Mechanism::Relay, false));
        let drift = check_routing(&fixture(), &tools);
        assert!(drift.contains(&Drift::Routing {
            harness: "manual-tool".into(),
            field: "surfaces.connect",
            manifest: "no integration".into(),
            code: "relay",
        }));
    }

    #[test]
    fn the_environment_channel_needs_no_entry() {
        let tools = vec![tool("env-proxy", Mechanism::Environment, false)];
        let drift = check_routing(&fixture(), &tools);
        assert!(!drift
            .iter()
            .any(|d| matches!(d, Drift::IntegrationNotInManifest { .. })));
    }

    #[test]
    fn a_domain_on_one_side_only_fails_naming_it() {
        assert_eq!(
            check_domain_slugs(&fixture(), &["example", "added"]),
            vec![Drift::DomainNotInManifest {
                slug: "added".into()
            }]
        );
        assert_eq!(
            check_domain_slugs(&fixture(), &[]),
            vec![Drift::DomainNotInCatalog {
                harness: "desktop-app".into(),
                slug: "example".into()
            }]
        );
    }

    #[test]
    fn a_domain_named_only_below_works_is_not_in_the_manifest() {
        let mut m = fixture();
        m.harnesses[2].tier = "unsupported".into();
        assert_eq!(
            check_domain_slugs(&m, &["example"]),
            vec![Drift::DomainNotInManifest {
                slug: "example".into()
            }]
        );
    }

    #[test]
    fn a_stamped_name_on_one_side_only_fails_naming_it() {
        assert_eq!(
            check_stamping_names(&fixture(), &["relay-tool", "renamed-tool"]),
            vec![
                Drift::StampNotInManifest {
                    slug: "renamed-tool".into()
                },
                Drift::StampMissing {
                    harness: "engine-tool".into(),
                    slug: "engine-tool".into()
                },
            ]
        );
    }
}
