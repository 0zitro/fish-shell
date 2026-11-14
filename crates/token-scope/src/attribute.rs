//! The overlays applied to a character's style after its scope resolves.

use std::sync::LazyLock;

use crate::entries::ColorVariableName;

/// One overlay applied to a character's style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum HighlightAttribute {
    /// A parameter or a redirection target that names an existing file.
    PathExisting,
    /// The visual selection.
    Selection,
    /// A history-search match, and the flash of an autosuggestion.
    SearchMatch,
    /// The autosuggestion range.
    Autosuggestion,
    /// The underline of the pager search field.
    ForcedUnderline,
}

impl HighlightAttribute {
    /// Every declared attribute that keeps a variable, in application order.
    ///
    /// The order is the declared `application_order` of each entry, not the
    /// order the table states them in.
    pub fn entries() -> &'static [HighlightAttributeEntry] {
        static ORDERED: LazyLock<Vec<HighlightAttributeEntry>> = LazyLock::new(|| {
            let mut entries = ATTRIBUTE_ENTRIES.to_vec();
            entries.sort_by_key(|entry| entry.application_order);
            entries
        });

        ORDERED.as_slice()
    }

    /// This attribute's entry, or none when the attribute keeps no variable.
    ///
    /// The pager search field's forced underline is an attribute with a fixed
    /// effect: it is no variable's value, so it has no entry.
    pub fn entry(self) -> Option<&'static HighlightAttributeEntry> {
        Self::entries().iter().find(|entry| entry.attribute == self)
    }
}

/// The set of attributes carried by one character.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct HighlightAttributeSet(u8);

impl HighlightAttributeSet {
    /// Whether the set holds this attribute.
    pub fn has_attribute(self, attribute: HighlightAttribute) -> bool {
        self.0 & (1 << attribute as u8) != 0
    }

    /// A set holding this attribute in addition to what this one holds.
    pub fn with_attribute(self, attribute: HighlightAttribute) -> Self {
        Self(self.0 | (1 << attribute as u8))
    }

    /// Whether the set holds no attribute.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// One field of a text style that an attribute can write.
///
/// A field names one field of the style.  This is not the terminal's paintable
/// region, which selects where one SGR color is painted and has no field for the
/// flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleField {
    /// The foreground color.
    Foreground,
    /// The background color.
    Background,
    /// The underline color.
    UnderlineColor,
    /// The resettable flags: bold, underline kind, italics, dim, reverse,
    /// strikethrough, as one group.
    Flags,
}

/// The fields of a text style that one attribute writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyleFieldSet(u8);

impl StyleFieldSet {
    /// The set holding these fields.
    pub(crate) const fn of(fields: &[StyleField]) -> Self {
        let mut bits = 0u8;
        let mut index = 0;
        while index < fields.len() {
            bits |= 1 << fields[index] as u8;
            index += 1;
        }
        Self(bits)
    }

    /// Whether the set holds this field.
    pub fn has_field(self, field: StyleField) -> bool {
        self.0 & (1 << field as u8) != 0
    }
}

/// Every field of a style: what an attribute that overrides a whole style
/// writes.
const EVERY_FIELD: &[StyleField] = &[
    StyleField::Foreground,
    StyleField::Background,
    StyleField::UnderlineColor,
    StyleField::Flags,
];

/// How an attribute's value combines with the style the scopes resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttributeMode {
    /// The value's named fields are written over the resolved style; the rest of
    /// the style stands, and an unset variable contributes nothing.
    Merge(StyleFieldSet),
    /// The field is discarded for the fields the mode names, and the value's
    /// fields take their place; a field the value leaves unnamed, or a variable
    /// that is unset, takes the terminal default.
    Replace(StyleFieldSet),
}

/// One declared attribute.
#[derive(Clone, Copy, Debug)]
pub struct HighlightAttributeEntry {
    /// The entry's attribute.
    pub attribute: HighlightAttribute,
    /// The variable that carries it.
    pub variable_name: ColorVariableName,
    /// How the value combines with the resolved style, and which fields it may
    /// write.
    pub mode: AttributeMode,
    /// Its position among the attributes, lowest applied first.  It is a declared
    /// value, not the place of the entry in the declaration.
    pub application_order: u8,
}

/// The attributes that keep a variable.
///
/// The field sets are the behaviour each variable has: a search match supplies
/// its background and its foreground only where the value names one, a selection
/// replaces the whole style, and an autosuggestion replaces the foreground it
/// names while the background the scopes composed stands.
static ATTRIBUTE_ENTRIES: &[HighlightAttributeEntry] = &[
    HighlightAttributeEntry {
        attribute: HighlightAttribute::SearchMatch,
        variable_name: ColorVariableName(fish_widestring::L!("fish_color_search_match")),
        mode: AttributeMode::Merge(StyleFieldSet::of(&[
            StyleField::Foreground,
            StyleField::Background,
        ])),
        application_order: 10,
    },
    HighlightAttributeEntry {
        attribute: HighlightAttribute::Selection,
        variable_name: ColorVariableName(fish_widestring::L!("fish_color_selection")),
        mode: AttributeMode::Replace(StyleFieldSet::of(EVERY_FIELD)),
        application_order: 20,
    },
    HighlightAttributeEntry {
        attribute: HighlightAttribute::Autosuggestion,
        variable_name: ColorVariableName(fish_widestring::L!("fish_color_autosuggestion")),
        mode: AttributeMode::Replace(StyleFieldSet::of(&[
            StyleField::Foreground,
            StyleField::UnderlineColor,
            StyleField::Flags,
        ])),
        application_order: 30,
    },
    HighlightAttributeEntry {
        attribute: HighlightAttribute::PathExisting,
        variable_name: ColorVariableName(fish_widestring::L!("fish_color_valid_path")),
        mode: AttributeMode::Merge(StyleFieldSet::of(EVERY_FIELD)),
        application_order: 40,
    },
];
