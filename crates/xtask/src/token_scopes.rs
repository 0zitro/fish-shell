//! The documented colour variables, generated from the declaration.
//!
//! The declaration states every scope, the variable that colours it and what it is;
//! the documentation's lists of variables are that declaration rendered, so a scope
//! cannot exist without a line and a line cannot name a scope that does not exist.

use std::path::Path;

use anyhow::{Context, Result, bail};
use fish_token_scope::{HighlightAttribute, TOKEN_SCOPE_ENTRIES};
use fish_widestring::WExt as _;

/// The documentation the generated blocks are written into.
const DOCUMENTATION: &str = "doc_src/interactive.rst";

/// The line that opens or closes a generated block.
fn marker(position: &str, block: &str) -> String {
    format!(".. {position} GENERATED {block}")
}

/// One generated block: what it documents, and the lines it renders to.
struct Block {
    /// The name the block's markers carry.
    name: &'static str,
    /// The lines between the markers.
    lines: Vec<String>,
}

impl Block {
    /// This block with its markers, as the lines the documentation holds.
    fn render(&self) -> Vec<String> {
        let mut lines = vec![marker("BEGIN", self.name)];
        lines.extend(self.lines.iter().cloned());
        lines.push(marker("END", self.name));
        lines
    }
}

/// One documented variable: its name and what it is.
struct Documented {
    variable: String,
    description: &'static str,
}

/// The variables the declaration declares in one namespace, in declaration order.
fn documented_in(namespace: &str) -> Vec<Documented> {
    TOKEN_SCOPE_ENTRIES
        .iter()
        .filter(|entry| entry.variable_name.as_ref().starts_with(namespace))
        .map(|entry| Documented {
            variable: entry.variable_name.as_ref().to_string(),
            description: entry.description,
        })
        .collect()
}

/// The variables the attributes declare, in the order they are applied.
fn documented_attributes() -> Vec<Documented> {
    HighlightAttribute::entries()
        .iter()
        .map(|entry| Documented {
            variable: entry.variable_name.as_ref().to_string(),
            description: entry.description,
        })
        .collect()
}

/// Every block the declaration generates.
fn blocks() -> Vec<Block> {
    let mut colour = documented_in("fish_color_");
    colour.extend(documented_attributes());

    vec![
        Block {
            name: "colour variables",
            lines: rendered_lines(colour),
        },
        Block {
            name: "pager variables",
            lines: rendered_lines(documented_in("fish_pager_color_")),
        },
    ]
}

/// The lines one variable list renders to.
fn rendered_lines(documented: Vec<Documented>) -> Vec<String> {
    let mut lines = Vec::with_capacity(documented.len() * 2);
    for documented in documented {
        lines.push(format!(".. envvar:: {}", documented.variable));
        lines.push(format!("    {}", documented.description));
    }
    lines
}

/// The documentation, with each generated block replaced by what the declaration
/// renders.
fn updated() -> Result<String> {
    let current = std::fs::read_to_string(DOCUMENTATION)
        .with_context(|| format!("reading {DOCUMENTATION}"))?;

    let mut lines: Vec<String> = current.lines().map(str::to_owned).collect();
    for block in blocks() {
        replace_block(&mut lines, &block)?;
    }

    let mut updated = lines.join("\n");
    updated.push('\n');
    Ok(updated)
}

/// Write the generated blocks into the documentation.
pub fn update() -> Result<()> {
    let updated = updated()?;
    std::fs::write(Path::new(DOCUMENTATION), updated)
        .with_context(|| format!("writing {DOCUMENTATION}"))
}

/// Fail unless the documentation states what the declaration renders.
pub fn check() -> Result<()> {
    let updated = updated()?;
    let current = std::fs::read_to_string(DOCUMENTATION)
        .with_context(|| format!("reading {DOCUMENTATION}"))?;
    if current != updated {
        bail!("{DOCUMENTATION} does not hold the declared scopes; run `cargo xtask token-scopes`");
    }
    Ok(())
}

/// Replace one block's lines, between its markers.
fn replace_block(lines: &mut Vec<String>, block: &Block) -> Result<()> {
    let begin = marker("BEGIN", block.name);
    let end = marker("END", block.name);

    let Some(begin_index) = lines.iter().position(|line| *line == begin) else {
        bail!("{DOCUMENTATION} holds no `{begin}` line");
    };
    let Some(end_offset) = lines[begin_index..].iter().position(|line| *line == end) else {
        bail!("{DOCUMENTATION} holds no `{end}` line after `{begin}`");
    };

    lines.splice(begin_index..=begin_index + end_offset, block.render());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Block, blocks, marker, replace_block};

    #[test]
    fn a_block_replaces_what_its_markers_hold() {
        let mut lines = vec![
            "before".to_owned(),
            marker("BEGIN", "scopes"),
            "stale".to_owned(),
            marker("END", "scopes"),
            "after".to_owned(),
        ];
        let block = Block {
            name: "scopes",
            lines: vec!["fresh".to_owned()],
        };

        replace_block(&mut lines, &block).expect("the markers are there");

        assert_eq!(
            lines,
            vec![
                "before".to_owned(),
                marker("BEGIN", "scopes"),
                "fresh".to_owned(),
                marker("END", "scopes"),
                "after".to_owned(),
            ]
        );
    }

    #[test]
    fn every_declared_variable_is_documented() {
        let documented: Vec<String> = blocks()
            .into_iter()
            .flat_map(|block| block.lines)
            .filter(|line| line.starts_with(".. envvar:: "))
            .collect();
        let declared = fish_token_scope::TOKEN_SCOPE_ENTRIES.len()
            + fish_token_scope::HighlightAttribute::entries().len();
        assert_eq!(documented.len(), declared);
    }
}
