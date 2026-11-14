//! The documented colour variables, generated from the declaration.
//!
//! The declaration states every scope, the variable that colours it and what it is;
//! the documentation's lists of variables are that declaration rendered, so a scope
//! cannot exist without a line and a line cannot name a scope that does not exist.

use std::path::Path;

use anyhow::{Context, Result, bail};
use fish_token_scope::{
    AttributeMode, HighlightAttribute, ResolutionChainStep, StyleField, TOKEN_SCOPE_ENTRIES,
    TokenScopeEntry,
};
use fish_widestring::WExt as _;

/// The documentation the generated blocks are written into.
const DOCUMENTATION: &str = "doc_src/interactive.rst";

/// The page's list of colour variables, generated from the declaration.
const COLOUR_VARIABLES_SCRIPT: &str = "share/tools/web_config/js/colour_variables.js";

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

/// One documented variable: its name, the scope it stands for and what it is.
struct Documented {
    variable: String,
    /// The scope's path, which the highlighting pass reports; a variable that stands
    /// for an attribute has none.
    path: Option<&'static str>,
    description: &'static str,
    /// The variables the scope resolves through, most specific first and ending at
    /// the root.  The page composes a character's style from them, as the shell does.
    chain: Vec<String>,
}

/// The variables the declaration declares in one namespace, in declaration order.
fn documented_in(namespace: &str) -> Vec<Documented> {
    TOKEN_SCOPE_ENTRIES
        .iter()
        .filter(|entry| entry.variable_name.as_ref().starts_with(namespace))
        .map(|entry| Documented {
            variable: entry.variable_name.as_ref().to_string(),
            path: Some(entry.path),
            description: entry.description,
            chain: resolution_chain_variables(entry),
        })
        .collect()
}

/// The variables a scope resolves through, most specific first.
fn resolution_chain_variables(entry: &TokenScopeEntry) -> Vec<String> {
    entry
        .resolution_chain
        .into_iter()
        .filter_map(|step| match step {
            ResolutionChainStep::Scope(scope) => {
                Some(scope.entry().variable_name.as_ref().to_string())
            }
            ResolutionChainStep::Attribute(attribute) => attribute
                .entry()
                .map(|entry| entry.variable_name.as_ref().to_string()),
        })
        .collect()
}

/// The variables the attributes declare, in the order they are applied.
fn documented_attributes() -> Vec<Documented> {
    HighlightAttribute::entries()
        .iter()
        .map(|entry| Documented {
            variable: entry.variable_name.as_ref().to_string(),
            path: None,
            description: entry.description,
            chain: Vec::new(),
        })
        .collect()
}

/// Every colour variable, in declaration order, the attributes last.
fn colour_variables() -> Vec<Documented> {
    let mut documented = documented_in("fish_color_");
    documented.extend(documented_attributes());
    documented
}

/// Every block the declaration generates.
fn blocks() -> Vec<Block> {
    vec![
        Block {
            name: "colour variables",
            lines: rendered_lines(colour_variables()),
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
        .with_context(|| format!("writing {DOCUMENTATION}"))?;
    std::fs::write(COLOUR_VARIABLES_SCRIPT, colour_variables_script())
        .with_context(|| format!("writing {COLOUR_VARIABLES_SCRIPT}"))
}

/// Fail unless the documentation and the page state what the declaration renders.
pub fn check() -> Result<()> {
    let updated = updated()?;
    let current = std::fs::read_to_string(DOCUMENTATION)
        .with_context(|| format!("reading {DOCUMENTATION}"))?;
    if current != updated {
        bail!("{DOCUMENTATION} does not hold the declared scopes; run `cargo xtask token-scopes`");
    }

    let rendered = colour_variables_script();
    let current = std::fs::read_to_string(COLOUR_VARIABLES_SCRIPT)
        .with_context(|| format!("reading {COLOUR_VARIABLES_SCRIPT}"))?;
    if current != rendered {
        bail!(
            "{COLOUR_VARIABLES_SCRIPT} does not hold the declared scopes; \
             run `cargo xtask token-scopes`"
        );
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

/// The page's list of colour variables: every declared variable, with the scope it
/// stands for and what it is.
///
/// The page colours the classified sample through the path each variable stands for, so
/// the list carries the paths; a variable that stands for an attribute carries none,
/// because the highlighting pass never reports an attribute as a character's scope.
fn colour_variables_script() -> String {
    let mut script = String::from(
        "// The colour variables, generated from the declared scopes by `cargo xtask\n\
         // token-scopes`; do not edit.\n\
         window.__fish_colour_variables = [\n",
    );
    for documented in colour_variables() {
        let path = match documented.path {
            Some(path) => format!("{path:?}"),
            None => "null".to_owned(),
        };
        script.push_str(&format!(
            "    {{variable: {:?}, path: {path}, description: {:?}, chain: [{}]}},\n",
            documented.variable,
            plain_text(documented.description),
            documented
                .chain
                .iter()
                .map(|variable| format!("{variable:?}"))
                .collect::<Vec<String>>()
                .join(", ")
        ));
    }
    script.push_str("];\n");

    script.push_str(
        "\n// The attributes, generated from the declared scopes by `cargo xtask\n\
         // token-scopes`; do not edit.\n\
         window.__fish_highlight_attributes = [\n",
    );
    for entry in HighlightAttribute::entries() {
        let (mode, fields) = match entry.mode {
            AttributeMode::Merge(fields) => ("merge", fields),
            AttributeMode::Replace(fields) => ("replace", fields),
        };
        let fields: Vec<&str> = STYLE_FIELDS
            .iter()
            .copied()
            .filter(|field| fields.has_field(*field))
            .map(style_field_name)
            .collect();
        script.push_str(&format!(
            "    {{variable: {:?}, mode: {:?}, fields: [{}]}},\n",
            entry.variable_name.as_ref().to_string(),
            mode,
            fields
                .iter()
                .map(|field| format!("{field:?}"))
                .collect::<Vec<String>>()
                .join(", ")
        ));
    }
    script.push_str("];\n");
    script
}

/// Every field a style has, in the order the page writes them.
const STYLE_FIELDS: &[StyleField] = &[
    StyleField::Foreground,
    StyleField::Background,
    StyleField::UnderlineColor,
    StyleField::Flags,
];

/// The name the page's style resolver knows a field by.
fn style_field_name(field: StyleField) -> &'static str {
    match field {
        StyleField::Foreground => "foreground",
        StyleField::Background => "background",
        StyleField::UnderlineColor => "underline_colour",
        StyleField::Flags => "flags",
    }
}

/// A description with its Markdown code spans marked out as plain text.
///
/// The page sets a description as text, so a ```x``` span would show its backticks.
fn plain_text(description: &str) -> String {
    description.replace("``", "")
}

#[cfg(test)]
mod tests {
    use super::{Block, blocks, colour_variables, colour_variables_script, marker, replace_block};

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
    fn the_page_lists_every_colour_variable() {
        let script = colour_variables_script();
        // Each variable's entry states the chain it resolves through; no attribute does.
        let listed = script.matches(", chain: [").count();
        assert_eq!(listed, colour_variables().len());
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
