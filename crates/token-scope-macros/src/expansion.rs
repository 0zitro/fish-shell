//! The declaration expanded into the vocabulary: the ids, the chains and the
//! table the macro writes.

use std::collections::HashMap;

use proc_macro2::{Span, TokenStream};

use crate::declaration::{Declaration, DeclarationError, FormKind, WrittenStep};

/// The declared vocabulary, as the Rust that declares it, or the failure that
/// stops the compilation at the declaration that could not be read.
pub(crate) fn expanded(input: TokenStream) -> TokenStream {
    declaration(input).unwrap_or_else(|error| error.to_compile_error())
}

/// One scope the declaration produces.
struct Scope {
    path: String,
    stem: String,
    /// The namespace prefix the scope's variable begins with.
    prefix: String,
    written: Option<Vec<WrittenStep>>,
    enclosing: bool,
    root: bool,
    /// Where the form that produced it was written, for a failure to resolve it.
    span: Span,
}

/// One step of a resolved chain.
#[derive(Clone, PartialEq, Eq)]
enum ChainStep {
    Scope(usize),
    Attribute(String),
}

fn declaration(input: TokenStream) -> Result<TokenStream, DeclarationError> {
    let declaration = Declaration::parse(input)?;
    let scopes = scopes_of(declaration)?;
    let (paths, root) = index_of(&scopes)?;
    let chains = resolution_chains(&scopes, &paths, root)?;
    let enclosing = enclosing_bits(&scopes)?;

    render(&scopes, &chains, &enclosing).parse().map_err(|_| {
        DeclarationError::at(
            Span::call_site(),
            "the declaration expanded to Rust that does not parse",
        )
    })
}

/// The scopes the declaration produces, in declaration order.
///
/// A sum's stems are its family's, so the forms are read repeatedly until every
/// sum whose family is declared finds it.
fn scopes_of(declaration: Declaration) -> Result<Vec<Scope>, DeclarationError> {
    let mut pending: Vec<(String, FormKind, Span)> = vec![];
    for namespace in declaration.namespaces {
        for form in namespace.forms {
            pending.push((namespace.prefix.clone(), form.kind, form.span));
        }
    }

    let mut scopes: Vec<Scope> = vec![];
    loop {
        let remaining = pending.len();
        let mut deferred: Vec<(String, FormKind, Span)> = vec![];

        for (prefix, kind, span) in pending {
            match kind {
                FormKind::Entry {
                    path,
                    stem,
                    chain,
                    enclosing,
                    root,
                } => {
                    let stem = stem.unwrap_or_else(|| flattened(&path));
                    scopes.push(Scope {
                        path,
                        stem,
                        prefix,
                        written: chain,
                        enclosing,
                        root,
                        span,
                    });
                }
                FormKind::Sum {
                    family,
                    stem_prefix,
                    variants,
                } => {
                    let Some(head) = scopes
                        .iter()
                        .find(|scope| scope.path == family)
                        .map(|family| stem_prefix.clone().unwrap_or_else(|| family.stem.clone()))
                    else {
                        deferred.push((
                            prefix,
                            FormKind::Sum {
                                family,
                                stem_prefix,
                                variants,
                            },
                            span,
                        ));
                        continue;
                    };

                    for variant in variants {
                        scopes.push(Scope {
                            path: format!("{family}.{variant}"),
                            stem: format!("{head}_{variant}"),
                            prefix: prefix.clone(),
                            written: None,
                            enclosing: false,
                            root: false,
                            span,
                        });
                    }
                }
                FormKind::Product {
                    path,
                    chain,
                    factors,
                } => {
                    for leaf in product_leaves(&path, &factors) {
                        // The product's chain is the family root's, and every
                        // leaf reaches it through the derivation.
                        let written = if leaf == path { chain.clone() } else { None };
                        scopes.push(Scope {
                            stem: flattened(&leaf),
                            path: leaf,
                            prefix: prefix.clone(),
                            written,
                            enclosing: false,
                            root: false,
                            span,
                        });
                    }
                }
            }
        }

        if deferred.is_empty() {
            break;
        }
        if deferred.len() == remaining {
            let family = match &deferred[0].1 {
                FormKind::Sum { family, .. } => family,
                _ => return Err(DeclarationError::at(deferred[0].2, "a form cannot be read")),
            };
            return Err(DeclarationError::at(
                deferred[0].2,
                format!("no entry declares the family `{family}`, so its sum cannot be read"),
            ));
        }
        pending = deferred;
    }

    check_unique(&scopes)?;
    Ok(scopes)
}

/// The leaves of a product: one path per tuple of factor values, the absent
/// value included.
fn product_leaves(path: &str, factors: &[Vec<String>]) -> Vec<String> {
    let mut leaves = vec![path.to_owned()];

    for factor in factors {
        let mut next = Vec::new();
        for leaf in &leaves {
            next.push(leaf.clone());
            for value in factor {
                next.push(format!("{leaf}.{value}"));
            }
        }
        leaves = next;
    }

    leaves
}

/// Every path and its scope, and the root's.
fn index_of(scopes: &[Scope]) -> Result<(HashMap<&str, usize>, usize), DeclarationError> {
    let mut paths = HashMap::new();
    let mut root = None;

    for (index, scope) in scopes.iter().enumerate() {
        if let Some(previous) = paths.insert(scope.path.as_str(), index) {
            return Err(DeclarationError::at(
                scope.span,
                format!(
                    "the path `{}` is declared twice, here and by `{}`",
                    scope.path, scopes[previous].stem
                ),
            ));
        }
        if scope.root && root.replace(index).is_some() {
            return Err(DeclarationError::at(
                scope.span,
                "one entry is the root, and another is marked root as well",
            ));
        }
    }

    let Some(root) = root else {
        return Err(DeclarationError::at(
            Span::call_site(),
            "the declaration marks no root; the entry every chain ends at is written `root`",
        ));
    };

    Ok((paths, root))
}

/// No two scopes may be spelled by one constant or one variable.
fn check_unique(scopes: &[Scope]) -> Result<(), DeclarationError> {
    let mut stems: HashMap<&str, &Scope> = HashMap::new();
    let mut variables: HashMap<String, &Scope> = HashMap::new();

    for scope in scopes {
        if let Some(previous) = stems.insert(scope.stem.as_str(), scope) {
            return Err(DeclarationError::at(
                scope.span,
                format!(
                    "`{}` and `{}` share the stem `{}`",
                    previous.path, scope.path, scope.stem
                ),
            ));
        }
        let variable = format!("{}{}", scope.prefix, scope.stem);
        if let Some(previous) = variables.insert(variable.clone(), scope) {
            return Err(DeclarationError::at(
                scope.span,
                format!(
                    "`{}` and `{}` share the variable `{variable}`",
                    previous.path, scope.path
                ),
            ));
        }
    }

    Ok(())
}

/// The steps consulted for each scope, most specific first, ending at the root
/// and excluding the scope itself.
fn resolution_chains(
    scopes: &[Scope],
    paths: &HashMap<&str, usize>,
    root: usize,
) -> Result<Vec<Vec<ChainStep>>, DeclarationError> {
    let mut memo: Vec<Option<Vec<ChainStep>>> = vec![None; scopes.len()];
    let mut chains = Vec::with_capacity(scopes.len());

    for index in 0..scopes.len() {
        let mut chain = steps_of(index, scopes, paths, &mut memo)?;
        chain.push(ChainStep::Scope(root));
        chain.retain(|step| step != &ChainStep::Scope(index));
        chains.push(chain);
    }

    Ok(chains)
}

/// The steps between one scope and the root, excluding both.
fn steps_of(
    index: usize,
    scopes: &[Scope],
    paths: &HashMap<&str, usize>,
    memo: &mut [Option<Vec<ChainStep>>],
) -> Result<Vec<ChainStep>, DeclarationError> {
    if let Some(steps) = &memo[index] {
        return Ok(steps.clone());
    }

    let scope = &scopes[index];
    let steps = match &scope.written {
        Some(written) => written
            .iter()
            .map(|step| resolve_step(step, paths))
            .collect::<Result<Vec<_>, _>>()?,
        None => match nearest_prefix(&scope.path, paths) {
            Some(prefix) => {
                let mut steps = vec![ChainStep::Scope(prefix)];
                steps.extend(steps_of(prefix, scopes, paths, memo)?);
                steps
            }
            None => vec![],
        },
    };

    memo[index] = Some(steps.clone());
    Ok(steps)
}

/// The nearest declared entry prefix of a path.
fn nearest_prefix(path: &str, paths: &HashMap<&str, usize>) -> Option<usize> {
    let mut end = path.len();
    while let Some(dot) = path[..end].rfind('.') {
        if let Some(&index) = paths.get(&path[..dot]) {
            return Some(index);
        }
        end = dot;
    }
    None
}

fn resolve_step(
    step: &WrittenStep,
    paths: &HashMap<&str, usize>,
) -> Result<ChainStep, DeclarationError> {
    Ok(match step {
        WrittenStep::Scope(path) => match paths.get(path.as_str()) {
            Some(&index) => ChainStep::Scope(index),
            None => {
                return Err(DeclarationError::at(
                    Span::call_site(),
                    format!("the chain names `{path}`, which no entry declares"),
                ));
            }
        },
        WrittenStep::Attribute(stem) => ChainStep::Attribute(stem.clone()),
    })
}

/// The scopes that enclose, in the order they are consulted: deepest path first,
/// then by path.  A member's bit is its index here.
fn enclosing_bits(scopes: &[Scope]) -> Result<Vec<usize>, DeclarationError> {
    let mut enclosing: Vec<usize> = scopes
        .iter()
        .enumerate()
        .filter(|(_, scope)| scope.enclosing)
        .map(|(index, _)| index)
        .collect();
    enclosing.sort_by(|left, right| {
        let depth = |scope: &Scope| scope.path.matches('.').count();
        depth(&scopes[*right])
            .cmp(&depth(&scopes[*left]))
            .then_with(|| scopes[*left].path.cmp(&scopes[*right].path))
    });

    if enclosing.len() > 16 {
        return Err(DeclarationError::at(
            Span::call_site(),
            "more scopes enclose than an `EnclosingScopeSet` has bits",
        ));
    }

    Ok(enclosing)
}

/// The expansion, as the Rust that declares the vocabulary.
fn render(scopes: &[Scope], chains: &[Vec<ChainStep>], enclosing: &[usize]) -> String {
    let mut source = Source::default();

    source.line("impl crate::entries::TokenScopeId {");
    for (index, scope) in scopes.iter().enumerate() {
        source.line(format!("    /// The `{}` scope.", scope.path));
        source.line(format!(
            "    pub const {}: Self = Self({index});",
            constant(scope)
        ));
    }
    source.line("}");
    source.blank();

    source
        .line("/// The scopes that enclose, in the order they are consulted: a scope's bit in an");
    source.line("/// `EnclosingScopeSet` is its index here.");
    source.line("pub(crate) static ENCLOSING_SCOPE_IDS: &[crate::entries::TokenScopeId] = &[");
    for index in enclosing {
        source.line(format!(
            "    crate::entries::TokenScopeId::{},",
            constant(&scopes[*index])
        ));
    }
    source.line("];");
    source.blank();

    source.line("impl crate::entries::TokenScopeId {");
    source.line("    /// The bit this scope occupies in an `EnclosingScopeSet`, or none when");
    source.line("    /// the scope encloses nothing.");
    source.line("    pub(crate) fn enclosing_bit(self) -> Option<u16> {");
    source.line("        match self {");
    for (bit, index) in enclosing.iter().enumerate() {
        source.line(format!(
            "            Self::{} => Some({bit}),",
            constant(&scopes[*index])
        ));
    }
    source.line("            _ => None,");
    source.line("        }");
    source.line("    }");
    source.line("}");
    source.blank();

    source.line("/// Every declared scope, in declaration order.");
    source.line("pub static TOKEN_SCOPE_ENTRIES: &[crate::entries::TokenScopeEntry] = &[");
    for (index, scope) in scopes.iter().enumerate() {
        let steps: Vec<String> = chains[index]
            .iter()
            .map(|step| match step {
                ChainStep::Scope(scope) => format!(
                    "crate::chain::ResolutionChainStep::Scope(crate::entries::TokenScopeId::{})",
                    constant(&scopes[*scope])
                ),
                ChainStep::Attribute(attribute) => format!(
                    "crate::chain::ResolutionChainStep::Attribute\
                     (crate::attribute::HighlightAttribute::{attribute})"
                ),
            })
            .collect();

        source.line("    crate::entries::TokenScopeEntry {");
        source.line(format!(
            "        scope: crate::entries::TokenScopeId::{},",
            constant(scope)
        ));
        source.line(format!("        path: {:?},", scope.path));
        source.line(format!(
            "        variable_name: crate::entries::ColorVariableName\
             (fish_widestring::L!({:?})),",
            format!("{}{}", scope.prefix, scope.stem)
        ));
        source.line(format!(
            "        resolution_chain: crate::entries::ScopeResolutionChain(&[{}]),",
            steps.join(", ")
        ));
        source.line("    },");
    }
    source.line("];");

    source.0
}

/// The constant a scope's stem is named by.
fn constant(scope: &Scope) -> String {
    scope.stem.to_uppercase()
}

/// A path with `.` as `_`: the stem the declaration derives.
fn flattened(path: &str) -> String {
    path.replace('.', "_")
}

/// The Rust a declaration expands to, written a line at a time.
#[derive(Default)]
struct Source(String);

impl Source {
    fn line(&mut self, line: impl AsRef<str>) {
        self.0.push_str(line.as_ref());
        self.0.push('\n');
    }

    fn blank(&mut self) {
        self.0.push('\n');
    }
}
