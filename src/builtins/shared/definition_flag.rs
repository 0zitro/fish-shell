//! Applying a definition flag to the mode a definition command requests.

use crate::{
    builtins::prelude::*,
    env::{EnvMode, EnvModeField},
};

/// Apply one definition flag to the mode a definition command requests.
///
/// The two fields that name an opposite — export and unexport, path and unpath — cannot
/// both be requested, because a mode carries one value for each; the diagnostic names the
/// pair that was refused.  A scope field denies nothing.
pub fn apply_definition_flag(
    parser: &Parser,
    streams: &mut IoStreams,
    cmd: &wstr,
    mode: &mut EnvMode,
    field: EnvModeField,
) -> Result<(), ErrorCode> {
    let Some(denied) = field.denies() else {
        field.set(mode);
        return Ok(());
    };
    if !denied.whether_requested(mode) {
        field.set(mode);
        return Ok(());
    }

    // Only the two field pairs an `EnvMode` holds one value for reach this point.
    let message = if matches!(denied, EnvModeField::Pathvar | EnvModeField::Unpathvar) {
        Error::PATH_UNPATH
    } else {
        Error::EXPORT_UNEXPORT
    };
    err_str!(message)
        .cmd(cmd)
        .full_trailer(parser)
        .finish(streams);
    Err(STATUS_INVALID_ARGS)
}
