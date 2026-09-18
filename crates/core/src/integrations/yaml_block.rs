//! A deliberately small YAML editor for one nested key in a file we do not own.
//!
//! Hermes' `config.yaml` is hand-written and heavily commented - the upstream
//! example ships more comment than config - so the obvious implementation, read
//! it with `serde_yaml` and write it back, is not available: a round-trip
//! returns a semantically equal document with every comment and every blank
//! line gone. The user would open their config after connecting and find it
//! stripped, which is a worse outcome than not being attributed.
//!
//! So this edits text and leaves every byte it did not come for alone. The
//! tradeoff is that it understands far less YAML than a parser does, and the
//! rule that makes that safe is: **anything it does not recognise, it refuses.**
//! A refusal costs an attribution label. A wrong edit costs the user's config.

use anyhow::Result;

/// What [`set_nested`] had to create, so [`remove_nested`] can take exactly
/// that back out and no more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Created {
    /// The top-level `<parent>:` line did not exist.
    #[serde(default)]
    pub parent: bool,
    /// The `<child>:` block under it did not exist.
    #[serde(default)]
    pub child: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Edit {
    /// The key was inserted; `created` says what scaffolding came with it.
    Inserted(Created),
    /// The key was there with a different value and now holds ours.
    Refreshed,
    /// Already exactly right; the file was not touched.
    Unchanged,
}

/// Why an edit was refused. Each variant is a document shape this module
/// declines to guess at rather than a malformed file.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// `<parent>:` carries a value on its own line (`model: {a: 1}` or
    /// `model: something`), so the block form this inserts into is not what the
    /// document uses.
    ParentIsNotABlock,
    /// Same, one level down.
    ChildIsNotABlock,
    /// The key is present but the line is not a plain `key: value` we can read
    /// back - an anchor, a multi-line scalar, a flow collection.
    ValueNotPlain,
}

/// Is `line` the block opener `name:` at exactly `indent` spaces?
///
/// A trailing comment is allowed because it is common and harmless; anything
/// else after the colon means the key has an inline value, which is the shape
/// this module refuses rather than edits.
fn opens_block(line: &str, name: &str, indent: usize) -> Option<bool> {
    let (lead, rest) = split_indent(line);
    if lead != indent {
        return None;
    }
    let rest = rest.strip_prefix(name)?.strip_prefix(':')?;
    let after = rest.trim_start();
    Some(after.is_empty() || after.starts_with('#'))
}

fn split_indent(line: &str) -> (usize, &str) {
    let trimmed = line.trim_start_matches(' ');
    (line.len() - trimmed.len(), trimmed)
}

/// True for a line that carries no structure of its own, so it can sit inside a
/// block without ending it.
fn is_filler(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// The half-open line range of the block opened at `open`, and the indent its
/// children use. `None` for the indent when the block has no children yet.
fn block_extent(lines: &[&str], open: usize, indent: usize) -> (usize, Option<usize>) {
    let mut end = open + 1;
    let mut child_indent = None;
    for (offset, line) in lines[open + 1..].iter().enumerate() {
        if is_filler(line) {
            continue;
        }
        let (lead, _) = split_indent(line);
        if lead <= indent {
            break;
        }
        child_indent.get_or_insert(lead);
        end = open + 1 + offset + 1;
    }
    (end, child_indent)
}

/// Set `<parent>.<child>.<key>` to `value`, preserving the rest of the document
/// byte for byte.
pub(crate) fn set_nested(
    body: &str,
    parent: &str,
    child: &str,
    key: &str,
    value: &str,
) -> Result<(String, Edit), Refusal> {
    let lines: Vec<&str> = body.lines().collect();
    let trailing_newline = body.is_empty() || body.ends_with('\n');

    let parent_open = lines
        .iter()
        .position(|l| opens_block(l, parent, 0) == Some(true));
    // Present but not a block: `model: {...}`. Refuse rather than guess at a
    // flow mapping, which would need a real parser to edit correctly.
    if parent_open.is_none()
        && lines
            .iter()
            .any(|l| opens_block(l, parent, 0) == Some(false))
    {
        return Err(Refusal::ParentIsNotABlock);
    }

    let Some(parent_open) = parent_open else {
        // No parent at all: append the whole path. Nothing existing is touched,
        // which makes this the safest branch rather than the most complex.
        let mut out = String::from(body);
        if !out.is_empty() && !trailing_newline {
            out.push('\n');
        }
        out.push_str(&format!("{parent}:\n  {child}:\n    {key}: {value}\n"));
        return Ok((
            out,
            Edit::Inserted(Created {
                parent: true,
                child: true,
            }),
        ));
    };

    let (parent_end, parent_child_indent) = block_extent(&lines, parent_open, 0);
    let indent = parent_child_indent.unwrap_or(2);

    let child_open = lines[parent_open + 1..parent_end]
        .iter()
        .position(|l| opens_block(l, child, indent) == Some(true))
        .map(|p| parent_open + 1 + p);
    if child_open.is_none()
        && lines[parent_open + 1..parent_end]
            .iter()
            .any(|l| opens_block(l, child, indent) == Some(false))
    {
        return Err(Refusal::ChildIsNotABlock);
    }

    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();

    let Some(child_open) = child_open else {
        // Parent exists, child does not: open the child block immediately after
        // the parent line, where it cannot land inside some other key's block.
        out.insert(
            parent_open + 1,
            format!(
                "{}{child}:\n{}{key}: {value}",
                " ".repeat(indent),
                " ".repeat(indent * 2)
            ),
        );
        return Ok((
            join(out, trailing_newline),
            Edit::Inserted(Created {
                parent: false,
                child: true,
            }),
        ));
    };

    let (child_end, child_child_indent) = block_extent(&lines, child_open, indent);
    let key_indent = child_child_indent.unwrap_or(indent * 2);

    for (i, line) in lines[child_open + 1..child_end].iter().enumerate() {
        let (lead, rest) = split_indent(line);
        if lead != key_indent {
            continue;
        }
        let Some(rest) = rest.strip_prefix(key).and_then(|r| r.strip_prefix(':')) else {
            continue;
        };
        let current = rest.trim();
        // A value we cannot read back is one we must not overwrite: `&anchor`,
        // `|` and `>` blocks, and flow collections all continue past this line.
        if current.starts_with(['&', '*', '|', '>', '[', '{']) {
            return Err(Refusal::ValueNotPlain);
        }
        if current.trim_matches(['"', '\'']) == value {
            return Ok((body.to_string(), Edit::Unchanged));
        }
        out[child_open + 1 + i] = format!("{}{key}: {value}", " ".repeat(key_indent));
        return Ok((join(out, trailing_newline), Edit::Refreshed));
    }

    out.insert(
        child_open + 1,
        format!("{}{key}: {value}", " ".repeat(key_indent)),
    );
    Ok((
        join(out, trailing_newline),
        Edit::Inserted(Created::default()),
    ))
}

/// Take back exactly what [`set_nested`] added, per the `created` it reported.
pub(crate) fn remove_nested(
    body: &str,
    parent: &str,
    child: &str,
    key: &str,
    created: Created,
) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let trailing_newline = body.is_empty() || body.ends_with('\n');
    let Some(parent_open) = lines
        .iter()
        .position(|l| opens_block(l, parent, 0) == Some(true))
    else {
        return body.to_string();
    };
    let (parent_end, parent_child_indent) = block_extent(&lines, parent_open, 0);
    let indent = parent_child_indent.unwrap_or(2);
    let Some(child_open) = lines[parent_open + 1..parent_end]
        .iter()
        .position(|l| opens_block(l, child, indent) == Some(true))
        .map(|p| parent_open + 1 + p)
    else {
        return body.to_string();
    };
    let (child_end, child_child_indent) = block_extent(&lines, child_open, indent);
    let key_indent = child_child_indent.unwrap_or(indent * 2);

    // Widest scaffolding first, so the narrower cases cannot strand a block we
    // opened. `created` is the record of what was ours; anything else here was
    // the user's before we arrived and stays.
    let drop = if created.parent {
        parent_open..parent_end
    } else if created.child {
        child_open..child_end
    } else {
        let Some(at) = lines[child_open + 1..child_end].iter().position(|l| {
            let (lead, rest) = split_indent(l);
            lead == key_indent && rest.strip_prefix(key).is_some_and(|r| r.starts_with(':'))
        }) else {
            return body.to_string();
        };
        let at = child_open + 1 + at;
        at..at + 1
    };

    let kept: Vec<String> = lines
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop.contains(i))
        .map(|(_, l)| (*l).to_string())
        .collect();
    join(kept, trailing_newline)
}

fn join(lines: Vec<String>, trailing_newline: bool) -> String {
    let mut out = lines.join("\n");
    if trailing_newline && !out.is_empty() {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "x-gate-tool";

    fn set(body: &str) -> Result<(String, Edit), Refusal> {
        set_nested(body, "model", "extra_headers", KEY, "hermes")
    }

    /// The whole reason this is not a `serde_yaml` round-trip: a Hermes config
    /// is mostly comments, and they have to still be there afterwards.
    #[test]
    fn comments_and_unrelated_keys_survive_byte_for_byte() {
        let before = "\
# Hermes agent configuration.
# See cli-config.yaml.example for the full reference.

model:
  # Which upstream to talk to. Defaults to OpenRouter.
  base_url: https://openrouter.ai/api/v1
  provider: auto

tools:
  # Shell access is off by default.
  terminal: false
";
        let (after, edit) = set(before).unwrap();
        assert_eq!(
            edit,
            Edit::Inserted(Created {
                parent: false,
                child: true
            })
        );
        for line in before.lines() {
            assert!(after.contains(line), "lost {line:?} from:\n{after}");
        }
        assert!(after.contains("  extra_headers:\n    x-gate-tool: hermes"));

        // And disconnect puts it back exactly as it was.
        let restored = remove_nested(
            &after,
            "model",
            "extra_headers",
            KEY,
            Created {
                parent: false,
                child: true,
            },
        );
        assert_eq!(restored, before);
    }

    /// A header block the user already keeps gains our key and loses nothing -
    /// theirs is not ours to remove, which is also what disconnect must honour.
    #[test]
    fn an_existing_header_block_is_joined_not_replaced() {
        let before = "\
model:
  extra_headers:
    CF-Access-Client-Id: \"${CF_ID}\"
    CF-Access-Client-Secret: \"${CF_SECRET}\"
";
        let (after, edit) = set(before).unwrap();
        assert_eq!(edit, Edit::Inserted(Created::default()));
        assert!(after.contains("CF-Access-Client-Id: \"${CF_ID}\""));
        assert!(after.contains("CF-Access-Client-Secret: \"${CF_SECRET}\""));
        assert!(after.contains("    x-gate-tool: hermes"));

        let restored = remove_nested(&after, "model", "extra_headers", KEY, Created::default());
        assert_eq!(restored, before, "only our own line comes back out");
    }

    /// The migration case #275 taught: a value of ours that has gone stale is
    /// corrected, and one already right is not reported as a change.
    #[test]
    fn our_own_value_is_refreshed_and_an_identical_one_is_left_alone() {
        let stale = "model:\n  extra_headers:\n    x-gate-tool: hermes-old\n";
        let (after, edit) = set(stale).unwrap();
        assert_eq!(edit, Edit::Refreshed);
        assert!(after.contains("    x-gate-tool: hermes\n"));
        assert!(!after.contains("hermes-old"));

        let (again, edit) = set(&after).unwrap();
        assert_eq!(edit, Edit::Unchanged);
        assert_eq!(again, after);
    }

    /// A fresh install has no config at all, and an empty file is the same
    /// case: write the whole path and nothing else.
    #[test]
    fn an_absent_or_empty_document_gets_the_whole_path() {
        for before in ["", "# just a comment\n"] {
            let (after, edit) = set(before).unwrap();
            assert_eq!(
                edit,
                Edit::Inserted(Created {
                    parent: true,
                    child: true
                })
            );
            assert!(after.ends_with("model:\n  extra_headers:\n    x-gate-tool: hermes\n"));
            assert!(after.starts_with(before));

            let restored = remove_nested(
                &after,
                "model",
                "extra_headers",
                KEY,
                Created {
                    parent: true,
                    child: true,
                },
            );
            assert_eq!(restored, before);
        }
    }

    /// Indentation is the document's, not ours. A config written with four
    /// spaces must not acquire a two-space line in the middle of it.
    #[test]
    fn the_documents_own_indentation_is_followed() {
        let before = "model:\n    base_url: https://openrouter.ai/api/v1\n";
        let (after, _) = set(before).unwrap();
        assert!(
            after.contains("    extra_headers:\n        x-gate-tool: hermes"),
            "expected four-space nesting:\n{after}"
        );
    }

    /// Shapes this module does not understand are refused, not guessed at. A
    /// refusal costs an attribution label; a wrong edit costs the user's config.
    #[test]
    fn shapes_it_cannot_read_are_refused_rather_than_mangled() {
        assert_eq!(
            set("model: {base_url: https://x/v1}\n").unwrap_err(),
            Refusal::ParentIsNotABlock
        );
        assert_eq!(
            set("model:\n  extra_headers: {a: b}\n").unwrap_err(),
            Refusal::ChildIsNotABlock
        );
        // An anchor or a block scalar continues past the line we would rewrite.
        assert_eq!(
            set("model:\n  extra_headers:\n    x-gate-tool: &anchor foo\n").unwrap_err(),
            Refusal::ValueNotPlain
        );
        assert_eq!(
            set("model:\n  extra_headers:\n    x-gate-tool: |\n      multi\n").unwrap_err(),
            Refusal::ValueNotPlain
        );
    }

    /// A key that merely starts with ours is a different key.
    #[test]
    fn a_similarly_named_key_is_not_ours() {
        let before = "model:\n  extra_headers:\n    x-gate-tool-version: \"2\"\n";
        let (after, edit) = set(before).unwrap();
        assert_eq!(edit, Edit::Inserted(Created::default()));
        assert!(after.contains("x-gate-tool-version: \"2\""));
        assert!(after.contains("    x-gate-tool: hermes"));
    }
}
