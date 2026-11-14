//! The declaration: its grammar, and the forms it is written from.
//!
//! A declaration is a flat sequence of namespaces.  A namespace is a variable
//! name prefix and the forms declared under it, and each form states either one
//! scope, the variants of a declared family, or the leaves of a product of
//! factors.  Nothing here knows about ids or chains; [`crate::expansion`] turns
//! the forms into them.

use proc_macro2::{Delimiter, Span, TokenStream, TokenTree};

/// A failure to read a declaration, at the tokens it concerns.
pub struct DeclarationError {
    message: String,
    span: Span,
}

impl DeclarationError {
    pub(crate) fn at(span: Span, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }

    /// The failure, as the tokens that stop the compilation at its cause.
    pub fn to_compile_error(&self) -> TokenStream {
        let message = format!("{:?}", self.message);
        // The span the declaration was read at, carried by the invocation.
        let _ = self.span;
        format!("::core::compile_error!({message});")
            .parse()
            .expect("a compile_error invocation is a token stream")
    }
}

/// A namespace: one variable name prefix and what is declared under it.
pub struct Namespace {
    /// The prefix every variable declared here begins with, such as
    /// `fish_color_`.
    pub prefix: String,
    pub forms: Vec<Form>,
}

/// One form of the declaration, and where it was written.
pub struct Form {
    pub span: Span,
    pub kind: FormKind,
}

/// What a form declares.
pub enum FormKind {
    /// One scope, written as its path.
    Entry {
        path: String,
        /// What the scope is, in the words the documentation and the constant's
        /// own documentation state it in.
        description: String,
        /// The stem the declaration writes, when the variable name the
        /// configuration fixed is not the path flattened.
        stem: Option<String>,
        chain: Option<Vec<WrittenStep>>,
        /// Whether the scope encloses characters rather than classifies them.
        enclosing: bool,
        /// Whether every chain ends at this scope.
        root: bool,
    },
    /// The variants of one declared family.
    Sum {
        family: String,
        /// The stem the variants begin with, when it is not the family's own.
        stem_prefix: Option<String>,
        variants: Vec<Declared>,
    },
    /// The leaves of a product of factors, each an enumeration of one name
    /// component.
    Product {
        path: String,
        chain: Option<Vec<WrittenStep>>,
        /// What the family is; a leaf's description also states the values it
        /// takes.
        description: String,
        factors: Vec<Vec<Declared>>,
    },
}

/// One value an enumeration declares, and what it is.
pub struct Declared {
    pub name: String,
    pub description: String,
}

/// One step of a written resolution chain.
#[derive(Clone)]
pub enum WrittenStep {
    /// The step names a scope, by path.
    Scope(String),
    /// The step names an attribute, by its variant.
    Attribute(String),
}

/// A declaration: its namespaces, in the order they were written.
pub struct Declaration {
    pub namespaces: Vec<Namespace>,
}

impl Declaration {
    pub fn parse(stream: TokenStream) -> Result<Self, DeclarationError> {
        let mut tokens = Tokens::new(stream);
        let mut namespaces = vec![];

        while let Some(tree) = tokens.peek() {
            match tree {
                TokenTree::Ident(ident) if ident == "namespace" => {
                    tokens.take();
                    let prefix = tokens.string("the variable-name prefix of a namespace")?;
                    let body = tokens.group(Delimiter::Brace, "the declarations of a namespace")?;
                    namespaces.push(Namespace {
                        prefix,
                        forms: parse_forms(body)?,
                    });
                }
                other => {
                    return Err(DeclarationError::at(
                        other.span(),
                        "a declaration is a sequence of `namespace \"<prefix>\" { ... }`",
                    ));
                }
            }
        }

        Ok(Self { namespaces })
    }
}

/// The forms of one namespace.
fn parse_forms(stream: TokenStream) -> Result<Vec<Form>, DeclarationError> {
    let mut tokens = Tokens::new(stream);
    let mut forms = vec![];

    while let Some(tree) = tokens.peek() {
        let form = match tree {
            TokenTree::Ident(ident) => match ident.to_string().as_str() {
                "sum" => parse_sum(&mut tokens)?,
                "product" => parse_product(&mut tokens)?,
                other => {
                    return Err(DeclarationError::at(
                        ident.span(),
                        format!(
                            "`{other}` is not a declaration form; a scope is written as a path"
                        ),
                    ));
                }
            },
            TokenTree::Literal(_) => parse_entry(&mut tokens)?,
            other => {
                return Err(DeclarationError::at(
                    other.span(),
                    "a declaration form is a scope path, `sum` or `product`",
                ));
            }
        };
        forms.push(form);
    }

    Ok(forms)
}

/// One scope, stated by its path.
fn parse_entry(tokens: &mut Tokens) -> Result<Form, DeclarationError> {
    let span = tokens.here();
    let path = tokens.string("a scope path")?;
    let description = tokens.description("what the scope is")?;
    let stem = tokens.optional_stem();
    let chain = tokens.optional_steps()?;
    let root = tokens.optional_ident("root");
    let enclosing = tokens.optional_ident("enclosing");
    tokens.expect_punct(';', "`;` after a scope")?;

    Ok(Form {
        span,
        kind: FormKind::Entry {
            path,
            description,
            stem,
            chain,
            enclosing,
            root,
        },
    })
}

/// The variants of one declared family.
fn parse_sum(tokens: &mut Tokens) -> Result<Form, DeclarationError> {
    let span = tokens.here();
    tokens.expect_ident("sum")?;
    let family = tokens.string("the path of the family a sum declares")?;
    let stem_prefix = tokens.optional_stem();
    let variants = tokens.group(Delimiter::Brace, "the variants of a sum")?;
    let variants = declared_values(variants, "a sum enumerates values")?;
    tokens.expect_punct(';', "`;` after a sum")?;

    Ok(Form {
        span,
        kind: FormKind::Sum {
            family,
            stem_prefix,
            variants,
        },
    })
}

/// The leaves of a product of factors.
fn parse_product(tokens: &mut Tokens) -> Result<Form, DeclarationError> {
    let span = tokens.here();
    tokens.expect_ident("product")?;
    let path = tokens.string("the path of the family a product declares")?;
    let chain = tokens.optional_steps()?;
    let description = tokens.description("what the family is")?;
    let body = tokens.group(Delimiter::Brace, "the factors of a product")?;

    let mut factors = Tokens::new(body);
    let mut enumerations = vec![];
    while !factors.at_end() {
        factors.expect_ident("sum")?;
        let values = factors.group(Delimiter::Brace, "the values of one factor")?;
        enumerations.push(declared_values(
            values,
            "a factor enumerates the values of one name component",
        )?);
    }
    tokens.expect_punct(';', "`;` after a product")?;

    Ok(Form {
        span,
        kind: FormKind::Product {
            path,
            chain,
            description,
            factors: enumerations,
        },
    })
}

/// The values one enumeration declares, each with what it is.
fn declared_values(
    stream: TokenStream,
    complaint: &str,
) -> Result<Vec<Declared>, DeclarationError> {
    let mut tokens = Tokens::new(stream);
    let mut values = vec![];
    while !tokens.at_end() {
        let name = tokens.ident(complaint)?;
        let description = tokens.description(complaint)?;
        values.push(Declared { name, description });
    }
    Ok(values)
}

/// The steps of one written chain.
fn chain_steps(stream: TokenStream) -> Result<Vec<WrittenStep>, DeclarationError> {
    let mut tokens = Tokens::new(stream);
    let mut steps = vec![];

    while !tokens.at_end() {
        let step = if tokens.optional_punct('@') {
            WrittenStep::Attribute(tokens.ident("the stem of an attribute")?)
        } else {
            WrittenStep::Scope(tokens.string("the path of a chain step")?)
        };
        steps.push(step);
        if !tokens.at_end() {
            tokens.expect_punct(',', "`,` between the steps of a chain")?;
        }
    }

    Ok(steps)
}

/// The text a doc comment's escapes spell, or none when it names an escape this
/// declaration does not read.
fn decode(written: &str) -> Option<String> {
    let mut decoded = String::with_capacity(written.len());
    let mut characters = written.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            decoded.push(character);
            continue;
        }
        match characters.next()? {
            'n' => decoded.push('\n'),
            't' => decoded.push('\t'),
            '"' => decoded.push('"'),
            '\'' => decoded.push('\''),
            '\\' => decoded.push('\\'),
            _ => return None,
        }
    }
    Some(decoded)
}

/// The tokens of one declaration, read in order.
struct Tokens {
    trees: Vec<TokenTree>,
    index: usize,
}

impl Tokens {
    fn new(stream: TokenStream) -> Self {
        Self {
            trees: stream.into_iter().collect(),
            index: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.index >= self.trees.len()
    }

    fn peek(&self) -> Option<&TokenTree> {
        self.trees.get(self.index)
    }

    fn take(&mut self) -> Option<TokenTree> {
        let tree = self.trees.get(self.index).cloned();
        if tree.is_some() {
            self.index += 1;
        }
        tree
    }

    /// The span a failure to read this position belongs to.
    fn here(&self) -> Span {
        self.peek().map_or_else(Span::call_site, TokenTree::span)
    }

    fn span_of(&self, tree: Option<TokenTree>) -> Span {
        tree.map_or_else(|| self.here(), |tree| tree.span())
    }

    fn string(&mut self, what: &str) -> Result<String, DeclarationError> {
        let span = self.here();
        let written = self.raw_string(what)?;
        if written.contains('\\') {
            return Err(DeclarationError::at(
                span,
                format!("{what} is a plain string literal, and it names no escape"),
            ));
        }
        Ok(written)
    }

    /// The body of a string literal, before its escapes are read.
    fn raw_string(&mut self, what: &str) -> Result<String, DeclarationError> {
        let tree = self.take();
        let span = self.span_of(tree.clone());
        match tree {
            Some(TokenTree::Literal(literal)) => {
                let written = literal.to_string();
                match written
                    .strip_prefix('"')
                    .and_then(|body| body.strip_suffix('"'))
                {
                    Some(body) => Ok(body.to_owned()),
                    None => Err(DeclarationError::at(
                        span,
                        format!("{what} is a string literal"),
                    )),
                }
            }
            _ => Err(DeclarationError::at(
                span,
                format!("{what} is a string literal"),
            )),
        }
    }

    /// The description a form writes, which is the doc comment after its name.
    ///
    /// A doc comment reaches a macro as the `#[doc = "..."]` attribute it stands for,
    /// so this reads that attribute.  The words are Markdown, so the same description
    /// reads as code spans in the generated documentation and in the constant's own
    /// documentation.
    fn description(&mut self, what: &str) -> Result<String, DeclarationError> {
        let complaint = || format!("{what} is a doc comment: `/** ... */`");
        let span = self.here();

        if !self.optional_punct('#') {
            return Err(DeclarationError::at(span, complaint()));
        }
        let attribute = self.group(Delimiter::Bracket, &complaint())?;
        let mut tokens = Tokens::new(attribute);
        tokens.expect_ident("doc")?;
        tokens.expect_punct('=', &complaint())?;
        let written = tokens.raw_string(what)?;
        let text = decode(&written).ok_or_else(|| {
            DeclarationError::at(
                span,
                format!("{what} names an escape other than `\\n`, `\\t`, `\\\"` and `\\\\`"),
            )
        })?;

        let description = text.trim();
        if description.is_empty() || description.contains('\n') {
            return Err(DeclarationError::at(
                span,
                format!("{what} is one line of prose"),
            ));
        }
        Ok(description.to_owned())
    }

    /// The stem the declaration writes, if it writes one.  A stem is an
    /// identifier, and the markers that may follow it are not.
    fn optional_stem(&mut self) -> Option<String> {
        let is_stem = match self.peek() {
            Some(TokenTree::Ident(ident)) => ident != "root" && ident != "enclosing",
            _ => false,
        };
        if !is_stem {
            return None;
        }
        match self.take() {
            Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
            _ => None,
        }
    }

    fn optional_steps(&mut self) -> Result<Option<Vec<WrittenStep>>, DeclarationError> {
        let Some(TokenTree::Group(group)) = self.peek() else {
            return Ok(None);
        };
        if group.delimiter() != Delimiter::Bracket {
            return Ok(None);
        }
        let group = group.clone();
        self.take();
        Ok(Some(chain_steps(group.stream())?))
    }

    fn optional_ident(&mut self, name: &str) -> bool {
        match self.peek() {
            Some(TokenTree::Ident(ident)) if ident == name => {
                self.take();
                true
            }
            _ => false,
        }
    }

    fn optional_punct(&mut self, punctuation: char) -> bool {
        match self.peek() {
            Some(TokenTree::Punct(punct)) if punct.as_char() == punctuation => {
                self.take();
                true
            }
            _ => false,
        }
    }

    fn ident(&mut self, what: &str) -> Result<String, DeclarationError> {
        let tree = self.take();
        let span = self.span_of(tree.clone());
        match tree {
            Some(TokenTree::Ident(ident)) => Ok(ident.to_string()),
            _ => Err(DeclarationError::at(
                span,
                format!("{what} is an identifier"),
            )),
        }
    }

    fn expect_ident(&mut self, name: &str) -> Result<(), DeclarationError> {
        let tree = self.take();
        let span = self.span_of(tree.clone());
        match tree {
            Some(TokenTree::Ident(ident)) if ident == name => Ok(()),
            _ => Err(DeclarationError::at(span, format!("`{name}` stands here"))),
        }
    }

    fn expect_punct(&mut self, punctuation: char, what: &str) -> Result<(), DeclarationError> {
        let tree = self.take();
        let span = self.span_of(tree.clone());
        match tree {
            Some(TokenTree::Punct(punct)) if punct.as_char() == punctuation => Ok(()),
            _ => Err(DeclarationError::at(span, what)),
        }
    }

    fn group(&mut self, delimiter: Delimiter, what: &str) -> Result<TokenStream, DeclarationError> {
        let tree = self.take();
        let span = self.span_of(tree.clone());
        match tree {
            Some(TokenTree::Group(group)) if group.delimiter() == delimiter => Ok(group.stream()),
            _ => Err(DeclarationError::at(span, what)),
        }
    }
}
