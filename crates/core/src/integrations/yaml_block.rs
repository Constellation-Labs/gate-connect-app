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
    /// The key appears twice at the same level. PyYAML keeps the LAST one, so
    /// editing the first would change nothing Hermes reads - refused rather
    /// than guessed at.
    DuplicateKey,
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
    let eol = eol_of(body);

    if lines.iter().filter(|l| opens_block(l, parent, 0).is_some()).count() > 1 {
        return Err(Refusal::DuplicateKey);
    }
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
            out.push_str(eol);
        }
        out.push_str(&format!("{parent}:\n  {child}:\n    {key}: {value}\n").replace('\n', eol));
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
            join(out, trailing_newline, eol),
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
        let comment = trailing_comment(rest).unwrap_or("");
        out[child_open + 1 + i] = format!("{}{key}: {value}{comment}", " ".repeat(key_indent));
        return Ok((join(out, trailing_newline, eol), Edit::Refreshed));
    }

    out.insert(
        child_open + 1,
        format!("{}{key}: {value}", " ".repeat(key_indent)),
    );
    Ok((
        join(out, trailing_newline, eol),
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
    let eol = eol_of(body);
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
    join(kept, trailing_newline, eol)
}

/// How a top-level key sits in the document before an edit, so the edit can be
/// taken back to exactly that shape.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub(crate) enum ParentShape {
    /// No `<parent>:` line at all.
    #[default]
    Absent,
    /// A block, `<parent>:` with children (or none yet).
    Block,
    /// An inline empty value: `{}`, `""`, `''`, `~` or `null`. A fresh Hermes
    /// install ships `model: ""`. Kept verbatim so a restore writes back the
    /// line the user had, not a normalised one.
    Empty(String),
}

/// Values that mean "nothing here" when a key carries them inline.
const EMPTY_INLINE: &[&str] = &["{}", "\"\"", "''", "~", "null"];

/// The shape of `<parent>` in `body`, or a refusal for an inline value this
/// module will not edit into (`model: {a: 1}`, `model: gpt-4o`).
pub(crate) fn parent_shape(body: &str, parent: &str) -> Result<ParentShape, Refusal> {
    let top_level = body
        .lines()
        .filter(|l| {
            let (lead, rest) = split_indent(l);
            lead == 0 && rest.strip_prefix(parent).is_some_and(|r| r.starts_with(':'))
        })
        .count();
    if top_level > 1 {
        return Err(Refusal::DuplicateKey);
    }
    for line in body.lines() {
        let (lead, rest) = split_indent(line);
        if lead != 0 {
            continue;
        }
        let Some(after) = rest.strip_prefix(parent).and_then(|r| r.strip_prefix(':')) else {
            continue;
        };
        let value = strip_comment(after.trim());
        if value.is_empty() {
            return Ok(ParentShape::Block);
        }
        if EMPTY_INLINE.contains(&value) {
            return Ok(ParentShape::Empty(line.to_string()));
        }
        return Err(Refusal::ParentIsNotABlock);
    }
    Ok(ParentShape::Absent)
}

/// A plain scalar with any trailing ` # comment` removed.
fn strip_comment(value: &str) -> &str {
    if value.starts_with(['"', '\'']) {
        return value;
    }
    match value.find(" #") {
        Some(i) => value[..i].trim_end(),
        None => value,
    }
}

/// Read a plain scalar back, unquoting it.
fn plain_value(raw: &str) -> Result<String, Refusal> {
    let v = strip_comment(raw.trim());
    if v.starts_with(['&', '*', '|', '>', '[', '{']) {
        return Err(Refusal::ValueNotPlain);
    }
    let unquoted = if (v.starts_with('"') && v.ends_with('"') && v.len() >= 2)
        || (v.starts_with('\'') && v.ends_with('\'') && v.len() >= 2)
    {
        &v[1..v.len() - 1]
    } else {
        v
    };
    Ok(unquoted.to_string())
}

/// Make `<parent>` a block, returning the lines and where it opens. An absent
/// parent is appended; an inline empty one is rewritten to `<parent>:`.
fn ensure_parent(lines: &mut Vec<String>, parent: &str, shape: &ParentShape) -> usize {
    match shape {
        ParentShape::Absent => {
            lines.push(format!("{parent}:"));
            lines.len() - 1
        }
        ParentShape::Empty(original) => {
            let at = lines
                .iter()
                .position(|l| l == original)
                .expect("shape was read from these lines");
            lines[at] = format!("{parent}:");
            at
        }
        ParentShape::Block => lines
            .iter()
            .position(|l| opens_block(l, parent, 0) == Some(true))
            .expect("shape was read from these lines"),
    }
}

/// The line index of `<key>:` directly under the block opened at `open`, with
/// the block's end and child indent.
fn find_child(lines: &[&str], open: usize, key: &str) -> (Option<usize>, usize, usize) {
    let (end, child_indent) = block_extent(lines, open, 0);
    let indent = child_indent.unwrap_or(2);
    let at = lines[open + 1..end]
        .iter()
        .position(|l| {
            let (lead, rest) = split_indent(l);
            lead == indent && rest.strip_prefix(key).is_some_and(|r| r.starts_with(':'))
        })
        .map(|p| open + 1 + p);
    (at, end, indent)
}

/// Is `<parent>.<key>` present, whatever it holds?
pub(crate) fn has_child(body: &str, parent: &str, key: &str) -> bool {
    let lines: Vec<&str> = body.lines().collect();
    lines
        .iter()
        .position(|l| opens_block(l, parent, 0) == Some(true))
        .is_some_and(|open| find_child(&lines, open, key).0.is_some())
}

/// `<parent>.<key>` as a plain scalar, or `None` when it is not set.
pub(crate) fn get_child(body: &str, parent: &str, key: &str) -> Result<Option<String>, Refusal> {
    let lines: Vec<&str> = body.lines().collect();
    let Some(open) = lines
        .iter()
        .position(|l| opens_block(l, parent, 0) == Some(true))
    else {
        return Ok(None);
    };
    if child_count(&lines, open, key) > 1 {
        return Err(Refusal::DuplicateKey);
    }
    let (at, _, _) = find_child(&lines, open, key);
    let Some(at) = at else {
        return Ok(None);
    };
    let (_, rest) = split_indent(lines[at]);
    let raw = rest[key.len() + 1..].trim();
    if raw.is_empty() {
        // `key:` with a block under it: not a scalar.
        return Err(Refusal::ValueNotPlain);
    }
    plain_value(raw).map(Some)
}

/// Set `<parent>.<key>` to a plain scalar, or remove it with `None`.
///
/// Values are written unquoted when they can be and double-quoted otherwise.
/// Removing a key never removes its parent: that is [`tidy_parent`]'s job,
/// which knows what the parent looked like before Gate arrived.
pub(crate) fn set_child(
    body: &str,
    parent: &str,
    key: &str,
    value: Option<&str>,
) -> Result<String, Refusal> {
    let trailing_newline = body.is_empty() || body.ends_with('\n');
    let eol = eol_of(body);
    let shape = parent_shape(body, parent)?;
    if value.is_none() && shape != ParentShape::Block {
        return Ok(body.to_string());
    }
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let open = ensure_parent(&mut lines, parent, &shape);
    let view: Vec<&str> = lines.iter().map(String::as_str).collect();
    if child_count(&view, open, key) > 1 {
        return Err(Refusal::DuplicateKey);
    }
    let (at, _, indent) = find_child(&view, open, key);
    let mut comment = String::new();
    if let Some(at) = at {
        let (_, rest) = split_indent(view[at]);
        let raw = rest[key.len() + 1..].trim();
        if raw.is_empty() {
            return Err(Refusal::ValueNotPlain);
        }
        plain_value(raw)?;
        comment = trailing_comment(raw).unwrap_or("").to_string();
    }
    match (at, value) {
        (Some(at), Some(v)) => {
            lines[at] = format!("{}{key}: {}{comment}", " ".repeat(indent), scalar(v))
        }
        (None, Some(v)) => lines.insert(
            open + 1,
            format!("{}{key}: {}", " ".repeat(indent), scalar(v)),
        ),
        (Some(at), None) => {
            lines.remove(at);
        }
        (None, None) => {}
    }
    Ok(join(lines, trailing_newline, eol))
}

/// Replace the whole `<parent>.<key>` subtree with `block`, or remove it with
/// `None`.
///
/// `block` is the subtree's body, one line per entry and indented relative to
/// the key's own children (so a nested list item is `"  - a/b"`); the `<key>:`
/// line and the indentation under it are written here. Whatever was under the key is replaced wholesale, which is the point:
/// the subtree is Gate's own, and a tool that wrote into it (Hermes adds
/// `default_model`) is overwritten on the next apply rather than merged with.
pub(crate) fn set_block(
    body: &str,
    parent: &str,
    key: &str,
    block: Option<&[String]>,
) -> Result<String, Refusal> {
    let trailing_newline = body.is_empty() || body.ends_with('\n');
    let eol = eol_of(body);
    let shape = parent_shape(body, parent)?;
    if block.is_none() && shape != ParentShape::Block {
        return Ok(body.to_string());
    }
    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    let open = ensure_parent(&mut lines, parent, &shape);
    let view: Vec<&str> = lines.iter().map(String::as_str).collect();
    if child_count(&view, open, key) > 1 {
        return Err(Refusal::DuplicateKey);
    }
    let (at, _, indent) = find_child(&view, open, key);
    // The existing subtree: the key line and everything indented past it.
    let span = at.map(|at| {
        let (end, _) = block_extent(&view, at, indent);
        at..end
    });
    let rendered: Option<Vec<String>> = block.map(|b| {
        let pad = " ".repeat(indent);
        std::iter::once(format!("{pad}{key}:"))
            .chain(b.iter().map(|l| format!("{pad}  {l}")))
            .collect()
    });
    match (span, rendered) {
        (Some(span), Some(new)) => {
            lines.splice(span, new);
        }
        (None, Some(new)) => {
            let at = open + 1;
            for (i, l) in new.into_iter().enumerate() {
                lines.insert(at + i, l);
            }
        }
        (Some(span), None) => {
            lines.drain(span);
        }
        (None, None) => {}
    }
    Ok(join(lines, trailing_newline, eol))
}

/// Put `<parent>` back to how it was before Gate edited under it, if Gate's
/// edits have left it with nothing of its own: removed when it was absent, the
/// original inline line when it was empty. A parent that still has children is
/// the user's and stays.
pub(crate) fn tidy_parent(body: &str, parent: &str, before: &ParentShape) -> String {
    if *before == ParentShape::Block {
        return body.to_string();
    }
    let trailing_newline = body.is_empty() || body.ends_with('\n');
    let eol = eol_of(body);
    let lines: Vec<&str> = body.lines().collect();
    let Some(open) = lines
        .iter()
        .position(|l| opens_block(l, parent, 0) == Some(true))
    else {
        return body.to_string();
    };
    let (_, child_indent) = block_extent(&lines, open, 0);
    if child_indent.is_some() {
        return body.to_string();
    }
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
    match before {
        ParentShape::Absent => {
            out.remove(open);
        }
        ParentShape::Empty(original) => out[open] = original.clone(),
        ParentShape::Block => {}
    }
    join(out, trailing_newline, eol)
}

/// A scalar as YAML will read it back unchanged: bare when that is safe, double
/// quoted otherwise.
fn scalar(v: &str) -> String {
    let bare_ok = !v.is_empty()
        && !v.starts_with([
            ' ', '&', '*', '|', '>', '[', '{', '"', '\'', '#', '!', '%', '@', '`', '-', '?', ',',
        ])
        && !v.ends_with([' ', ':'])
        && !v.contains(": ")
        && !v.contains(" #")
        && !EMPTY_INLINE.contains(&v)
        && !matches!(v, "true" | "false" | "yes" | "no" | "on" | "off");
    if bare_ok {
        v.to_string()
    } else {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// The file's own line ending. `str::lines` strips a `\r` before each `\n`,
/// so an edit that joined on `\n` alone turned a CRLF file into an LF one.
fn eol_of(body: &str) -> &'static str {
    if body.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

fn join(lines: Vec<String>, trailing_newline: bool, eol: &str) -> String {
    // An inserted entry may carry its own `\n`s (a new block and its key in one
    // element); they take the file's ending too.
    let lines: Vec<String> = if eol == "\n" {
        lines
    } else {
        lines.into_iter().map(|l| l.replace('\n', eol)).collect()
    };
    let mut out = lines.join(eol);
    if trailing_newline && !out.is_empty() {
        out.push_str(eol);
    }
    out
}

/// A value's trailing ` # comment`, if it has one, so a refresh can keep it.
/// Quoted values are skipped past their closing quote first: a `#` inside the
/// quotes is part of the value.
fn trailing_comment(raw: &str) -> Option<&str> {
    let raw = raw.trim_end();
    let start = match raw.chars().next() {
        Some(q @ ('"' | '\'')) => raw[1..].find(q).map(|i| i + 2)?,
        _ => 0,
    };
    let hash = raw[start..].find(" #").map(|i| start + i + 1)?;
    // Back to the start of the whitespace run, so `  # note` keeps its gap.
    let from = raw[..hash].trim_end().len();
    Some(&raw[from..])
}

/// How many times `<key>:` appears directly under the block opened at `open`.
fn child_count(lines: &[&str], open: usize, key: &str) -> usize {
    let (end, child_indent) = block_extent(lines, open, 0);
    let indent = child_indent.unwrap_or(2);
    lines[open + 1..end]
        .iter()
        .filter(|l| {
            let (lead, rest) = split_indent(l);
            lead == indent && rest.strip_prefix(key).is_some_and(|r| r.starts_with(':'))
        })
        .count()
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

    #[test]
    fn a_child_scalar_is_read_set_and_removed_without_touching_anything_else() {
        let before = "# top\nmodel:\n  # which one\n  default: z-ai/glm-5.2  # mine\n  provider: openrouter\ntools: {}\n";
        assert_eq!(
            get_child(before, "model", "default"),
            Ok(Some("z-ai/glm-5.2".into()))
        );
        assert_eq!(get_child(before, "model", "base_url"), Ok(None));
        let set = set_child(before, "model", "provider", Some("gate-connect")).unwrap();
        assert_eq!(set, "# top\nmodel:\n  # which one\n  default: z-ai/glm-5.2  # mine\n  provider: gate-connect\ntools: {}\n");
        let added = set_child(&set, "model", "api_mode", Some("chat_completions")).unwrap();
        assert!(added.contains("model:\n  api_mode: chat_completions\n  # which one"));
        let removed = set_child(&added, "model", "api_mode", None).unwrap();
        assert_eq!(removed, set);
    }

    #[test]
    fn an_empty_inline_parent_is_opened_and_put_back_exactly() {
        let before = "model: \"\"\nother: 1\n";
        let shape = parent_shape(before, "model").unwrap();
        assert_eq!(shape, ParentShape::Empty("model: \"\"".into()));
        let set = set_child(before, "model", "default", Some("openai/gpt-5.6-luna")).unwrap();
        assert_eq!(set, "model:\n  default: openai/gpt-5.6-luna\nother: 1\n");
        let cleared = set_child(&set, "model", "default", None).unwrap();
        assert_eq!(tidy_parent(&cleared, "model", &shape), before);
    }

    #[test]
    fn an_absent_parent_is_appended_and_removed() {
        let before = "tools:\n  a: 1\n";
        let shape = parent_shape(before, "providers").unwrap();
        let block = vec![
            "name: Gate Connect".to_string(),
            "models:".into(),
            "  - a/b".into(),
        ];
        let set = set_block(before, "providers", "gate-connect", Some(&block)).unwrap();
        assert_eq!(set, "tools:\n  a: 1\nproviders:\n  gate-connect:\n    name: Gate Connect\n    models:\n      - a/b\n");
        let removed = set_block(&set, "providers", "gate-connect", None).unwrap();
        assert_eq!(tidy_parent(&removed, "providers", &shape), before);
    }

    #[test]
    fn a_block_is_replaced_wholesale_and_neighbours_survive() {
        let before = "providers:\n  mine:\n    api: http://x\n  gate-connect:\n    name: Old\n    default_model: a/b\n    models:\n      - a/b\n  after:\n    api: http://y\n";
        let block = vec![
            "name: Gate Connect".to_string(),
            "models:".into(),
            "  - c/d".into(),
        ];
        let set = set_block(before, "providers", "gate-connect", Some(&block)).unwrap();
        assert_eq!(set, "providers:\n  mine:\n    api: http://x\n  gate-connect:\n    name: Gate Connect\n    models:\n      - c/d\n  after:\n    api: http://y\n");
        let removed = set_block(&set, "providers", "gate-connect", None).unwrap();
        assert_eq!(
            removed,
            "providers:\n  mine:\n    api: http://x\n  after:\n    api: http://y\n"
        );
        assert_eq!(
            tidy_parent(&removed, "providers", &ParentShape::Block),
            removed
        );
    }

    #[test]
    fn shapes_it_cannot_edit_are_refused() {
        assert_eq!(
            parent_shape("model: gpt-4o\n", "model"),
            Err(Refusal::ParentIsNotABlock)
        );
        assert_eq!(
            set_child("model:\n  default: &a x\n", "model", "default", Some("y")),
            Err(Refusal::ValueNotPlain)
        );
        assert_eq!(
            get_child("model:\n  default:\n    nested: 1\n", "model", "default"),
            Err(Refusal::ValueNotPlain)
        );
    }

    #[test]
    fn scalars_are_quoted_only_when_yaml_would_misread_them() {
        assert_eq!(scalar("openai/gpt-5.6-luna"), "openai/gpt-5.6-luna");
        assert_eq!(
            scalar("http://127.0.0.1:1/__gate/t/hermes/gate/v1"),
            "http://127.0.0.1:1/__gate/t/hermes/gate/v1"
        );
        assert_eq!(scalar("a: b"), "\"a: b\"");
        assert_eq!(scalar("true"), "\"true\"");
        assert_eq!(scalar(""), "\"\"");
    }

    #[test]
    fn a_crlf_file_stays_crlf() {
        let before = "model:\r\n  default: a/b\r\n  provider: x\r\nother: 1\r\n";
        let after = set_child(before, "model", "default", Some("c/d")).unwrap();
        assert_eq!(after, "model:\r\n  default: c/d\r\n  provider: x\r\nother: 1\r\n");
        let (nested, _) = set_nested(before, "model", "extra_headers", "k", "v").unwrap();
        assert!(!nested.replace("\r\n", "").contains('\n'), "{nested:?}");
    }

    #[test]
    fn a_duplicated_key_is_refused_not_half_edited() {
        let dup_parent = "model:\n  default: a/b\nmodel:\n  default: c/d\n";
        assert_eq!(parent_shape(dup_parent, "model"), Err(Refusal::DuplicateKey));
        assert_eq!(
            set_child(dup_parent, "model", "default", Some("x/y")),
            Err(Refusal::DuplicateKey)
        );
        assert_eq!(
            set_nested(dup_parent, "model", "extra_headers", "k", "v").map(|_| ()),
            Err(Refusal::DuplicateKey)
        );
        let dup_child = "model:\n  default: a/b\n  default: c/d\n";
        assert_eq!(get_child(dup_child, "model", "default"), Err(Refusal::DuplicateKey));
        assert_eq!(
            set_child(dup_child, "model", "default", Some("x/y")),
            Err(Refusal::DuplicateKey)
        );
    }

    #[test]
    fn a_refreshed_value_keeps_its_comment() {
        let before = "model:\n  default: a/b  # my pick\n  provider: \"x # y\"\n";
        let after = set_child(before, "model", "default", Some("c/d")).unwrap();
        assert_eq!(after, "model:\n  default: c/d  # my pick\n  provider: \"x # y\"\n");
        let after = set_child(before, "model", "provider", Some("z")).unwrap();
        assert!(after.contains("  provider: z\n"), "a # inside quotes is not a comment: {after}");
        let nested = "model:\n  extra_headers:\n    k: old  # keep\n";
        let (after, _) = set_nested(nested, "model", "extra_headers", "k", "new").unwrap();
        assert_eq!(after, "model:\n  extra_headers:\n    k: new  # keep\n");
    }
}

