//! What outranks the file Gate writes.
//!
//! Every integration writes one configuration file and then reads it back to
//! say whether the tool is connected. That reading answers "is our value on
//! disk", which is not the same question as "is this what the harness loads":
//! each of these tools merges several layers, and the one Gate is allowed to
//! edit is rarely the top of the stack. When a higher layer sets the same key,
//! the tool's traffic goes where that layer says and our file is inert - and
//! before AG-674 every one of those cases reported `Connected`.
//!
//! This module is the vocabulary for saying so. It holds no policy: which
//! layers exist, and which key inside one displaces which, is per tool and
//! lives with that tool's integration. What is shared is the shape of the
//! answer, so five integrations describe the same situation the same way.
//!
//! **What this can and cannot see.** Gate Connect is a windowed process; the
//! harness runs in the user's terminal. So a layer is visible here when it is a
//! file at a path that does not depend on where the user is standing - a
//! machine-wide managed config, an env var in our own login environment - and
//! invisible when it does. The one that matters and stays invisible is the
//! project-level config (`./opencode.json`, `.claude/settings.json` in a repo):
//! it is chosen by the harness's working directory, which nothing here knows.
//! Each integration says in its own comment which of its layers it can reach.

use std::fmt;

/// One configuration layer that outranks Gate's write, in the terms the person
/// reading the status line needs: where the winning value lives, and what it
/// does to ours.
///
/// The source is not optional. An override the user cannot locate is a status
/// line that says their traffic is not governed and offers nowhere to go, which
/// is barely better than the green pill it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Override {
    /// Where the winning value lives. A path where there is one, otherwise the
    /// name of the mechanism ("the OPENCODE_CONFIG_CONTENT environment
    /// variable") - never a bare "somewhere else".
    pub source: String,
    /// What that layer says, and what it displaces. One clause, no period: it
    /// is read after the source in a sentence this type assembles.
    pub detail: String,
}

impl fmt::Display for Override {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.source, self.detail)
    }
}

impl Override {
    pub(crate) fn new(source: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            detail: detail.into(),
        }
    }

    /// The payload for [`crate::registry::Status::Overridden`].
    pub(crate) fn into_status(self) -> crate::registry::Status {
        crate::registry::Status::Overridden(self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_and_its_clause_read_as_one_sentence() {
        let o = Override::new(
            "/etc/opencode/opencode.json",
            "points provider \"anthropic\" at \"https://api.anthropic.com\"",
        );
        assert_eq!(
            o.to_string(),
            "/etc/opencode/opencode.json points provider \"anthropic\" at \
             \"https://api.anthropic.com\""
        );
    }
}
