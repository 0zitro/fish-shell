// Implementation of the return builtin.

use std::ops::ControlFlow;

use crate::{builtins::Error, err_fmt, err_str, parser::BlockType};

use super::prelude::*;

#[derive(Debug, Clone, Copy, Default)]
struct Options {
    print_help: bool,
}

fn parse_options(
    args: &mut [&wstr],
    parser: &Parser,
    streams: &mut IoStreams,
) -> ControlFlow<ErrorCode, (Options, usize)> {
    let cmd = args[0];

    const SHORT_OPTS: &wstr = L!("h");
    const LONG_OPTS: &[WOption] = &[wopt(L!("help"), ArgType::NoArgument, 'h')];

    let mut opts = Options::default();

    let mut w = WGetopter::new(SHORT_OPTS, LONG_OPTS, args);

    while let Some(c) = w.next_opt() {
        match c {
            'h' => opts.print_help = true,
            ':' => {
                builtin_missing_argument(parser, streams, cmd, None, args[w.wopt_index - 1], true);
                return ControlFlow::Break(STATUS_INVALID_ARGS);
            }
            ';' => {
                builtin_unexpected_argument(parser, streams, cmd, args[w.wopt_index - 1], true);
                return ControlFlow::Break(STATUS_INVALID_ARGS);
            }
            '?' => {
                // We would normally invoke builtin_unknown_option() and return an error.
                // But for this command we want to let it try and parse the value as a negative
                // return value.
                return ControlFlow::Continue((opts, w.wopt_index - 1));
            }
            _ => {
                panic!("unexpected retval from next_opt");
            }
        }
    }

    ControlFlow::Continue((opts, w.wopt_index))
}

/// Function for handling the return builtin.
pub fn r#return(parser: &mut Parser, streams: &mut IoStreams, args: &mut [&wstr]) -> BuiltinResult {
    let mut retval = match parse_return_value(args, parser, streams) {
        ControlFlow::Continue(r) => r,
        ControlFlow::Break(result) => return result,
    };

    // *nix does not support negative return values, but our `return` builtin happily accepts being
    // called with negative literals (e.g. `return -1`).
    // Map negative values to (256 - their absolute value). This prevents `return -1` from
    // evaluating to a `$status` of 0 and keeps us from running into undefined behavior by trying to
    // left shift a negative value in W_EXITCODE().
    // Note in Rust, dividend % divisor has the same sign as the dividend.
    if retval < 0 {
        retval = 256 - (retval % 256).abs();
    }

    let retval = BuiltinResult::from_dynamic(retval);

    // The return target is the innermost enclosing boundary among a function
    // call, a command substitution and a pipeline stage.  A pipeline stage is
    // in-process for builtins and blocks, so it is compared by its block-stack
    // depth: a boundary block entered inside the stage is inner to it.
    let block_count = parser.blocks_size();
    let return_boundary_depth =
        parser
            .blocks_iter_rev()
            .enumerate()
            .find_map(|(depth_from_top, block)| {
                (block.is_function_call() || block.typ() == BlockType::Subst)
                    .then_some(block_count - 1 - depth_from_top)
            });
    let stage_is_innermost = parser
        .libdata()
        .pipeline_stage_depth
        .is_some_and(|stage_depth| {
            return_boundary_depth.is_none_or(|boundary_depth| boundary_depth < stage_depth)
        });

    if stage_is_innermost || return_boundary_depth.is_some() {
        // A function call, command substitution or pipeline stage consumes the
        // signal at its own boundary.
        parser.libdata_mut().returning = true;
        return retval;
    }

    // Outside those boundaries, exit the current script (but not an
    // interactive shell).
    if !parser.scope().is_interactive {
        parser.libdata_mut().exit_current_script = true;
    }
    retval
}

pub fn parse_return_value(
    args: &mut [&wstr],
    parser: &mut Parser,
    streams: &mut IoStreams,
) -> ControlFlow<BuiltinResult, i32> {
    let cmd = args[0];
    let (opts, optind) = parse_options(args, parser, streams).map_break(BuiltinResult::Err)?;

    if opts.print_help {
        builtin_print_help(parser, streams, cmd);
        return ControlFlow::Break(Ok(SUCCESS));
    }
    if optind + 1 < args.len() {
        err_str!(Error::TOO_MANY_ARGUMENTS)
            .cmd(cmd)
            .full_trailer(parser)
            .finish(streams);
        return ControlFlow::Break(Err(STATUS_INVALID_ARGS));
    }
    if optind == args.len() {
        ControlFlow::Continue(parser.last_status())
    } else {
        match fish_wcstoi(args[optind]) {
            Ok(i) => ControlFlow::Continue(i),
            Err(_e) => {
                err_fmt!(Error::NOT_NUMBER, args[1])
                    .cmd(cmd)
                    .full_trailer(parser)
                    .finish(streams);
                ControlFlow::Break(Err(STATUS_INVALID_ARGS))
            }
        }
    }
}
