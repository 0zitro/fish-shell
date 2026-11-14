//! Syntax highlighting.
//!
//! The module holds the visitor that classifies every character of a source buffer
//! as a token scope, and the resolution of that classification into the style the
//! terminal is given.  Classification depends on the buffer, the cursor and the
//! environment; resolution depends on the color variables alone.

mod classification;
mod file_tester;
mod function_parameters;
mod style_resolution;
mod visitor;

pub use classification::HighlightClassification;
pub use style_resolution::HighlightStyleResolver;
pub use visitor::{
    autosuggest_validate_from_history, colorize_text, highlight_and_colorize, highlight_shell,
    is_veritable_cd,
};
