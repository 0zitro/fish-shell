use std::ops::ControlFlow;

use super::prelude::*;
use super::r#return::parse_return_value;

/// Function for handling the exit builtin.
pub fn exit(parser: &mut Parser, streams: &mut IoStreams, args: &mut [&wstr]) -> BuiltinResult {
    let retval = match parse_return_value(args, parser, streams) {
        ControlFlow::Continue(r) => r,
        ControlFlow::Break(result) => return result,
    };

    // Only the session task ends the shell; any other task finishes here.
    if parser.current_task_is_session() {
        parser.libdata_mut().exit_current_script = true;
    } else {
        parser.finish_current_task();
    }

    BuiltinResult::from_dynamic(retval)
}
