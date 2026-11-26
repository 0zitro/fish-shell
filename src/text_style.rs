//! The style one span of text is drawn in: its three colors and its flags.
//!
//! The style is the value domain the terminal is written with, and the value the
//! colour variables parse into. A variable names the fields it sets and leaves the
//! rest absent, so the type a variable parses into is a style whose every field may
//! be missing: [`SpecifiedTextStyle`]. A style every field of which is present is
//! what the terminal is given: [`TextStyle`].

use crate::prelude::*;
use crate::terminal::{self, get_color_support};
use fish_color::Color;
use fish_token_scope::{StyleField, StyleFieldSet};
use fish_wgetopt::{ArgType, WGetopter, WOption, wopt};

pub(crate) trait StyleSet {
    fn union_prefer_right(self, other: Self) -> Self;
    fn difference_prefer_empty(self, other: Self) -> Self;
}

#[derive(Default, Debug, Copy, Clone, PartialEq)]
pub(crate) enum ResettableStyle<T = ()>
where
    T: Copy + std::fmt::Debug,
{
    #[default]
    Unchanged,
    Off,
    On(T),
}

impl<T> StyleSet for ResettableStyle<T>
where
    T: Copy + std::fmt::Debug,
{
    fn union_prefer_right(self, other: Self) -> Self {
        if matches!(other, Self::Unchanged) {
            self
        } else {
            other
        }
    }

    fn difference_prefer_empty(self, other: Self) -> Self {
        if !matches!(other, Self::Unchanged) {
            Self::Unchanged
        } else {
            self
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub(crate) enum UnderlineStyle {
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// The resettable flags of a style.
///
/// A flag is worth an attribute rather than a modifier: bold and dim are set or not,
/// while the rest are a three-valued decision, because a style may state that an
/// underline is off, and that is not the absence of a decision.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct TextStyleFlags {
    pub(crate) bold: bool,
    pub(crate) underline_style: ResettableStyle<UnderlineStyle>,
    pub(crate) italics: ResettableStyle,
    pub(crate) dim: bool,
    pub(crate) reverse: ResettableStyle,
    pub(crate) strikethrough: ResettableStyle,
}

impl TextStyleFlags {
    pub(crate) const fn terminal_default() -> Self {
        Self {
            bold: false,
            underline_style: ResettableStyle::Off,
            italics: ResettableStyle::Off,
            dim: false,
            reverse: ResettableStyle::Off,
            strikethrough: ResettableStyle::Off,
        }
    }

    pub(crate) const fn unknown() -> Self {
        Self {
            bold: false,
            underline_style: ResettableStyle::Unchanged,
            italics: ResettableStyle::Unchanged,
            dim: false,
            reverse: ResettableStyle::Unchanged,
            strikethrough: ResettableStyle::Unchanged,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::unknown()
    }

    #[cfg(test)]
    pub(crate) fn all_set(&self) -> bool {
        (self.underline_style != ResettableStyle::Unchanged)
            && (self.italics != ResettableStyle::Unchanged)
            && (self.reverse != ResettableStyle::Unchanged)
            && (self.strikethrough != ResettableStyle::Unchanged)
    }

    pub(crate) fn union_prefer_right(self, other: Self) -> Self {
        Self {
            bold: self.is_bold() || other.is_bold(),
            underline_style: self
                .underline_style
                .union_prefer_right(other.underline_style),
            italics: self.italics.union_prefer_right(other.italics),
            dim: self.is_dim() || other.is_dim(),
            reverse: self.reverse.union_prefer_right(other.reverse),
            strikethrough: self.strikethrough.union_prefer_right(other.strikethrough),
        }
    }

    pub(crate) fn difference_prefer_empty(self, other: Self) -> Self {
        Self {
            bold: self.is_bold() && !other.is_bold(),
            underline_style: self
                .underline_style
                .difference_prefer_empty(other.underline_style),
            italics: self.italics.difference_prefer_empty(other.italics),
            dim: self.is_dim() && !other.is_dim(),
            reverse: self.reverse.difference_prefer_empty(other.reverse),
            strikethrough: self
                .strikethrough
                .difference_prefer_empty(other.strikethrough),
        }
    }

    /// Returns whether the text style is bold.
    pub(crate) const fn is_bold(self) -> bool {
        self.bold
    }

    #[cfg(test)]
    pub(crate) const fn underline_style(self) -> ResettableStyle<UnderlineStyle> {
        self.underline_style
    }

    /// Set the given underline style.
    pub(crate) fn inject_underline(&mut self, underline: ResettableStyle<UnderlineStyle>) {
        self.underline_style = underline;
    }

    /// Returns whether the text style is dim.
    pub(crate) const fn is_dim(self) -> bool {
        self.dim
    }
}

/// The style one span of text is drawn in: its three colors and its flags.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) underline_color: Color,
    pub(crate) flags: TextStyleFlags,
}

impl TextStyle {
    pub(crate) const fn terminal_default() -> Self {
        Self {
            fg: Color::Normal,
            bg: Color::Normal,
            underline_color: Color::Normal,
            flags: TextStyleFlags::terminal_default(),
        }
    }

    pub(crate) const fn unknown() -> Self {
        Self {
            fg: Color::None,
            bg: Color::None,
            underline_color: Color::None,
            flags: TextStyleFlags::unknown(),
        }
    }

    pub(crate) fn new(fg: Color, bg: Color, underline_color: Color, flags: TextStyleFlags) -> Self {
        Self {
            fg,
            bg,
            underline_color,
            flags,
        }
    }

    /// This style with `other`'s fields written over it.
    ///
    /// A field [`Color::None`] names is absent, so it does not speak for that field:
    /// this is how a composition keeps the value a less specific variable gave it,
    /// and how a channel contributes only the fields it composes.
    pub(crate) fn union_prefer_right(self, other: Self) -> Self {
        Self {
            fg: prefer_right(self.fg, other.fg),
            bg: prefer_right(self.bg, other.bg),
            underline_color: prefer_right(self.underline_color, other.underline_color),
            flags: self.flags.union_prefer_right(other.flags),
        }
    }

    /// This style with every field it leaves absent taken from the terminal default.
    pub(crate) fn with_terminal_defaults(self) -> Self {
        let default = Self::terminal_default();
        let or_default = |value: Color, fallback: Color| {
            if value.is_none() { fallback } else { value }
        };
        Self {
            fg: or_default(self.fg, default.fg),
            bg: or_default(self.bg, default.bg),
            underline_color: or_default(self.underline_color, default.underline_color),
            // A flag group is one field: a flag the style names stands, and one it leaves
            // unchanged takes the default.
            flags: default.flags.union_prefer_right(self.flags),
        }
    }

    /// This style with the fields in `fields` cleared.
    pub(crate) fn without(self, fields: StyleFieldSet) -> Self {
        Self {
            fg: cleared(self.fg, fields.has_field(StyleField::Foreground)),
            bg: cleared(self.bg, fields.has_field(StyleField::Background)),
            underline_color: cleared(
                self.underline_color,
                fields.has_field(StyleField::UnderlineColor),
            ),
            flags: if fields.has_field(StyleField::Flags) {
                TextStyleFlags::unknown()
            } else {
                self.flags
            },
        }
    }

    /// This style with every field outside `fields` cleared.
    pub(crate) fn restrict(self, fields: StyleFieldSet) -> Self {
        Self {
            fg: cleared(self.fg, !fields.has_field(StyleField::Foreground)),
            bg: cleared(self.bg, !fields.has_field(StyleField::Background)),
            underline_color: cleared(
                self.underline_color,
                !fields.has_field(StyleField::UnderlineColor),
            ),
            flags: if fields.has_field(StyleField::Flags) {
                self.flags
            } else {
                TextStyleFlags::unknown()
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn all_set(&self) -> bool {
        !self.fg.is_none()
            && !self.bg.is_none()
            && !self.underline_color.is_none()
            && self.flags.all_set()
    }
}

/// `right` where it names a field, `left` where it does not.
fn prefer_right(left: Color, right: Color) -> Color {
    if right.is_none() { left } else { right }
}

/// The color, or `Color::None` where the field is cleared.
fn cleared(color: Color, cleared: bool) -> Color {
    if cleared { Color::None } else { color }
}

/// The style a color variable denotes: the fields it names, and nothing else.
///
/// A field left absent is a field the variable does not speak about, which is what
/// lets a composition keep the value a less specific variable gave that field.
#[derive(Debug, PartialEq)]
pub(crate) struct SpecifiedTextStyle {
    pub(crate) fg: Option<Color>,
    pub(crate) bg: Option<Color>,
    pub(crate) underline_color: Option<Color>,
    pub(crate) flags: TextStyleFlags,
    pub(crate) reset: bool,
}

impl Default for SpecifiedTextStyle {
    fn default() -> Self {
        SpecifiedTextStyle {
            fg: Default::default(),
            bg: Default::default(),
            underline_color: Default::default(),
            flags: TextStyleFlags::unknown(),
            reset: false,
        }
    }
}

impl SpecifiedTextStyle {
    /// The fields the value names, absent where it names none.
    pub(crate) fn named_fields(&self) -> TextStyle {
        TextStyle {
            fg: self.fg.unwrap_or(Color::None),
            bg: self.bg.unwrap_or(Color::None),
            underline_color: self.underline_color.unwrap_or(Color::None),
            flags: self.flags,
        }
    }

    /// The style this value denotes, or none when it specifies nothing.
    ///
    /// Every field it leaves unspecified takes the terminal default.
    pub(crate) fn find_style(&self) -> Option<TextStyle> {
        (*self != Self::default()).then(|| {
            let default = TextStyle::terminal_default();
            TextStyle {
                fg: self.fg.unwrap_or(default.fg),
                bg: self.bg.unwrap_or(default.bg),
                underline_color: self.underline_color.unwrap_or(default.underline_color),
                flags: default.flags.union_prefer_right(self.flags),
            }
        })
    }
}

pub(crate) fn parse_text_style(arguments: &[WString]) -> SpecifiedTextStyle {
    let mut argv: Vec<&wstr> = Some(L!(""))
        .into_iter()
        .chain(arguments.iter().map(|s| s.as_utfstr()))
        .collect();
    use ParsedArgs::*;
    match parse_text_style_and_options(&mut argv, /*is_builtin=*/ false) {
        Ok(SetStyle(specified_style)) => specified_style,
        Err(_) => Default::default(),
        Ok(PrintColors(_)) | Ok(PrintHelp) => unreachable!(),
    }
}

pub(crate) struct PrintColorsArgs<'argarray, 'args> {
    pub(crate) fg_args: &'argarray [&'args wstr],
    pub(crate) bg: Option<Color>,
    pub(crate) underline_color: Option<Color>,
    pub(crate) flags: TextStyleFlags,
}

pub(crate) enum ParsedArgs<'argarray, 'args> {
    SetStyle(SpecifiedTextStyle),
    PrintHelp,
    PrintColors(PrintColorsArgs<'argarray, 'args>),
}

pub(crate) enum ParseError<'args> {
    MissingOptArg,
    UnexpectedOptArg(usize),
    InvalidOptArg(&'static wstr, &'args wstr),
    UnknownColor(&'args wstr),
    UnknownUnderlineStyle(&'args wstr),
    UnknownOption(usize),
    InvalidFgArgCombination,
    InvalidFgPrintColorCombination,
}

fn parse_resettable_style<'a>(w: &WGetopter<'_, 'a, '_>) -> Result<ResettableStyle, &'a wstr> {
    let Some(arg) = w.woptarg else {
        return Ok(ResettableStyle::On(()));
    };
    if arg == "off" {
        Ok(ResettableStyle::Off)
    } else if arg == "on" {
        Ok(ResettableStyle::On(()))
    } else {
        Err(arg)
    }
}

pub(crate) fn parse_text_style_and_options<'argarray, 'args>(
    argv: &'argarray mut [&'args wstr],
    is_builtin: bool,
) -> Result<ParsedArgs<'argarray, 'args>, ParseError<'args>> {
    let builtin_extra_args = if is_builtin { 0 } else { "hc".len() };
    let short_options = L!("f:b:oi::dr::s::u::ch");
    let short_options = &short_options[..short_options.len() - builtin_extra_args];
    let long_options: &[WOption] = &[
        wopt(L!("foreground"), ArgType::RequiredArgument, 'f'),
        wopt(L!("background"), ArgType::RequiredArgument, 'b'),
        wopt(L!("underline-color"), ArgType::RequiredArgument, '\x02'),
        wopt(L!("bold"), ArgType::NoArgument, 'o'),
        wopt(L!("underline"), ArgType::OptionalArgument, 'u'),
        wopt(L!("italics"), ArgType::OptionalArgument, 'i'),
        wopt(L!("dim"), ArgType::NoArgument, 'd'),
        wopt(L!("reverse"), ArgType::OptionalArgument, 'r'),
        wopt(L!("strikethrough"), ArgType::OptionalArgument, 's'),
        wopt(L!("theme"), ArgType::RequiredArgument, '\x01'),
        wopt(L!("reset"), ArgType::NoArgument, '\x03'),
        wopt(L!("help"), ArgType::NoArgument, 'h'),
        wopt(L!("print-colors"), ArgType::NoArgument, 'c'),
    ];
    let long_options = &long_options[..long_options.len() - builtin_extra_args];

    use ParseError::*;
    use ParsedArgs::*;

    let parse_color = |color_str| match Color::from_wstr(color_str) {
        color @ Some(_) => Ok(color),
        None => {
            if is_builtin {
                Err(UnknownColor(color_str))
            } else {
                Ok(None)
            }
        }
    };

    let mut fg_colors = vec![];
    let mut bg_colors = vec![];
    let mut underline_colors = vec![];
    let mut flags = TextStyleFlags::unknown();
    let mut print_color_mode = false;
    let mut reset = false;

    let mut w = WGetopter::new(short_options, long_options, argv);
    while let Some(c) = w.next_opt() {
        match c {
            'f' => {
                if let Some(fg) = parse_color(w.woptarg.unwrap())? {
                    fg_colors.push(fg);
                }
            }
            'b' => {
                if let Some(bg) = parse_color(w.woptarg.unwrap())? {
                    bg_colors.push(bg);
                }
            }
            '\x01' => (),
            '\x02' => {
                if let Some(underline_color) = parse_color(w.woptarg.unwrap())? {
                    underline_colors.push(underline_color);
                }
            }
            '\x03' => {
                reset = true;
            }
            'h' => {
                assert!(is_builtin);
                return Ok(PrintHelp);
            }
            'o' => flags.bold = true,
            'i' => {
                flags.italics =
                    parse_resettable_style(&w).map_err(|v| InvalidOptArg(L!("--italics"), v))?;
            }
            'd' => flags.dim = true,
            'r' => {
                flags.reverse =
                    parse_resettable_style(&w).map_err(|v| InvalidOptArg(L!("--reverse"), v))?;
            }
            's' => {
                flags.strikethrough = parse_resettable_style(&w)
                    .map_err(|v| InvalidOptArg(L!("--strikethrough"), v))?;
            }
            'u' => {
                let arg = w.woptarg.unwrap_or(L!("single"));
                flags.underline_style = if arg == "single" {
                    ResettableStyle::On(UnderlineStyle::Single)
                } else if arg == "double" {
                    ResettableStyle::On(UnderlineStyle::Double)
                } else if arg == "curly" {
                    ResettableStyle::On(UnderlineStyle::Curly)
                } else if arg == "dotted" {
                    ResettableStyle::On(UnderlineStyle::Dotted)
                } else if arg == "dashed" {
                    ResettableStyle::On(UnderlineStyle::Dashed)
                } else if arg == "off" {
                    ResettableStyle::Off
                } else {
                    return Err(UnknownUnderlineStyle(arg));
                };
            }
            'c' => {
                assert!(is_builtin);
                print_color_mode = true;
            }
            ':' => {
                return Err(MissingOptArg);
            }
            ';' => {
                return Err(UnexpectedOptArg(w.wopt_index - 1));
            }
            '?' => {
                return Err(UnknownOption(w.wopt_index - 1));
            }
            _ => unreachable!("unexpected retval from WGetopter"),
        }
    }

    let fg_args = &w.argv[w.wopt_index..];
    if !fg_args.is_empty() && !fg_colors.is_empty() {
        return Err(InvalidFgArgCombination);
    }

    let best_color =
        |colors: Vec<Color>| terminal::best_color(colors.into_iter(), get_color_support());

    let bg = best_color(bg_colors);
    let underline_color = best_color(underline_colors);

    if print_color_mode {
        if !fg_colors.is_empty() {
            return Err(InvalidFgPrintColorCombination);
        }
        return Ok(PrintColors(PrintColorsArgs {
            fg_args,
            bg,
            underline_color,
            flags,
        }));
    }

    // Historical behavior: reset only applies if it's the first argument.
    if is_builtin && fg_args.first().is_some_and(|fg| fg == "reset") {
        return Ok(SetStyle(SpecifiedTextStyle {
            reset: true,
            ..Default::default()
        }));
    }

    fg_colors.reserve(fg_args.len());
    for fg in fg_args {
        if is_builtin && fg == "reset" {
            continue;
        }
        if let Some(fg) = parse_color(fg)? {
            if fg == Color::Normal {
                reset = true;
            }
            fg_colors.push(fg);
        }
    }
    // #1323: We may have multiple foreground colors. Choose the best one.
    let fg = best_color(fg_colors);
    Ok(SetStyle(SpecifiedTextStyle {
        fg,
        bg,
        underline_color,
        flags,
        reset,
    }))
}

#[cfg(test)]
mod tests {
    use super::{SpecifiedTextStyle, TextStyleFlags, parse_text_style};
    use crate::env::{EnvVar, EnvVarFlags};
    use crate::prelude::*;
    use fish_color::{Color, Color24};

    #[test]
    fn test_parse_text_style() {
        assert_eq!(
            parse_text_style(&["0000ee".into(), "--theme=default".into()]),
            SpecifiedTextStyle {
                fg: Some(Color::Rgb(Color24 {
                    r: 0,
                    g: 0,
                    b: 0xee
                })),
                bg: None,
                underline_color: None,
                flags: TextStyleFlags::unknown(),
                reset: false
            }
        );
    }

    /// A value every field of which is named denotes a style none of whose fields
    /// is absent, so a composition over it needs no terminal default.
    #[test]
    fn test_find_style_fully_specified() {
        let assert_all_set = |values: Vec<WString>| {
            let var = EnvVar::new_vec(values.clone(), EnvVarFlags::default());
            let style = parse_text_style(var.as_list()).find_style();
            assert!(
                style.is_some_and(|style| style.all_set()),
                "Underspecified result for {:?}\n => {:?}",
                values,
                style
            );
        };

        assert_all_set(vec![L!("--reset").into()]);
        assert_all_set(vec![L!("normal").into()]);
        assert_all_set(vec![L!("green").into()]);
        assert_all_set(vec![L!("--foreground=normal").into()]);
        assert_all_set(vec![L!("--foreground=green").into()]);
        assert_all_set(vec![L!("--background=normal").into()]);
        assert_all_set(vec![L!("--background=green").into()]);
        assert_all_set(vec![L!("--underline-color=normal").into()]);
        assert_all_set(vec![L!("--underline-color=green").into()]);
        assert_all_set(vec![L!("--italics").into()]);
        assert_all_set(vec![L!("--italics=off").into()]);
        assert_all_set(vec![L!("--reverse").into()]);
        assert_all_set(vec![L!("--reverse=off").into()]);
        assert_all_set(vec![L!("--strikethrough").into()]);
        assert_all_set(vec![L!("--strikethrough=off").into()]);
        assert_all_set(vec![L!("--underline").into()]);
        assert_all_set(vec![L!("--underline=off").into()]);
    }
}
