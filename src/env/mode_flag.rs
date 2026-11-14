//! The options that set a variable's definition mode.
//!
//! `set` and `read` build their option tables from this one, each selecting the flags
//! it accepts; the visitor reads a definition's argument list against the same table.
//! A flag renamed here cannot keep highlighting an old spelling.

use fish_wgetopt::{ArgType, WOption, wopt};
use fish_widestring::{L, WExt as _, wstr};

use super::var::EnvMode;

/// One option of a definition command, and the mode field it sets.
#[derive(Clone, Copy, Debug)]
pub struct EnvModeFlag {
    /// The short spelling, as `wgetopt` takes it.
    ///
    /// A flag the commands spell only in its long form still carries one, because
    /// `wgetopt` identifies every matched option by the character it returns.
    pub short: char,
    /// The long spelling.
    pub long: &'static wstr,
    /// The mode field this flag sets.
    pub sets: EnvModeField,
}

impl EnvModeFlag {
    /// This flag as a `wgetopt` option.
    pub fn option(self) -> WOption<'static> {
        wopt(self.long, ArgType::NoArgument, self.short)
    }

    /// Whether `word` is this flag's spelling.
    pub fn whether_spelled_by(self, word: &wstr) -> bool {
        if word.len() < 2 || word.char_at(0) != '-' {
            return false;
        }
        if word.char_at(1) == '-' {
            return word[2..] == *self.long;
        }
        word.len() == 2 && word.char_at(1) == self.short
    }
}

/// Which field of an `EnvMode` a definition flag sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvModeField {
    Local,
    Function,
    Global,
    Universal,
    Export,
    Unexport,
    Pathvar,
    Unpathvar,
}

impl EnvModeField {
    /// Apply this field to a mode.
    pub fn set(self, mode: &mut EnvMode) {
        match self {
            Self::Local => mode.local = true,
            Self::Function => mode.function = true,
            Self::Global => mode.global = true,
            Self::Universal => mode.universal = true,
            Self::Export => mode.export = Some(true),
            Self::Unexport => mode.export = Some(false),
            Self::Pathvar => mode.pathvar = Some(true),
            Self::Unpathvar => mode.pathvar = Some(false),
        }
    }

    /// Whether the mode already requests this field.
    pub fn whether_requested(self, mode: &EnvMode) -> bool {
        match self {
            Self::Local => mode.local,
            Self::Function => mode.function,
            Self::Global => mode.global,
            Self::Universal => mode.universal,
            Self::Export => mode.export == Some(true),
            Self::Unexport => mode.export == Some(false),
            Self::Pathvar => mode.pathvar == Some(true),
            Self::Unpathvar => mode.pathvar == Some(false),
        }
    }

    /// The field whose request this one refuses, when a mode cannot carry both.
    pub fn denies(self) -> Option<Self> {
        match self {
            Self::Export => Some(Self::Unexport),
            Self::Unexport => Some(Self::Export),
            Self::Pathvar => Some(Self::Unpathvar),
            Self::Unpathvar => Some(Self::Pathvar),
            Self::Local | Self::Function | Self::Global | Self::Universal => None,
        }
    }
}

/// The flag value `wgetopt` returns for `--path`, which has no short spelling.
const PATHVAR_VALUE: char = '\u{11}';
/// The flag value `wgetopt` returns for `--unpath`.
const UNPATHVAR_VALUE: char = '\u{12}';

/// Every option that sets a definition mode, in one table.
pub static ENV_MODE_FLAGS: &[EnvModeFlag] = &[
    EnvModeFlag {
        short: 'l',
        long: L!("local"),
        sets: EnvModeField::Local,
    },
    EnvModeFlag {
        short: 'f',
        long: L!("function"),
        sets: EnvModeField::Function,
    },
    EnvModeFlag {
        short: 'g',
        long: L!("global"),
        sets: EnvModeField::Global,
    },
    EnvModeFlag {
        short: 'U',
        long: L!("universal"),
        sets: EnvModeField::Universal,
    },
    EnvModeFlag {
        short: 'x',
        long: L!("export"),
        sets: EnvModeField::Export,
    },
    EnvModeFlag {
        short: 'u',
        long: L!("unexport"),
        sets: EnvModeField::Unexport,
    },
    EnvModeFlag {
        short: PATHVAR_VALUE,
        long: L!("path"),
        sets: EnvModeField::Pathvar,
    },
    EnvModeFlag {
        short: UNPATHVAR_VALUE,
        long: L!("unpath"),
        sets: EnvModeField::Unpathvar,
    },
];

/// The flag `wgetopt` returned, when it belongs to a definition mode.
pub fn flag_of(value: char) -> Option<EnvModeFlag> {
    ENV_MODE_FLAGS
        .iter()
        .copied()
        .find(|flag| flag.short == value)
}

#[cfg(test)]
mod tests {
    use super::{ENV_MODE_FLAGS, EnvModeField, flag_of};
    use crate::env::EnvMode;
    use fish_widestring::{L, WString};

    #[test]
    fn a_flag_is_spelled_in_both_forms() {
        let local = ENV_MODE_FLAGS
            .iter()
            .copied()
            .find(|flag| flag.sets == EnvModeField::Local)
            .expect("local is declared");

        assert!(local.whether_spelled_by(L!("-l")));
        assert!(local.whether_spelled_by(L!("--local")));
        assert!(!local.whether_spelled_by(L!("-g")));
        assert!(!local.whether_spelled_by(L!("--global")));
        assert!(!local.whether_spelled_by(L!("local")));
    }

    #[test]
    fn a_field_sets_its_own_factor() {
        let mut mode = EnvMode::default();
        EnvModeField::Global.set(&mut mode);
        EnvModeField::Unexport.set(&mut mode);

        assert!(mode.global);
        assert_eq!(mode.export, Some(false));
        assert!(!mode.local);
        assert_eq!(mode.pathvar, None);
    }

    #[test]
    fn the_opposites_refuse_each_other() {
        assert_eq!(EnvModeField::Unexport.denies(), Some(EnvModeField::Export));
        assert_eq!(
            EnvModeField::Unpathvar.denies(),
            Some(EnvModeField::Pathvar)
        );
        assert_eq!(EnvModeField::Local.denies(), None);
    }

    #[test]
    fn every_flag_is_reachable_from_its_value() {
        for flag in ENV_MODE_FLAGS {
            let found = flag_of(flag.short).expect("the value names a flag");
            assert_eq!(found.sets, flag.sets);
            assert!(found.whether_spelled_by(&concat_long(flag.long)));
        }
    }

    /// The long spelling of a flag's name.
    fn concat_long(name: &fish_widestring::wstr) -> fish_widestring::WString {
        let mut long = WString::from(L!("--"));
        long.push_utfstr(name);
        long
    }
}
