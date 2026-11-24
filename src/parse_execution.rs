//! Provides the "linkage" between an ast and actual execution structures (job_t, etc.).

use crate::{
    ast::{
        self, BlockStatementHeader, Keyword as _, Leaf as _, Node as _, Statement, Token as _,
        unescape_keyword,
    },
    builtins::{
        self, Error, STATUS_CMD_ERROR, STATUS_CMD_OK, STATUS_CMD_UNKNOWN, STATUS_EXPAND_ERROR,
        STATUS_ILLEGAL_CMD, STATUS_INVALID_ARGS, STATUS_NOT_EXECUTABLE, STATUS_UNMATCHED_WILDCARD,
        builtin_exists,
    },
    common::{get_program_name, valid_func_name, valid_var_name},
    complete::CompletionList,
    env::{EnvMode, EnvStackSetResult, EnvVar, Environment as _, Statuses, handle_env_return},
    err_fmt,
    event::{self, Event},
    exec::{
        PartialPipes, PendingStage, abort_pipeline_from, allow_exec_with_background_jobs,
        exec_builtin_process, exec_external_command, function_prepare_environment,
        function_restore_environment, get_deferred_process, internal_exec, launch_stage,
        pop_assignment_block, push_assignment_block, run_internal_process_or_short_circuit,
    },
    expand::{
        self, ExpandFlags, ExpandResultCode, expand_one, expand_string, expand_to_command_and_args,
    },
    fds::{PIPE_ERROR, make_autoclose_pipes},
    flog::flog,
    flogf, function,
    io::{FdOutputStream, IoBufferfill, IoChain, IoStreams, OutputStream, StringOutputStream},
    job_group::JobGroup,
    operation_context::OperationContext,
    parse_constants::{
        CALL_STACK_LIMIT_EXCEEDED_ERR_MSG, ERROR_TIME_BACKGROUND,
        FAILED_EXPANSION_VARIABLE_NAME_ERR_MSG, ILLEGAL_FD_ERR_MSG,
        INFINITE_FUNC_RECURSION_ERR_MSG, ParseError, ParseErrorCode, ParseErrorList, ParseKeyword,
        ParseTokenType, StatementDecoration, parse_error_offset_source_start,
    },
    parse_tree::{NodeRef, ParsedSourceRef},
    parse_util::{locate_cmdsubst_range, unescape_wildcards},
    parser::{
        Block, BlockData, BlockId, BlockType, LoopStatus, Parser, ParserEnvSetMode, ProfileItem,
        ScopedData, node_ref_of,
    },
    parser_keywords::parser_keywords_is_subcommand,
    path::{path_as_implicit_cd, path_try_get_path},
    prelude::*,
    proc::{
        ConcreteAssignment, Job, JobControl, JobProperties, ProcStatus, Process, ProcessType,
        TaskStage, get_job_control_mode, job_reap, no_exec, process_mark_finished_children,
    },
    reader::fish_is_unwinding_for_exit,
    redirection::{RedirectionMode, RedirectionSpec, RedirectionSpecList},
    signal::RawSignal,
    task::{StageTask, TaskId, TaskLifecycle, WaitReason},
    task_frame::{
        AndorJobListFrame, BeginFrame, ConjunctionFrame, ForFrame, Frame, FrameStack,
        FunctionCallFrame, IfFrame, IfStage, JobFrame, JobListFrame, JobPhase, ProfileItemFinish,
        SimpleBlockJobFrame, StageFrame, StatementFrame, SwitchFrame, WhileFrame, WhileStage,
    },
    timer::push_timer,
    tokenizer::{PipeOrRedir, TokenType, variable_assignment_equals_pos},
    trace::{trace_if_enabled, trace_if_enabled_with_args},
    wildcard::wildcard_match,
};
use fish_common::{ScopeGuard, escape, help_section, truncate_at_nul};
use fish_widestring::WExt as _;
use libc::{EXIT_SUCCESS, STDERR_FILENO, STDOUT_FILENO, c_int};
use nix::errno::Errno;
use std::{io::ErrorKind, rc::Rc, sync::Arc};

/// An eval_result represents evaluation errors including wildcards which failed to match, syntax
/// errors, or other expansion errors. It also tracks when evaluation was skipped due to signal
/// cancellation. Note it does not track the exit status of commands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EndExecutionReason {
    /// Evaluation was successful.
    Ok,

    /// Evaluation was skipped due to control flow (break or return).
    ControlFlow,

    /// Evaluation was cancelled, e.g. because of a signal or exit.
    Cancelled,

    /// A parse error or failed expansion (but not an error exit status from a command).
    Error,
}

pub struct ExecutionContext {
    // The parsed source and its AST.
    pstree: ParsedSourceRef,

    // If set, one of our processes received a cancellation signal (INT or QUIT) so we are
    // unwinding.
    cancel_signal: Option<RawSignal>,

    /// The block IO chain.
    /// For example, in `begin; foo ; end < file.txt` this would have the 'file.txt' IO.
    block_io: IoChain,

    /// Whether the next job this context runs starts in the background.
    background_next_job: bool,

    /// The job group nested jobs reuse.
    pub(crate) job_group: Option<crate::proc::JobGroupRef>,
}

// Report an error, setting $status to `status`. Always returns
// 'end_execution_reason_t::error'.
macro_rules! report_error {
    ( $self:ident, $ctx:expr, $status:expr, $node:expr, $fmt:expr $(, $arg:expr )* $(,)? ) => {
        report_error_formatted!($self, $ctx, $status, $node, wgettext_fmt!($fmt $(, $arg )*))
    };
}
macro_rules! report_error_formatted {
    ( $self:ident, $ctx:expr, $status:expr, $node:expr, $text:expr $(,)? ) => {{
        let r = $node.source_range();
        // Create an error.
        let mut error = ParseError::default();
        error.source_start = r.start();
        error.source_length = r.length();
        error.code = ParseErrorCode::Syntax; // hackish
        error.text = $text;
        $self.report_errors($ctx, $status, &vec![error])
    }};
}

pub fn varname_error<'a>(command: &'a wstr, bad_name: &'a wstr) -> Error<'a> {
    err_fmt!(
        Error::INVALID_VARNAME,
        bad_name,
        help_section!("language#shell-variable-and-function-names")
    )
    .cmd(command)
}

/// The action one frame step takes.
enum FrameAction {
    /// Push a child frame; the frame waits for the child's outcome.
    Push(Box<Frame>),

    /// The frame is complete with this reason.
    Done(EndExecutionReason),

    /// The frame waits for an event before it can continue.
    Block(WaitReason),
}

/// The outcome of evaluating one task's frames.
pub(crate) enum EvalOutcome {
    /// Every frame completed with this reason.
    Completed(EndExecutionReason),

    /// The evaluation waits for an event; its frame records where it continues.
    Blocked(WaitReason),
}

/// Return a push action for `frame`.
fn push_frame(frame: Frame) -> FrameAction {
    FrameAction::Push(Box::new(frame))
}

/// The outcome of running one job.
enum JobStep {
    /// The job completed with this reason.
    Completed(EndExecutionReason),

    /// The job is a simple block; this frame evaluates it.
    Frame(Box<Frame>),
}

impl ExecutionContext {
    /// Construct a context in preparation for evaluating a node in a tree, with the given block_io.
    /// The execution context may access the parser and parent job group (if any) through ctx.
    pub fn new(pstree: ParsedSourceRef, block_io: IoChain) -> Self {
        Self {
            pstree,
            cancel_signal: None,
            block_io,
            background_next_job: false,
            job_group: None,
        }
    }

    /// Construct a context whose next job starts in the background.
    pub fn new_with_background_job(pstree: ParsedSourceRef, block_io: IoChain) -> Self {
        Self {
            background_next_job: true,
            ..Self::new(pstree, block_io)
        }
    }

    fn pstree(&self) -> &ParsedSourceRef {
        &self.pstree
    }

    pub fn cancel_signal(&self) -> Option<RawSignal> {
        self.cancel_signal
    }

    /// Evaluate frames until the stack empties or a frame blocks.
    pub(crate) fn evaluate_frames(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frames: &mut FrameStack,
        initial: Frame,
    ) -> EvalOutcome {
        let mut frame = initial;
        let mut child_result = None;
        loop {
            let source = frame.parsed_source();
            self.pstree = source;
            ctx.job_group = self.job_group.clone();
            match self.step_frame(ctx, &mut frame, child_result.take()) {
                FrameAction::Push(child) => {
                    frames.push(frame);
                    frame = *child;
                }
                FrameAction::Done(reason) => match frames.pop() {
                    Some(parent) => {
                        child_result = Some(reason);
                        frame = parent;
                    }
                    None => return EvalOutcome::Completed(reason),
                },
                FrameAction::Block(reason) => {
                    frames.push(frame);
                    return EvalOutcome::Blocked(reason);
                }
            }
        }
    }

    /// Evaluate a frame stack that already records its frames.
    pub(crate) fn evaluate_frame_stack(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frames: &mut FrameStack,
    ) -> EvalOutcome {
        match frames.pop() {
            Some(initial) => self.evaluate_frames(ctx, frames, initial),
            None => EvalOutcome::Completed(EndExecutionReason::Ok),
        }
    }

    /// Advance one frame by one step.
    fn step_frame(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut Frame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        match frame {
            Frame::JobList(frame) => self.step_job_list(ctx, frame, child_result),
            Frame::AndorJobList(frame) => self.step_andor_job_list(ctx, frame, child_result),
            Frame::Conjunction(frame) => self.step_conjunction(ctx, frame, child_result),
            Frame::Statement(frame) => self.step_statement(ctx, frame, child_result),
            Frame::SimpleBlockJob(frame) => self.step_simple_block_job(ctx, frame, child_result),
            Frame::FunctionCall(frame) => self.step_function_call(ctx, frame, child_result),
            Frame::Job(frame) => self.step_job_frame(ctx, frame, child_result),
            Frame::Stage(frame) => self.step_stage_frame(ctx, frame, child_result),
            Frame::Begin(frame) => self.step_begin(ctx, frame, child_result),
            Frame::If(frame) => self.step_if(ctx, frame, child_result),
            Frame::While(frame) => self.step_while(ctx, frame, child_result),
            Frame::For(frame) => self.step_for(ctx, frame, child_result),
            Frame::Switch(frame) => self.step_switch(ctx, frame, child_result),
        }
    }

    /// Advance a job frame.
    fn step_job_frame(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut JobFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            if reason != EndExecutionReason::Ok && frame.end_reason.is_none() {
                frame.end_reason = Some(reason);
            }

            let process_index = frame
                .active_process_index
                .take()
                .expect("a completed stage records its process");
            frame.processes_launched += 1;
            if frame.suspended_at.is_none()
                && ctx.parser().suspension_count() != frame.suspension_count
            {
                frame.suspended_at = Some(process_index);
            }
            let process = &frame.job.processes()[process_index];
            if process.leads_pgrp && frame.job.group().wants_terminal() {
                frame
                    .handoff
                    .to_job_group(frame.job.group.as_ref().unwrap());
            }
            if frame.active_stage_is_deferred {
                frame.active_stage_is_deferred = false;
            } else {
                frame.stage_launch_index += 1;
            }
        }

        loop {
            match frame.phase {
                JobPhase::Launch => {
                    if !frame.started {
                        if frame.job.processes()[0].is_exec() {
                            if ctx.parser().current_task_is_session() {
                                return self.launch_exec_job(ctx, frame);
                            }
                            // A task's exec starts the command as a process stage and discards the
                            // task's evaluation.
                            let process = &frame.job.processes()[0];
                            if exec_external_command(
                                ctx.parser(),
                                &frame.job,
                                process,
                                &frame.block_io,
                            )
                            .is_err()
                            {
                                process.mark_aborted_before_launch();
                                remove_job(ctx.parser(), &frame.job);
                                frame.end_reason.get_or_insert(EndExecutionReason::Error);
                                return self.finish_job_frame(ctx, frame);
                            }
                            let _ = process.pid().map(|pid| {
                                ctx.parser().mut_wait_handles().remove_by_pid(pid);
                            });
                            ctx.parser().finish_current_task();
                            frame.job.mark_constructed();
                            frame.started = true;
                            frame.phase = JobPhase::Wait;
                            continue;
                        }
                        frame.deferred_process = get_deferred_process(&frame.job);
                        frame.suspension_count = ctx.parser().suspension_count();
                        frame.started = true;
                    }

                    if frame.stage_launch_index < frame.job.processes().len() {
                        let index = frame.stage_launch_index;
                        let process = &frame.job.processes()[index];
                        let mut proc_pipes = PartialPipes::default();
                        std::mem::swap(&mut proc_pipes.read, &mut frame.pipe_read_for_next_stage);
                        if !process.is_last_in_job {
                            let Ok(pipes) = make_autoclose_pipes() else {
                                flog!(warning, wgettext!(PIPE_ERROR));
                                abort_pipeline_from(&frame.job, index);
                                frame.pipeline_aborted = true;
                                frame.stage_launch_index = frame.job.processes().len();
                                frame.pipe_read_for_next_stage = None;
                                continue;
                            };
                            frame.pipe_read_for_next_stage = Some(pipes.read);
                            proc_pipes.write = Some(pipes.write);
                            if Some(index) == frame.deferred_process {
                                frame.deferred_pipes = proc_pipes;
                                frame.stage_launch_index += 1;
                                continue;
                            }
                        }

                        let deferred_pipes = &frame.deferred_pipes;
                        match launch_stage(
                            ctx.parser(),
                            &frame.job,
                            process,
                            frame.block_io.clone(),
                            proc_pipes,
                            deferred_pipes,
                            false,
                        ) {
                            Ok(Some(stage)) => {
                                frame.active_process_index = Some(index);
                                frame.active_stage_is_deferred = false;
                                return Self::push_stage_frame(frame, stage);
                            }
                            Ok(None) => {
                                frame.processes_launched += 1;
                                if frame.suspended_at.is_none()
                                    && ctx.parser().suspension_count() != frame.suspension_count
                                {
                                    frame.suspended_at = Some(index);
                                }
                                if process.leads_pgrp && frame.job.group().wants_terminal() {
                                    frame
                                        .handoff
                                        .to_job_group(frame.job.group.as_ref().unwrap());
                                }
                                frame.stage_launch_index += 1;
                                continue;
                            }
                            Err(()) => {
                                frame.pipeline_aborted = true;
                                abort_pipeline_from(&frame.job, index);
                                frame.stage_launch_index = frame.job.processes().len();
                                frame.pipe_read_for_next_stage = None;
                                continue;
                            }
                        }
                    }

                    if !frame.deferred_stage_launched {
                        frame.deferred_stage_launched = true;
                        if let Some(dp) = frame.deferred_process {
                            if frame.pipeline_aborted {
                                frame.job.processes()[dp].mark_aborted_before_launch();
                            } else {
                                let pipes = std::mem::take(&mut frame.deferred_pipes);
                                match launch_stage(
                                    ctx.parser(),
                                    &frame.job,
                                    &frame.job.processes()[dp],
                                    frame.block_io.clone(),
                                    pipes,
                                    &PartialPipes::default(),
                                    true,
                                ) {
                                    Ok(Some(stage)) => {
                                        frame.active_process_index = Some(dp);
                                        frame.active_stage_is_deferred = true;
                                        return Self::push_stage_frame(frame, stage);
                                    }
                                    Ok(None) => {
                                        frame.processes_launched += 1;
                                        if frame.suspended_at.is_none()
                                            && ctx.parser().suspension_count()
                                                != frame.suspension_count
                                        {
                                            frame.suspended_at = Some(dp);
                                        }
                                    }
                                    Err(()) => {
                                        frame.job.processes()[dp].mark_aborted_before_launch();
                                    }
                                }
                            }
                        }
                        continue;
                    }

                    frame.pipe_read_for_next_stage = None;
                    if frame.pipeline_aborted && frame.processes_launched == 0 {
                        if let Some(statuses) = frame.job.statuses() {
                            ctx.parser().set_last_statuses(statuses);
                            ctx.parser().libdata_mut().status_count += 1;
                        }
                        remove_job(ctx.parser(), &frame.job);
                        return self.finish_job_frame(ctx, frame);
                    }

                    flogf!(
                        exec_job_exec,
                        "Executed job %d from command '%s'",
                        frame.job.job_id(),
                        frame.job.command()
                    );
                    frame.job.mark_constructed();
                    if !frame.job.is_foreground()
                        && let Some(last_pid) = frame.job.last_pid()
                    {
                        ctx.parser().set_one(
                            L!("last_pid"),
                            ParserEnvSetMode::new(EnvMode::GLOBAL),
                            last_pid.to_wstring(),
                        );
                    }
                    frame.phase = JobPhase::Wait;
                }
                JobPhase::Wait => {
                    if frame.job.is_initially_background() {
                        return self.finish_job_frame(ctx, frame);
                    }

                    if let Some(suspend_index) = frame.suspended_at {
                        // The stages after the suspending stage are its consumers and run to
                        // completion; the stages before it produce the suspended stream.
                        while frame.job.processes()[suspend_index + 1..]
                            .iter()
                            .any(|process| !process.is_completed() && !process.is_stopped())
                        {
                            ctx.parser().run_runnable_tasks();
                            process_mark_finished_children(
                                ctx.parser(),
                                true,
                                Some(&frame.block_io),
                            );
                        }
                        crate::proc::collect_finished_process_output(&frame.block_io);
                        if let Some(statuses) = frame.job.statuses() {
                            ctx.parser().set_last_statuses(statuses);
                            ctx.parser().libdata_mut().status_count += 1;
                        }
                        if !frame.job.is_completed() {
                            frame.job.group().set_is_foreground(false);
                        }
                        return self.finish_job_frame(ctx, frame);
                    }

                    if frame.job.is_completed() {
                        crate::proc::collect_finished_process_output(&frame.block_io);
                        let last_process = frame
                            .job
                            .processes()
                            .last()
                            .expect("the job records its last stage");
                        let last_process_status = last_process.status();
                        let last_process_exited = last_process_status.normal_exited()
                            || last_process_status.signal_exited();
                        if last_process_exited && let Some(statuses) = frame.job.statuses() {
                            ctx.parser().set_last_statuses(statuses);
                            ctx.parser().libdata_mut().status_count += 1;
                        }
                        return self.finish_job_frame(ctx, frame);
                    }

                    if ctx
                        .parser()
                        .job_with_internal_id(frame.job.internal_job_id)
                        .is_none()
                    {
                        // The job entry was removed while it ran, as a disowned job is.
                        return self.finish_job_frame(ctx, frame);
                    }

                    if frame.job.is_stopped() || fish_is_unwinding_for_exit() {
                        if frame.job.is_stopped() {
                            frame.handoff.save_tty_modes();
                        }
                        return self.finish_job_frame(ctx, frame);
                    }

                    return FrameAction::Block(WaitReason::Job(frame.job.internal_job_id));
                }
            }
        }
    }

    fn push_stage_frame(frame: &JobFrame, stage: PendingStage) -> FrameAction {
        let parsed_source = match &stage {
            PendingStage::Builtin { .. } => frame.parsed_source.clone(),
            PendingStage::Function { props, .. } => props.func_node.parsed_source_ref(),
            PendingStage::Block { statement, .. } => statement.parsed_source_ref(),
        };
        push_frame(Frame::Stage(Box::new(StageFrame {
            job: Rc::clone(&frame.job),
            node: frame.node.clone(),
            stage,
            assignment_block: None,
            stage_scope_block: None,
            started: false,
            block_io_before: None,
            node_before: None,
            scoped_data_before: None,
            job_group_before: None,
            output_bufferfill: None,
            parsed_source,
        })))
    }

    /// Launch an exec job: it replaces the shell process and returns only on failure.
    fn launch_exec_job(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut JobFrame,
    ) -> FrameAction {
        if allow_exec_with_background_jobs(ctx.parser()) {
            for assignment in &frame.job.processes()[0].variable_assignments {
                ctx.parser().set_var(
                    &assignment.variable_name,
                    ParserEnvSetMode::new(EnvMode::LOCAL_EXPORTED),
                    assignment.values.clone(),
                );
            }
            let parser = ctx.parser();
            let is_repainting = parser.is_repainting();
            internal_exec(
                parser.vars(),
                is_repainting,
                &frame.job,
                frame.block_io.clone(),
            );
            let status = if frame.job.flags().negate { 0 } else { 1 };
            ctx.parser().set_last_statuses(Statuses::just(status));
        }
        for process in frame.job.processes().iter() {
            process.mark_aborted_before_launch();
        }
        remove_job(ctx.parser(), &frame.job);
        frame.end_reason.get_or_insert(EndExecutionReason::Error);
        self.finish_job_frame(ctx, frame)
    }

    /// Finish a job frame: record its profile entry, its universal variables and its cancellation.
    fn finish_job_frame(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut JobFrame,
    ) -> FrameAction {
        let cancel_signal = finish_job_frame_record(ctx.parser(), frame, Some(&self.block_io));
        if self.cancel_signal.is_none() {
            self.cancel_signal = cancel_signal;
        }
        FrameAction::Done(frame.end_reason.unwrap_or(EndExecutionReason::Ok))
    }

    /// Advance a shell-evaluated job stage.
    fn step_stage_frame(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut StageFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            let process_index = stage_process_index(&frame.stage);
            let chain = stage_chain(&frame.stage).clone();
            self.restore_stage_scope(ctx, frame);
            if let Some(stage_scope_block) = frame.stage_scope_block.take() {
                ctx.parser().pop_block(stage_scope_block);
            }
            pop_assignment_block(ctx.parser(), frame.assignment_block.take());
            if let Some(saved) = frame.block_io_before.take() {
                self.block_io = saved;
            }

            if let Some(bufferfill) = frame.output_bufferfill.take() {
                // The stage wrote into a capture; the bytes enter the pipe row now that the
                // processes reading the row are launched.
                let contents = IoBufferfill::finish(bufferfill).newline_serialized();
                let process = &frame.job.processes()[process_index];
                process
                    .status
                    .set(ProcStatus::from_exit_code(ctx.parser().last_status()));
                run_internal_process_or_short_circuit(
                    ctx.parser(),
                    &frame.job,
                    process,
                    contents,
                    vec![],
                    &chain,
                );
                return FrameAction::Done(EndExecutionReason::Ok);
            }

            let status = ctx.parser().last_status();
            complete_stage_process(ctx.parser(), &frame.job, process_index, &chain, status);
            return FrameAction::Done(reason);
        }

        frame.started = true;
        let mut chain = stage_chain(&frame.stage).clone();
        let assignments = match &frame.stage {
            PendingStage::Builtin {
                variable_assignments,
                ..
            }
            | PendingStage::Function {
                variable_assignments,
                ..
            }
            | PendingStage::Block {
                variable_assignments,
                ..
            } => variable_assignments,
        };

        let piped_output_needs_buffering = match &frame.stage {
            PendingStage::Builtin { .. } => false,
            PendingStage::Function {
                piped_output_needs_buffering,
                ..
            }
            | PendingStage::Block {
                piped_output_needs_buffering,
                ..
            } => *piped_output_needs_buffering,
        };
        if piped_output_needs_buffering {
            match IoBufferfill::create() {
                Ok(bufferfill) => {
                    chain.push(bufferfill.clone());
                    frame.output_bufferfill = Some(bufferfill);
                }
                Err(_) => {
                    abort_pipeline_from(&frame.job, stage_process_index(&frame.stage));
                    return FrameAction::Done(EndExecutionReason::Error);
                }
            }
        }

        // The stage runs with its job's node and one more evaluation level, so a diagnostic the
        // stage reports carries the command line it came from.
        frame.node_before = Some(
            ctx.parser()
                .current_node()
                .replace(Some(frame.node.clone())),
        );
        let mut scope = ctx.parser().scope();
        frame.scoped_data_before = Some(scope);
        scope.eval_level += 1;
        ctx.parser().set_scope(scope);
        frame.job_group_before = Some(self.job_group.take());
        self.job_group = frame.job.group.clone();

        frame.assignment_block = push_assignment_block(ctx.parser(), assignments);
        if matches!(
            &frame.stage,
            PendingStage::Function { .. } | PendingStage::Block { .. }
        ) {
            frame.stage_scope_block =
                Some(ctx.parser().push_block(Block::scope_block(BlockType::Top)));
        }
        frame.block_io_before = Some(std::mem::replace(&mut self.block_io, chain));

        match &frame.stage {
            PendingStage::Builtin {
                chain,
                piped_output_needs_buffering,
                ..
            } => {
                let process_index = stage_process_index(&frame.stage);
                exec_builtin_process(
                    ctx.parser(),
                    &frame.job,
                    &frame.job.processes()[process_index],
                    chain,
                    *piped_output_needs_buffering,
                )
                .expect("a builtin stage launches");
                self.restore_stage_scope(ctx, frame);
                if let Some(saved) = frame.block_io_before.take() {
                    self.block_io = saved;
                }
                pop_assignment_block(ctx.parser(), frame.assignment_block.take());
                FrameAction::Done(EndExecutionReason::Ok)
            }
            PendingStage::Function { props, argv, .. } => {
                push_frame(Frame::FunctionCall(Box::new(FunctionCallFrame {
                    props: Arc::clone(props),
                    argv: argv.clone(),
                    function_block: None,
                    scope_block: None,
                    prev_exec_count: 0,
                })))
            }
            PendingStage::Block { statement, .. } => push_frame(Frame::Statement(StatementFrame {
                statement: statement.clone(),
                associated_block: None,
                started: false,
            })),
        }
    }

    /// Restore the node, evaluation level and job group a stage saved.
    fn restore_stage_scope(&mut self, ctx: &mut OperationContext<'_>, frame: &mut StageFrame) {
        if let Some(saved) = frame.scoped_data_before.take() {
            ctx.parser().set_scope(saved);
        }
        if let Some(saved) = frame.node_before.take() {
            ctx.parser().current_node().replace(saved);
        }
        if let Some(saved) = frame.job_group_before.take() {
            self.job_group = saved;
        }
    }

    /// Advance a command list frame.
    fn step_job_list(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut JobListFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            frame.result = reason;
            frame.next_conjunction += 1;
        }
        if !frame.checked {
            frame.checked = true;
            // Check for infinite recursion: a function which immediately calls itself.
            let mut func_name = WString::new();
            if let Some(infinite_recursive_node) =
                self.infinite_recursive_statement_in_job_list(ctx, &frame.jobs, &mut func_name)
            {
                return FrameAction::Done(report_error!(
                    self,
                    ctx,
                    STATUS_CMD_ERROR,
                    infinite_recursive_node,
                    INFINITE_FUNC_RECURSION_ERR_MSG,
                    func_name
                ));
            }

            // Check for stack overflow in case of function calls (regular stack overflow) or
            // string substitution blocks, which can be recursively called with eval (issue #9302).
            let block_type = frame.associated_block.map_or(BlockType::Top, |block| {
                ctx.parser().block_with_id(block).typ()
            });
            if (block_type == BlockType::Top && ctx.parser().function_stack_is_overflowing())
                || (block_type == BlockType::Subst && ctx.parser().is_eval_depth_exceeded())
            {
                return FrameAction::Done(report_error!(
                    self,
                    ctx,
                    STATUS_CMD_ERROR,
                    &frame.jobs,
                    CALL_STACK_LIMIT_EXCEEDED_ERR_MSG
                ));
            }
        }
        if frame.next_conjunction >= frame.jobs.len() {
            return FrameAction::Done(frame.result);
        }
        let conjunction = node_ref_of(&frame.jobs, &frame.jobs[frame.next_conjunction]);
        push_frame(Frame::Conjunction(ConjunctionFrame {
            conjunction,
            associated_block: frame.associated_block,
            next_job: 0,
            tested_decorator: false,
            awaiting_child: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Advance an and-or list frame.
    fn step_andor_job_list(
        &mut self,
        _ctx: &mut OperationContext<'_>,
        frame: &mut AndorJobListFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            frame.result = reason;
            frame.next_job += 1;
        }
        if frame.next_job >= frame.jobs.len() {
            return FrameAction::Done(frame.result);
        }
        let conjunction = node_ref_of(&frame.jobs, &frame.jobs[frame.next_job].job);
        push_frame(Frame::Conjunction(ConjunctionFrame {
            conjunction,
            associated_block: frame.associated_block,
            next_job: 0,
            tested_decorator: false,
            awaiting_child: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Advance a conjunction frame.
    fn step_conjunction(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut ConjunctionFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            frame.result = reason;
            frame.next_job += 1;
            frame.awaiting_child = false;
        }
        if !frame.tested_decorator {
            frame.tested_decorator = true;
            if let Some(reason) = self.check_end_execution(ctx) {
                return FrameAction::Done(reason);
            }
            if let Some(decorator) = &frame.conjunction.decorator {
                let last_status = ctx.parser().last_status();
                let skip = match decorator.keyword() {
                    ParseKeyword::And => last_status != 0,
                    ParseKeyword::Or => last_status == 0,
                    _ => unreachable!(),
                };
                if skip {
                    return FrameAction::Done(EndExecutionReason::Ok);
                }
            }
        }
        loop {
            if frame.next_job == 0 {
                let job_node = node_ref_of(&frame.conjunction, &frame.conjunction.job);
                match self.run_job_step(ctx, &job_node, frame.associated_block) {
                    JobStep::Completed(reason) => {
                        frame.result = reason;
                        frame.next_job += 1;
                        continue;
                    }
                    JobStep::Frame(child) => {
                        frame.awaiting_child = true;
                        return FrameAction::Push(child);
                    }
                }
            }
            let index = frame.next_job - 1;
            if index >= frame.conjunction.continuations.len() {
                return FrameAction::Done(frame.result);
            }
            if let Some(reason) = self.check_end_execution(ctx) {
                return FrameAction::Done(reason);
            }
            let continuation = &frame.conjunction.continuations[index];
            let last_status = ctx.parser().last_status();
            let skip = match continuation.conjunction.token_type() {
                ParseTokenType::AndAnd => last_status != 0,
                ParseTokenType::OrOr => last_status == 0,
                _ => unreachable!(),
            };
            if skip {
                frame.next_job += 1;
                continue;
            }
            let job_node = node_ref_of(&frame.conjunction, &continuation.job);
            match self.run_job_step(ctx, &job_node, frame.associated_block) {
                JobStep::Completed(reason) => {
                    frame.result = reason;
                    frame.next_job += 1;
                }
                JobStep::Frame(child) => {
                    frame.awaiting_child = true;
                    return FrameAction::Push(child);
                }
            }
        }
    }

    /// Run one job, or return the frame that evaluates it when it is a simple block.
    fn run_job_step(
        &mut self,
        ctx: &mut OperationContext<'_>,
        job_node: &NodeRef<ast::JobPipeline>,
        associated_block: Option<BlockId>,
    ) -> JobStep {
        if let Some(reason) = self.check_end_execution(ctx) {
            return JobStep::Completed(reason);
        }
        if no_exec() {
            return JobStep::Completed(EndExecutionReason::Ok);
        }
        let background_next_job = std::mem::take(&mut self.background_next_job);
        if self.job_is_simple_block(job_node) && !background_next_job && job_node.bg.is_none() {
            let statement = node_ref_of(job_node, &job_node.statement);
            return JobStep::Frame(Box::new(Frame::SimpleBlockJob(Box::new(
                SimpleBlockJobFrame {
                    job_node: job_node.clone(),
                    statement,
                    associated_block,
                    started: false,
                    assignment_block: None,
                    scoped_data_before: ScopedData::default(),
                    node_before: None,
                    timer: None,
                    profile_item: None,
                },
            ))));
        }
        self.run_job_leaf(ctx, job_node, associated_block, background_next_job)
    }

    /// Advance the frame that evaluates a simple block job.
    fn step_simple_block_job(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut SimpleBlockJobFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            return self.finish_simple_block_job(ctx, frame, reason);
        }
        if frame.started {
            return FrameAction::Done(EndExecutionReason::Ok);
        }
        frame.started = true;

        frame.scoped_data_before = ctx.parser().scope();
        let mut scope = frame.scoped_data_before;
        scope.eval_level += 1;
        ctx.parser().set_scope(scope);
        frame.node_before = ctx
            .parser()
            .current_node()
            .replace(Some(frame.job_node.clone()));

        if let Some(item_id) = ctx.parser().create_profile_item() {
            frame.profile_item = Some(ProfileItemFinish {
                item_id,
                start_time: ProfileItem::now(),
                level: ctx.parser().scope().eval_level,
                skipped: false,
                command: profiling_cmd_name_for_redirectable_block(&frame.statement, self.pstree()),
            });
        }
        if job_node_wants_timing(&frame.job_node) {
            frame.timer = Some(push_timer());
        }

        let mut block = None;
        let result =
            self.apply_variable_assignments(ctx, None, &frame.job_node.variables, &mut block);
        frame.assignment_block = block;
        if result != EndExecutionReason::Ok {
            return self.finish_simple_block_job(ctx, frame, result);
        }

        push_frame(Frame::Statement(StatementFrame {
            statement: frame.statement.clone(),
            associated_block: frame.associated_block,
            started: false,
        }))
    }

    /// Finish a simple block job after its statement completes.
    fn finish_simple_block_job(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut SimpleBlockJobFrame,
        reason: EndExecutionReason,
    ) -> FrameAction {
        if let Some(profile_item) = frame.profile_item.take() {
            finish_profile_item(ctx.parser(), profile_item);
        }
        frame.timer.take();
        let node_before = frame.node_before.take();
        let _ = ctx.parser().current_node().replace(node_before);
        ctx.parser().set_scope(frame.scoped_data_before);
        if let Some(block) = frame.assignment_block.take() {
            ctx.parser().pop_block(block);
        }
        FrameAction::Done(reason)
    }

    /// Advance a statement frame.
    fn step_statement(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut StatementFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            return FrameAction::Done(reason);
        }
        if frame.started {
            return FrameAction::Done(EndExecutionReason::Ok);
        }
        frame.started = true;

        let statement: &ast::Statement = &frame.statement;
        match statement {
            Statement::Brace(brace_statement) => {
                trace_if_enabled(ctx.parser(), L!("begin"));
                let block_id = ctx
                    .parser()
                    .push_block(Block::scope_block(BlockType::Begin));
                let contents = node_ref_of(&frame.statement, &brace_statement.jobs);
                push_frame(Frame::Begin(BeginFrame {
                    contents,
                    block_id,
                    started: false,
                }))
            }
            Statement::Block(block_statement) => match &block_statement.header {
                BlockStatementHeader::For(header) => {
                    let header = node_ref_of(&frame.statement, header);
                    let contents = node_ref_of(&frame.statement, &block_statement.jobs);
                    push_frame(Frame::For(ForFrame {
                        header,
                        contents,
                        var_name: None,
                        arguments: Vec::new(),
                        event: None,
                        next_value: 0,
                        body_block: None,
                        started: false,
                        result: EndExecutionReason::Ok,
                    }))
                }
                BlockStatementHeader::While(header) => {
                    let header = node_ref_of(&frame.statement, header);
                    let contents = node_ref_of(&frame.statement, &block_statement.jobs);
                    push_frame(Frame::While(WhileFrame {
                        header,
                        contents,
                        associated_block: frame.associated_block,
                        started: false,
                        first_condition: true,
                        saved_status: Statuses::just(EXIT_SUCCESS),
                        condition_result: EndExecutionReason::Ok,
                        stage: WhileStage::Condition,
                        awaiting_child: false,
                        body_block: None,
                        result: EndExecutionReason::Ok,
                    }))
                }
                BlockStatementHeader::Function(header) => FrameAction::Done(
                    self.run_function_statement(ctx, block_statement.as_ref(), header),
                ),
                BlockStatementHeader::Begin(_) => {
                    trace_if_enabled(ctx.parser(), L!("begin"));
                    let block_id = ctx
                        .parser()
                        .push_block(Block::scope_block(BlockType::Begin));
                    let contents = node_ref_of(&frame.statement, &block_statement.jobs);
                    push_frame(Frame::Begin(BeginFrame {
                        contents,
                        block_id,
                        started: false,
                    }))
                }
            },
            Statement::If(if_statement) => {
                let statement = node_ref_of(&frame.statement, if_statement.as_ref());
                push_frame(Frame::If(IfFrame {
                    statement,
                    associated_block: frame.associated_block,
                    clause_index: 0,
                    stage: IfStage::Condition,
                    awaiting_child: false,
                    body_block: None,
                    result: EndExecutionReason::Ok,
                }))
            }
            Statement::Switch(switch_statement) => {
                let statement = node_ref_of(&frame.statement, switch_statement.as_ref());
                push_frame(Frame::Switch(SwitchFrame {
                    statement,
                    block_id: None,
                    searched: false,
                    result: EndExecutionReason::Ok,
                    awaiting_child: false,
                }))
            }
            Statement::Decorated(_) | Statement::Not(_) => panic!(),
        }
    }

    /// Advance a begin or brace block frame.
    fn step_begin(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut BeginFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            ctx.parser().pop_block(frame.block_id);
            trace_if_enabled(ctx.parser(), L!("end begin"));
            return FrameAction::Done(reason);
        }
        if !frame.started {
            frame.started = true;
            return push_frame(Frame::JobList(JobListFrame {
                jobs: frame.contents.clone(),
                next_conjunction: 0,
                associated_block: Some(frame.block_id),
                checked: false,
                result: EndExecutionReason::Ok,
            }));
        }
        FrameAction::Done(EndExecutionReason::Ok)
    }

    /// Advance a function call frame.
    fn step_function_call(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut FunctionCallFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            if let Some(scope_block) = frame.scope_block.take() {
                ctx.parser().pop_block(scope_block);
            }
            if let Some(function_block) = frame.function_block.take() {
                function_restore_environment(ctx.parser(), function_block);
            }
            let libdata = ctx.parser().libdata();
            let was_empty =
                reason == EndExecutionReason::Ok && libdata.exec_count == frame.prev_exec_count;
            if was_empty {
                ctx.parser().set_last_statuses(Statuses::just(EXIT_SUCCESS));
            }
            return FrameAction::Done(reason);
        }

        frame.prev_exec_count = ctx.parser().libdata().exec_count;
        frame.function_block = Some(function_prepare_environment(
            ctx.parser(),
            frame.argv.clone(),
            &frame.props,
        ));
        let scope_block = ctx.parser().push_block(Block::scope_block(BlockType::Top));
        frame.scope_block = Some(scope_block);
        let body = frame.props.func_node.child_ref(|node| &node.jobs);
        push_frame(Frame::JobList(JobListFrame {
            jobs: body,
            next_conjunction: 0,
            associated_block: Some(scope_block),
            checked: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Advance an if statement frame.
    fn step_if(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut IfFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            match frame.stage {
                IfStage::Condition => {
                    if reason == EndExecutionReason::Ok {
                        let Some(clause) = if_clause_of(&frame.statement, frame.clause_index)
                        else {
                            return FrameAction::Done(EndExecutionReason::Ok);
                        };
                        let tail = node_ref_of(&frame.statement, &clause.andor_tail);
                        frame.stage = IfStage::Andor;
                        frame.awaiting_child = true;
                        return push_frame(Frame::AndorJobList(AndorJobListFrame {
                            jobs: tail,
                            next_job: 0,
                            associated_block: frame.associated_block,
                            result: EndExecutionReason::Ok,
                        }));
                    }
                    return self.advance_if_clause(ctx, frame);
                }
                IfStage::Andor => {
                    let take_branch = reason == EndExecutionReason::Ok
                        && ctx.parser().last_status() == EXIT_SUCCESS;
                    if take_branch {
                        return self.push_if_clause_body(ctx, frame);
                    }
                    return self.advance_if_clause(ctx, frame);
                }
                IfStage::Body | IfStage::Else => {
                    frame.result = reason;
                    if let Some(block) = frame.body_block.take() {
                        ctx.parser().pop_block(block);
                    }
                    if let Some(reason) = self.check_end_execution(ctx) {
                        frame.result = reason;
                    }
                    return self.finish_if(ctx, frame);
                }
            }
        }

        if frame.clause_index == 0 {
            trace_if_enabled(ctx.parser(), L!("if"));
        }
        if let Some(reason) = self.check_end_execution(ctx) {
            frame.result = reason;
            return self.advance_if_clause(ctx, frame);
        }
        let Some(clause) = if_clause_of(&frame.statement, frame.clause_index) else {
            frame.result = EndExecutionReason::Ok;
            return self.finish_if(ctx, frame);
        };
        let condition = node_ref_of(&frame.statement, &clause.condition);
        frame.stage = IfStage::Condition;
        frame.awaiting_child = true;
        push_frame(Frame::Conjunction(ConjunctionFrame {
            conjunction: condition,
            associated_block: frame.associated_block,
            next_job: 0,
            tested_decorator: false,
            awaiting_child: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Move an if statement frame to its next clause, else clause, or completion.
    fn advance_if_clause(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut IfFrame,
    ) -> FrameAction {
        frame.clause_index += 1;
        if frame.clause_index < 1 + frame.statement.elseif_clauses.len() {
            trace_if_enabled(ctx.parser(), L!("else if"));
            let Some(clause) = if_clause_of(&frame.statement, frame.clause_index) else {
                return self.finish_if(ctx, frame);
            };
            let condition = node_ref_of(&frame.statement, &clause.condition);
            frame.stage = IfStage::Condition;
            frame.awaiting_child = true;
            return push_frame(Frame::Conjunction(ConjunctionFrame {
                conjunction: condition,
                associated_block: frame.associated_block,
                next_job: 0,
                tested_decorator: false,
                awaiting_child: false,
                result: EndExecutionReason::Ok,
            }));
        }
        let Some(else_clause) = frame.statement.else_clause.as_ref() else {
            ctx.parser()
                .set_last_statuses(Statuses::just(STATUS_CMD_OK));
            return self.finish_if(ctx, frame);
        };
        trace_if_enabled(ctx.parser(), L!("else"));
        let body = node_ref_of(&frame.statement, &else_clause.body);
        self.push_if_body(ctx, frame, body, IfStage::Else)
    }

    /// Push the body of the current if clause.
    fn push_if_clause_body(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut IfFrame,
    ) -> FrameAction {
        let Some(clause) = if_clause_of(&frame.statement, frame.clause_index) else {
            frame.result = EndExecutionReason::Ok;
            return self.finish_if(ctx, frame);
        };
        let body = node_ref_of(&frame.statement, &clause.body);
        self.push_if_body(ctx, frame, body, IfStage::Body)
    }

    /// Finish an if statement frame.
    fn finish_if(&mut self, ctx: &mut OperationContext<'_>, frame: &mut IfFrame) -> FrameAction {
        trace_if_enabled(ctx.parser(), L!("end if"));
        FrameAction::Done(frame.result)
    }

    /// Push an if body with its block.
    fn push_if_body(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut IfFrame,
        body: NodeRef<ast::JobList>,
        stage: IfStage,
    ) -> FrameAction {
        let block_id = ctx.parser().push_block(Block::if_block());
        frame.body_block = Some(block_id);
        frame.stage = stage;
        frame.awaiting_child = true;
        push_frame(Frame::JobList(JobListFrame {
            jobs: body,
            next_conjunction: 0,
            associated_block: Some(block_id),
            checked: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Advance a while statement frame.
    fn step_while(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut WhileFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            match frame.stage {
                WhileStage::Condition => {
                    frame.condition_result = reason;
                    if reason != EndExecutionReason::Ok {
                        return self.finish_while(ctx, frame);
                    }
                    let tail = node_ref_of(&frame.header, &frame.header.andor_tail);
                    frame.stage = WhileStage::Andor;
                    frame.awaiting_child = true;
                    return push_frame(Frame::AndorJobList(AndorJobListFrame {
                        jobs: tail,
                        next_job: 0,
                        associated_block: frame.associated_block,
                        result: EndExecutionReason::Ok,
                    }));
                }
                WhileStage::Andor => {
                    frame.condition_result = reason;
                    if frame.condition_result != EndExecutionReason::Ok {
                        return self.finish_while(ctx, frame);
                    }
                    if ctx.parser().last_status() != EXIT_SUCCESS {
                        ctx.parser().set_last_statuses(frame.saved_status.clone());
                        return self.finish_while(ctx, frame);
                    }
                    if let Some(reason) = self.check_end_execution(ctx) {
                        frame.result = reason;
                        return self.finish_while(ctx, frame);
                    }
                    ctx.parser().libdata_mut().loop_status = LoopStatus::Normals;
                    let block_id = ctx.parser().push_block(Block::while_block());
                    frame.body_block = Some(block_id);
                    frame.stage = WhileStage::Body;
                    return push_frame(Frame::JobList(JobListFrame {
                        jobs: frame.contents.clone(),
                        next_conjunction: 0,
                        associated_block: Some(block_id),
                        checked: false,
                        result: EndExecutionReason::Ok,
                    }));
                }
                WhileStage::Body => {
                    let cancel_reason = self.check_end_execution(ctx);
                    if let Some(block) = frame.body_block.take() {
                        ctx.parser().pop_block(block);
                    }
                    if cancel_reason == Some(EndExecutionReason::ControlFlow) {
                        let do_break = ctx.parser().libdata().loop_status == LoopStatus::Breaks;
                        ctx.parser().libdata_mut().loop_status = LoopStatus::Normals;
                        if do_break {
                            return self.finish_while(ctx, frame);
                        }
                    } else if no_exec() {
                        return self.finish_while(ctx, frame);
                    }
                    return self.start_while_condition(ctx, frame);
                }
            }
        }
        if !frame.started {
            frame.started = true;
            trace_if_enabled(ctx.parser(), L!("while"));
        }
        self.start_while_condition(ctx, frame)
    }

    /// Push the next while condition.
    fn start_while_condition(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut WhileFrame,
    ) -> FrameAction {
        frame.saved_status = if frame.first_condition {
            Statuses::just(EXIT_SUCCESS)
        } else {
            ctx.parser().last_statuses()
        };
        frame.first_condition = false;
        let condition = node_ref_of(&frame.header, &frame.header.condition);
        frame.stage = WhileStage::Condition;
        frame.awaiting_child = true;
        push_frame(Frame::Conjunction(ConjunctionFrame {
            conjunction: condition,
            associated_block: frame.associated_block,
            next_job: 0,
            tested_decorator: false,
            awaiting_child: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Finish a while statement frame.
    fn finish_while(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut WhileFrame,
    ) -> FrameAction {
        trace_if_enabled(ctx.parser(), L!("end while"));
        FrameAction::Done(frame.result)
    }

    /// Advance a for statement frame.
    fn step_for(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut ForFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(_reason) = child_result {
            if let Some(block) = frame.body_block.take() {
                ctx.parser().pop_block(block);
            }
            if self.check_end_execution(ctx) == Some(EndExecutionReason::ControlFlow) {
                let do_break = ctx.parser().libdata().loop_status == LoopStatus::Breaks;
                ctx.parser().libdata_mut().loop_status = LoopStatus::Normals;
                if do_break {
                    trace_if_enabled(ctx.parser(), L!("end for"));
                    return FrameAction::Done(frame.result);
                }
            }
            frame.next_value += 1;
            return self.start_for_iteration(ctx, frame);
        }
        if !frame.started {
            frame.started = true;
            return self.start_for(ctx, frame);
        }
        self.start_for_iteration(ctx, frame)
    }

    /// Expand a for loop's variable name and values, then start its first iteration.
    fn start_for(&mut self, ctx: &mut OperationContext<'_>, frame: &mut ForFrame) -> FrameAction {
        let mut var_name = self.node_source_owned(&frame.header.var_name);
        if !expand_one(&mut var_name, ExpandFlags::default(), ctx, None) {
            return FrameAction::Done(report_error!(
                self,
                ctx,
                STATUS_EXPAND_ERROR,
                &frame.header.var_name,
                FAILED_EXPANSION_VARIABLE_NAME_ERR_MSG,
                var_name
            ));
        }
        if !valid_var_name(&var_name) {
            return FrameAction::Done(report_error!(
                self,
                ctx,
                STATUS_INVALID_ARGS,
                frame.header.var_name,
                "%s",
                &varname_error(L!("for"), &var_name).to_string()
            ));
        }
        let mut arguments = vec![];
        let arg_nodes = Self::get_argument_nodes(&frame.header.args);
        let result = self.expand_arguments_from_nodes(
            ctx,
            &arg_nodes,
            &mut arguments,
            WildcardNoMatchBehavior::Allow,
        );
        if result != EndExecutionReason::Ok {
            return FrameAction::Done(result);
        }
        if EnvVar::flags_for(&var_name).read_only {
            return FrameAction::Done(report_error!(
                self,
                ctx,
                STATUS_INVALID_ARGS,
                frame.header.var_name,
                "%s: %s: cannot overwrite read-only variable",
                "for",
                var_name
            ));
        }
        let existing = ctx.parser().vars().get(&var_name);
        let set_result = ctx.parser().set_var(
            &var_name,
            ParserEnvSetMode::user(EnvMode::LOCAL),
            existing.map_or(vec![], |var| var.as_list().to_owned()),
        );
        assert_eq!(set_result, EnvStackSetResult::Ok);
        trace_if_enabled_with_args(ctx.parser(), L!("for"), &arguments);
        frame.event = Some(Event::variable_set(var_name.clone()));
        frame.var_name = Some(var_name);
        frame.arguments = arguments;
        self.start_for_iteration(ctx, frame)
    }

    /// Start one for loop iteration.
    fn start_for_iteration(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut ForFrame,
    ) -> FrameAction {
        let Some(var_name) = frame.var_name.clone() else {
            return FrameAction::Done(EndExecutionReason::Ok);
        };
        if frame.next_value >= frame.arguments.len() {
            trace_if_enabled(ctx.parser(), L!("end for"));
            return FrameAction::Done(frame.result);
        }
        if let Some(reason) = self.check_end_execution(ctx) {
            return FrameAction::Done(reason);
        }
        let value = frame.arguments[frame.next_value].clone();
        let set_result = ctx.parser().set_var(
            &var_name,
            ParserEnvSetMode::user(EnvMode::default()),
            vec![value],
        );
        assert_eq!(set_result, EnvStackSetResult::Ok);
        if let Some(event) = frame.event.as_ref() {
            event::fire(ctx.parser(), event.clone());
        }
        ctx.parser().libdata_mut().loop_status = LoopStatus::Normals;
        let block_id = ctx.parser().push_block(Block::for_block());
        frame.body_block = Some(block_id);
        push_frame(Frame::JobList(JobListFrame {
            jobs: frame.contents.clone(),
            next_conjunction: 0,
            associated_block: Some(block_id),
            checked: false,
            result: EndExecutionReason::Ok,
        }))
    }

    /// Advance a switch statement frame.
    fn step_switch(
        &mut self,
        ctx: &mut OperationContext<'_>,
        frame: &mut SwitchFrame,
        child_result: Option<EndExecutionReason>,
    ) -> FrameAction {
        if let Some(reason) = child_result {
            if let Some(block) = frame.block_id.take() {
                ctx.parser().pop_block(block);
            }
            trace_if_enabled(ctx.parser(), L!("end switch"));
            return FrameAction::Done(reason);
        }
        if frame.searched {
            return FrameAction::Done(frame.result);
        }
        frame.searched = true;

        // Expand the switch argument.
        let switch_value = self.node_source_owned(&frame.statement.argument);
        let mut switch_values_expanded = vec![];
        let mut errors = ParseErrorList::new();
        let expand_ret = expand_string(
            switch_value,
            &mut switch_values_expanded,
            ExpandFlags::default(),
            ctx,
            Some(&mut errors),
        );
        parse_error_offset_source_start(
            &mut errors,
            frame.statement.argument.range().unwrap().start(),
        );
        match expand_ret.result {
            ExpandResultCode::Error | ExpandResultCode::Overflow => {
                return FrameAction::Done(self.report_errors(ctx, expand_ret.status, &errors));
            }
            ExpandResultCode::Cancel => {
                return FrameAction::Done(EndExecutionReason::Cancelled);
            }
            ExpandResultCode::WildcardNoMatch => {
                return FrameAction::Done(
                    self.report_wildcard_error(ctx, &frame.statement.argument),
                );
            }
            ExpandResultCode::Ok => {
                if switch_values_expanded.len() > 1 {
                    let mut error = wgettext_fmt!(
                        "switch: Expected at most one argument, got %u",
                        switch_values_expanded.len()
                    );
                    error.push('\n');
                    return FrameAction::Done(report_error_formatted!(
                        self,
                        ctx,
                        STATUS_INVALID_ARGS,
                        &frame.statement.argument,
                        error
                    ));
                }
            }
        }
        let switch_value_expanded = if switch_values_expanded.is_empty() {
            WString::new()
        } else {
            switch_values_expanded.remove(0).completion
        };

        trace_if_enabled_with_args(ctx.parser(), L!("switch"), &[&switch_value_expanded]);
        let block_id = ctx.parser().push_block(Block::switch_block());
        frame.block_id = Some(block_id);

        let mut result = EndExecutionReason::Ok;
        let mut matching_case_item = None;
        for case_item in &frame.statement.cases {
            if let Some(reason) = self.check_end_execution(ctx) {
                result = reason;
                break;
            }
            let arg_nodes = Self::get_argument_nodes(&case_item.arguments);
            let mut case_args = vec![];
            let case_result = self.expand_arguments_from_nodes(
                ctx,
                &arg_nodes,
                &mut case_args,
                WildcardNoMatchBehavior::Fail,
            );
            if case_result == EndExecutionReason::Ok {
                for arg in case_args {
                    let unescaped_arg = unescape_wildcards(&arg);
                    if wildcard_match(&switch_value_expanded, &unescaped_arg, false) {
                        matching_case_item = Some(case_item);
                        break;
                    }
                }
            }
            if matching_case_item.is_some() {
                break;
            }
        }

        if result != EndExecutionReason::Ok {
            ctx.parser().pop_block(block_id);
            trace_if_enabled(ctx.parser(), L!("end switch"));
            return FrameAction::Done(result);
        }
        let Some(case_item) = matching_case_item else {
            ctx.parser().pop_block(block_id);
            trace_if_enabled(ctx.parser(), L!("end switch"));
            return FrameAction::Done(EndExecutionReason::Ok);
        };
        let body = node_ref_of(&frame.statement, &case_item.body);
        frame.result = EndExecutionReason::Ok;
        frame.awaiting_child = true;
        push_frame(Frame::JobList(JobListFrame {
            jobs: body,
            next_conjunction: 0,
            associated_block: Some(block_id),
            checked: false,
            result: EndExecutionReason::Ok,
        }))
    }

    // Check to see if we should end execution.
    // Return the eval result to end with, or none() to continue on.
    // This will never return end_execution_reason_t::ok.
    fn check_end_execution(&self, ctx: &mut OperationContext<'_>) -> Option<EndExecutionReason> {
        // If one of our jobs ended with SIGINT, we stop execution.
        // Likewise if fish itself got a SIGINT, or if something ran exit, etc.
        if self.cancel_signal.is_some() || ctx.check_cancel() || fish_is_unwinding_for_exit() {
            return Some(EndExecutionReason::Cancelled);
        }
        let parser = ctx.parser();
        if parser.current_task_lifecycle() == TaskLifecycle::Finished {
            return Some(EndExecutionReason::Cancelled);
        }
        let ld = &parser.libdata();
        if ld.exit_current_script {
            return Some(EndExecutionReason::Cancelled);
        }
        if ld.returning {
            return Some(EndExecutionReason::ControlFlow);
        }
        if ld.loop_status != LoopStatus::Normals {
            return Some(EndExecutionReason::ControlFlow);
        }
        None
    }

    fn report_errors(
        &self,
        ctx: &mut OperationContext<'_>,
        status: c_int,
        error_list: &ParseErrorList,
    ) -> EndExecutionReason {
        if !ctx.check_cancel() {
            if error_list.is_empty() {
                flog!(error, "Error reported but no error text found.");
            }

            // Get a backtrace.
            let backtrace_and_desc = ctx.parser().get_backtrace(&self.pstree().src, error_list);

            // Print it.
            if !ctx.parser().test_only_suppress_stderr() {
                eprintf!("%s", backtrace_and_desc);
            }

            // Mark status.
            ctx.parser().set_last_statuses(Statuses::just(status));
        }
        EndExecutionReason::Error
    }

    /// Command not found support.
    fn handle_command_not_found(
        &mut self,
        ctx: &mut OperationContext<'_>,
        cmd: &wstr,
        statement: &ast::DecoratedStatement,
        err: std::io::Error,
    ) -> EndExecutionReason {
        // We couldn't find the specified command. This is a non-fatal error. We want to set the exit
        // status to 127, which is the standard number used by other shells like bash and zsh.

        if err.kind() != ErrorKind::NotFound {
            // TODO: We currently handle all errors here the same,
            // but this mainly applies to EACCES. We could also feasibly get:
            // ELOOP
            // ENAMETOOLONG
            if matches!(Errno::try_from(err), Ok(Errno::ENOTDIR)) {
                // If the original command did not include a "/", assume we found it via $PATH.
                let src = self.node_source(&statement.command);
                if !src.contains('/') {
                    return report_error!(
                        self,
                        ctx,
                        STATUS_NOT_EXECUTABLE,
                        &statement.command,
                        "Unknown command. A component of '%s' is not a directory. Check your $PATH.",
                        cmd
                    );
                } else {
                    return report_error!(
                        self,
                        ctx,
                        STATUS_NOT_EXECUTABLE,
                        &statement.command,
                        "Unknown command. A component of '%s' is not a directory.",
                        cmd
                    );
                }
            }

            return report_error!(
                self,
                ctx,
                STATUS_NOT_EXECUTABLE,
                &statement.command,
                "Unknown command. '%s' exists but is not an executable file.",
                cmd
            );
        }

        // Handle unrecognized commands with standard command not found handler that can make better
        // error messages.
        let mut event_args = vec![];
        {
            let args = Self::get_argument_nodes_no_redirs(&statement.args_or_redirs);
            let arg_result = self.expand_arguments_from_nodes(
                ctx,
                &args,
                &mut event_args,
                WildcardNoMatchBehavior::Fail,
            );
            if arg_result != EndExecutionReason::Ok {
                return arg_result;
            }

            event_args.insert(0, cmd.to_owned());
        }

        let mut error = WString::new();

        // Redirect to stderr
        let mut io = IoChain::new();
        let mut list = RedirectionSpecList::new();
        list.push(RedirectionSpec::new(
            STDOUT_FILENO,
            RedirectionMode::Fd,
            L!("2").to_owned(),
        ));
        io.append_from_specs(&list, L!(""));

        let function_name = L!("fish_command_not_found");
        if function::exists(function_name, ctx.parser()) {
            let mut buffer = function_name.to_owned();
            for arg in &event_args {
                buffer.push(' ');
                buffer.push_utfstr(&escape(arg));
            }
            let parser = ctx.parser();
            let prev_statuses = parser.last_statuses();

            let event = Event::generic(function_name.to_owned());
            let b = parser.push_block(Block::event_block(event));
            parser.eval(&buffer, &io);
            parser.pop_block(b);
            parser.set_last_statuses(prev_statuses);
        } else {
            // If we have no handler, just print it as a normal error.
            error = wgettext!("Unknown command:").to_owned();
            if !event_args.is_empty() {
                error.push(' ');
                error.push_utfstr(&escape(&event_args[0]));
            }
        }

        // Here we want to report an error (so it shows a backtrace).
        // If the handler printed text, that's already shown, so error will be empty.
        report_error_formatted!(self, ctx, STATUS_CMD_UNKNOWN, &statement.command, error)
    }

    // Utilities.
    // Return the source for a node.
    fn node_source<'b>(&'b self, node: &dyn ast::Node) -> &'b wstr {
        node.source(&self.pstree().src)
    }

    // Return the source for a node as an owning WString.
    fn node_source_owned(&self, node: &dyn ast::Node) -> WString {
        self.node_source(node).to_owned()
    }

    fn infinite_recursive_statement_in_job_list<'a>(
        &self,
        ctx: &mut OperationContext<'_>,
        jobs: &'a ast::JobList,
        out_func_name: &mut WString,
    ) -> Option<&'a ast::DecoratedStatement> {
        // This is a bit fragile. It is a test to see if we are inside of function call, but
        // not inside a block in that function call. If, in the future, the rules for what
        // block scopes are pushed on function invocation changes, then this check will break.
        fn parent_fn_name<'a, 'ctx>(ctx: &'ctx mut OperationContext<'a>) -> Option<&'ctx wstr> {
            let parser = ctx.parser();
            match (parser.block_at_index(0), parser.block_at_index(1)) {
                (Some(current), Some(p)) if current.typ() == BlockType::Top => match p.data() {
                    Some(BlockData::Function { name, .. }) => Some(name),
                    _ => None,
                },
                _ => None, // Not within function call.
            }
        }

        // Get the function name of the immediate block.
        let forbidden_function_name = parent_fn_name;

        // Get the first job in the job list.
        let jc = &jobs.first()?;
        let job = &jc.job;

        // Helper to return if a statement is infinitely recursive in this function.
        let statement_recurses = |ctx: &mut OperationContext<'_>,
                                  stat: &'a ast::Statement|
         -> Option<Option<&'a ast::DecoratedStatement>> {
            // Ignore non-decorated statements like `if`, etc.
            let Statement::Decorated(dc) = &stat else {
                return Some(None);
            };

            // Ignore statements with decorations like 'builtin' or 'command', since those
            // are not infinite recursion. In particular that is what enables 'wrapper functions'.
            if dc.decoration() != StatementDecoration::None {
                return Some(None);
            }

            // Check the command.
            let mut cmd = self.node_source_owned(&dc.command);
            let forbidden = !cmd.is_empty()
                && expand_one(
                    &mut cmd,
                    ExpandFlags {
                        cmdsubst: expand::CmdsubstMode::Fail,
                        skip_variables: true,
                        ..Default::default()
                    },
                    ctx,
                    None,
                )
                && cmd == forbidden_function_name(ctx)?;
            if forbidden {
                Some(Some(dc))
            } else {
                Some(None)
            }
        };

        // Check main statement.
        let infinite_recursive_statement = statement_recurses(ctx, &jc.job.statement)?
            // Check piped remainder.
            .or_else(|| {
                for c in &job.continuation {
                    let s = statement_recurses(ctx, &c.statement)?;
                    if s.is_some() {
                        return s;
                    }
                }
                None
            });

        if infinite_recursive_statement.is_some() {
            forbidden_function_name(ctx)?.clone_into(out_func_name);
        }

        // may be none
        infinite_recursive_statement
    }

    fn report_wildcard_error(
        &self,
        ctx: &mut OperationContext<'_>,
        node: &dyn ast::Node,
    ) -> EndExecutionReason {
        report_error!(
            self,
            ctx,
            STATUS_UNMATCHED_WILDCARD,
            node,
            crate::parse_constants::WILDCARD_ERR_MSG,
            self.node_source(node),
            help_section!("language#wildcards-globbing")
        )
    }

    // Expand a command which may contain variables, producing an expand command and possibly
    // arguments. Prints an error message on error.
    fn expand_command(
        &mut self,
        ctx: &mut OperationContext<'_>,
        statement: &ast::DecoratedStatement,
        out_cmd: &mut WString,
        out_args: &mut Vec<WString>,
    ) -> EndExecutionReason {
        // Here we're expanding a command, for example $HOME/bin/stuff or $randomthing. The first
        // completion becomes the command itself, everything after becomes arguments. Command
        // substitutions with the `$()` form can produce the command and its arguments.
        let mut errors = ParseErrorList::new();

        // Get the unexpanded command string. We expect to always get it here.
        let unexp_cmd = self.node_source(&statement.command);
        let pos_of_command_token = statement.command.range().unwrap().start();

        // Expand the string to produce completions, and report errors.
        let expand_err = expand_to_command_and_args(
            unexp_cmd,
            ctx,
            out_cmd,
            Some(out_args),
            Some(&mut errors),
            /*skip_cmdsubs=*/ false,
            /*skip_wildcards=*/ false,
        );
        match expand_err.result {
            ExpandResultCode::Error | ExpandResultCode::Overflow => {
                // Issue #5812 - the expansions were done on the command token,
                // excluding prefixes such as " " or "if ".
                // This means that the error positions are relative to the beginning
                // of the token; we need to make them relative to the original source.
                parse_error_offset_source_start(&mut errors, pos_of_command_token);
                return self.report_errors(ctx, STATUS_ILLEGAL_CMD, &errors);
            }
            ExpandResultCode::WildcardNoMatch => {
                return self.report_wildcard_error(ctx, statement);
            }
            ExpandResultCode::Cancel => {
                return EndExecutionReason::Cancelled;
            }
            ExpandResultCode::Ok => {}
        }

        // Complain if the resulting expansion was empty, or expanded to an empty string.
        // For no-exec it's okay, as we can't really perform the expansion.
        if out_cmd.is_empty() && !no_exec() {
            return report_error!(
                self,
                ctx,
                STATUS_ILLEGAL_CMD,
                &statement.command,
                "The expanded command was empty."
            );
        }
        // Complain if we've expanded to a subcommand keyword like "command" or "if",
        // because these will call the corresponding *builtin*,
        // which won't be doing what the user asks for
        //
        // (skipping in no-exec because we don't have the actual variable value)
        if !no_exec()
            && &unescape_keyword(TokenType::String, unexp_cmd) != out_cmd
            && parser_keywords_is_subcommand(out_cmd)
        {
            return report_error!(
                self,
                ctx,
                STATUS_ILLEGAL_CMD,
                &statement.command,
                "The expanded command is a keyword."
            );
        }
        EndExecutionReason::Ok
    }

    /// Indicates whether a job is a simple block (one block, no redirections).
    fn job_is_simple_block(&self, job: &ast::JobPipeline) -> bool {
        // Must be no pipes.
        if !job.continuation.is_empty() {
            return false;
        }

        // Helper to check if an argument_or_redirection_list_t has no redirections.
        let no_redirs =
            |list: &ast::ArgumentOrRedirectionList| !list.iter().any(|val| val.is_redirection());

        // Check if we're a block statement with redirections.
        match &job.statement {
            Statement::Block(stmt) => no_redirs(&stmt.args_or_redirs),
            Statement::Brace(stmt) => no_redirs(&stmt.args_or_redirs),
            Statement::Switch(stmt) => no_redirs(&stmt.args_or_redirs),
            Statement::If(stmt) => no_redirs(&stmt.args_or_redirs),
            Statement::Not(_) | Statement::Decorated(_) => {
                // not block statements
                false
            }
        }
    }

    fn resolve_function(cmd: &wstr, parser: &mut Parser) -> Option<ProcessType> {
        if !valid_func_name(cmd) {
            return None;
        }
        let props = function::get_props_autoload(cmd, parser)?;
        Some(ProcessType::Function { props: Some(props) })
    }

    fn process_type_for_command(
        &self,
        ctx: &mut OperationContext<'_>,
        statement: &ast::DecoratedStatement,
        cmd: &wstr,
    ) -> ProcessType {
        // Determine the process type, which depends on the statement decoration (command, builtin,
        // etc).
        match statement.decoration() {
            StatementDecoration::Exec => ProcessType::Exec,
            StatementDecoration::Command => ProcessType::External,
            StatementDecoration::Builtin => ProcessType::Builtin,
            StatementDecoration::None => {
                if function::exists(cmd, ctx.parser()) {
                    ProcessType::Function { props: None }
                } else if builtin_exists(cmd) {
                    ProcessType::Builtin
                } else {
                    ProcessType::External
                }
            }
        }
    }

    fn apply_variable_assignments(
        &mut self,
        ctx: &mut OperationContext<'_>,
        mut proc: Option<&mut Process>,
        variable_assignment_list: &ast::VariableAssignmentList,
        block: &mut Option<BlockId>,
    ) -> EndExecutionReason {
        if variable_assignment_list.is_empty() {
            return EndExecutionReason::Ok;
        }
        *block = Some(ctx.parser().push_block(Block::variable_assignment_block()));
        for variable_assignment in variable_assignment_list {
            let source = self.node_source(variable_assignment);
            let equals_pos = variable_assignment_equals_pos(source).unwrap();
            let variable_name = &source[..equals_pos];
            let expression = &source[equals_pos + 1..];
            let mut expression_expanded = vec![];
            let mut errors = ParseErrorList::new();
            // TODO this is mostly copied from expand_arguments_from_nodes, maybe extract to function
            let expand_ret = expand_string(
                expression.to_owned(),
                &mut expression_expanded,
                ExpandFlags::default(),
                ctx,
                Some(&mut errors),
            );
            parse_error_offset_source_start(
                &mut errors,
                variable_assignment.range().unwrap().start() + equals_pos + 1,
            );
            match expand_ret.result {
                ExpandResultCode::Error|ExpandResultCode::Overflow => {
                    return self.report_errors(ctx, expand_ret.status, &errors);
                }
                ExpandResultCode::Cancel => {
                    return EndExecutionReason::Cancelled;
                }
                ExpandResultCode::WildcardNoMatch // nullglob (equivalent to set)
                    | ExpandResultCode::Ok => {}
            }
            let vals: Vec<_> = expression_expanded
                .into_iter()
                .map(|comp| comp.completion)
                .collect();
            if let Some(proc) = &mut proc {
                proc.variable_assignments.push(ConcreteAssignment::new(
                    variable_name.to_owned(),
                    vals.clone(),
                ));
            }
            let set_result = ctx.parser().set_var_and_fire(
                variable_name,
                ParserEnvSetMode::new(EnvMode::LOCAL_EXPORTED),
                vals,
            );
            // TODO Do we really want to create `IoStreams` here?
            let mut outs = OutputStream::Fd(FdOutputStream::new(STDOUT_FILENO));
            let mut errs = OutputStream::Fd(FdOutputStream::new(STDERR_FILENO));
            let mut streams = IoStreams::new(&mut outs, &mut errs, &self.block_io);
            handle_env_return(set_result, get_program_name(), variable_name, &mut streams);
            if set_result != EnvStackSetResult::Ok {
                return EndExecutionReason::Error;
            }
        }
        EndExecutionReason::Ok
    }

    // These create process_t structures from statements.
    fn populate_job_process(
        &mut self,
        ctx: &mut OperationContext<'_>,
        job: &mut Job,
        proc: &mut Process,
        statement: &ast::Statement,
        variable_assignments: &ast::VariableAssignmentList,
    ) -> EndExecutionReason {
        let mut block = None;
        let result =
            self.apply_variable_assignments(ctx, Some(proc), variable_assignments, &mut block);
        let ctx = &mut **ScopeGuard::new(ctx, |ctx| {
            if let Some(block) = block {
                ctx.parser().pop_block(block);
            }
        });
        if result != EndExecutionReason::Ok {
            return result;
        }

        match &statement {
            Statement::Not(not_statement) => {
                self.populate_not_process(ctx, job, proc, not_statement)
            }
            Statement::Block(_) | Statement::Brace(_) | Statement::If(_) | Statement::Switch(_) => {
                self.populate_block_process(ctx, proc, statement)
            }
            Statement::Decorated(decorated_statement) => {
                self.populate_plain_process(ctx, proc, decorated_statement)
            }
        }
    }

    fn populate_not_process(
        &mut self,
        ctx: &mut OperationContext<'_>,
        job: &mut Job,
        proc: &mut Process,
        not_statement: &ast::NotStatement,
    ) -> EndExecutionReason {
        {
            let mut flags = job.flags_mut();
            flags.negate = !flags.negate;
        }
        self.populate_job_process(
            ctx,
            job,
            proc,
            &not_statement.contents,
            &not_statement.variables,
        )
    }

    /// Creates a 'normal' (non-block) process.
    fn populate_plain_process(
        &mut self,
        ctx: &mut OperationContext<'_>,
        proc: &mut Process,
        statement: &ast::DecoratedStatement,
    ) -> EndExecutionReason {
        // We may decide that a command should be an implicit cd.
        let mut use_implicit_cd = false;

        // Get the command and any arguments due to expanding the command.
        let mut cmd = WString::new();
        let mut args_from_cmd_expansion = vec![];
        let ret = self.expand_command(ctx, statement, &mut cmd, &mut args_from_cmd_expansion);
        if ret != EndExecutionReason::Ok {
            return ret;
        }

        // For no-exec, having an empty command is okay. We can't do anything more with it tho.
        if no_exec() {
            return EndExecutionReason::Ok;
        }

        assert!(
            !cmd.is_empty(),
            "expand_command should not produce an empty command",
        );

        // Determine the process type.
        let mut process_type = self.process_type_for_command(ctx, statement, &cmd);
        let external_cmd = if matches!(process_type, ProcessType::External | ProcessType::Exec) {
            let parser = ctx.parser();
            // Determine the actual command. This may be an implicit cd.
            let external_cmd = path_try_get_path(&cmd, parser.vars());
            let has_command = external_cmd.err.is_none();

            let mut path = WString::new();
            if has_command {
                path = external_cmd.path;
            } else {
                // If the specified command does not exist, and is undecorated, try using an implicit cd.
                if statement.decoration() == StatementDecoration::None {
                    // Implicit cd requires an empty argument and redirection list.
                    if statement.args_or_redirs.is_empty() {
                        // Ok, no arguments or redirections; check to see if the command is a directory.
                        use_implicit_cd = path_as_implicit_cd(
                            &cmd,
                            &parser.vars().get_pwd_slash(),
                            parser.vars(),
                        )
                        .is_some();
                    }
                }

                if !use_implicit_cd {
                    return self.handle_command_not_found(
                        ctx,
                        if external_cmd.path.is_empty() {
                            &cmd
                        } else {
                            &external_cmd.path
                        },
                        statement,
                        std::io::Error::from(external_cmd.err.unwrap()),
                    );
                }
            }
            path
        } else {
            WString::new()
        };

        // Produce the full argument list and the set of IO redirections.
        let mut cmd_args = vec![];
        let mut redirections = RedirectionSpecList::new();
        if use_implicit_cd {
            // Implicit cd is simple.
            cmd_args = vec![L!("cd").to_owned(), L!("--").to_owned(), cmd];

            // If we have defined a wrapper around cd, use it, otherwise use the cd builtin.
            process_type = if function::exists(L!("cd"), ctx.parser()) {
                ProcessType::Function { props: None }
            } else {
                ProcessType::Builtin
            };
        } else {
            // Not implicit cd.
            let glob_behavior = if [L!("set"), L!("count"), L!("path")].contains(&&cmd[..]) {
                WildcardNoMatchBehavior::Allow
            } else {
                WildcardNoMatchBehavior::Fail
            };
            // Form the list of arguments. The command is the first argument, followed by any arguments
            // from expanding the command, followed by the argument nodes themselves. E.g. if the
            // command is '$gco foo' and $gco is git checkout.
            cmd_args.push(cmd);
            cmd_args.extend_from_slice(&args_from_cmd_expansion);
            let arg_nodes = Self::get_argument_nodes_no_redirs(&statement.args_or_redirs);
            let arg_result =
                self.expand_arguments_from_nodes(ctx, &arg_nodes, &mut cmd_args, glob_behavior);
            if arg_result != EndExecutionReason::Ok {
                return arg_result;
            }

            // The set of IO redirections that we construct for the process.
            let reason =
                self.determine_redirections(ctx, &statement.args_or_redirs, &mut redirections);
            if reason != EndExecutionReason::Ok {
                return reason;
            }
        }

        // Populate the process.
        if let ProcessType::Function { props } = &mut process_type {
            let Some(ProcessType::Function {
                props: resolved_props,
            }) = Self::resolve_function(&cmd_args[0], ctx.parser())
            else {
                flog!(error, wgettext_fmt!("Unknown function '%s'", &cmd_args[0]));
                return EndExecutionReason::Error;
            };
            *props = resolved_props;
        }
        proc.typ = process_type;
        proc.set_argv(cmd_args);
        proc.set_redirection_specs(redirections);
        proc.actual_cmd = external_cmd;
        EndExecutionReason::Ok
    }

    fn populate_block_process(
        &mut self,
        ctx: &mut OperationContext<'_>,
        proc: &mut Process,
        statement: &ast::Statement,
    ) -> EndExecutionReason {
        // We handle block statements by creating ProcessType::block_node, that will bounce back to
        // us when it's time to execute them.
        // Get the argument or redirections list.
        // TODO: args_or_redirs should be available without resolving the statement type.
        let args_or_redirs = match statement {
            Statement::Block(block_statement) => &block_statement.args_or_redirs,
            Statement::Brace(brace_statement) => &brace_statement.args_or_redirs,
            Statement::If(if_statement) => &if_statement.args_or_redirs,
            Statement::Switch(switch_statement) => &switch_statement.args_or_redirs,
            _ => panic!("Unexpected block node type"),
        };

        let mut redirections = RedirectionSpecList::new();
        let reason = self.determine_redirections(ctx, args_or_redirs, &mut redirections);
        if reason == EndExecutionReason::Ok {
            proc.typ = ProcessType::BlockNode(
                // SAFETY: `statement` must be a descendant of the `eval_node()`'s node, which is owned by our parsed source.
                unsafe { NodeRef::new(Arc::clone(self.pstree()), statement) },
            );
            proc.set_redirection_specs(redirections);
        }
        reason
    }

    // Define a function.
    fn run_function_statement(
        &mut self,
        ctx: &mut OperationContext<'_>,
        statement: &ast::BlockStatement,
        header: &ast::FunctionHeader,
    ) -> EndExecutionReason {
        // Get arguments.
        let mut arguments = vec![];
        let mut arg_nodes = Self::get_argument_nodes(&header.args);
        arg_nodes.insert(0, &header.first_arg);
        let result = self.expand_arguments_from_nodes(
            ctx,
            &arg_nodes,
            &mut arguments,
            WildcardNoMatchBehavior::Fail,
        );

        if result != EndExecutionReason::Ok {
            return result;
        }

        trace_if_enabled_with_args(ctx.parser(), L!("function"), &arguments);
        let mut outs = OutputStream::Null;
        let mut errs = OutputStream::String(StringOutputStream::new());
        let io_chain = IoChain::new();
        let mut streams = IoStreams::new(&mut outs, &mut errs, &io_chain);
        let mut shim_arguments: Vec<&wstr> = arguments
            .iter()
            .map(|s| truncate_at_nul(s.as_ref()))
            .collect();
        let err_code = builtins::function::function(
            ctx.parser(),
            &mut streams,
            &mut shim_arguments,
            // SAFETY: `statement` must be a descendant of the `eval_node()`'s node, which is owned by our parsed source.
            unsafe { NodeRef::new(Arc::clone(self.pstree()), statement) },
        )
        .err()
        .unwrap_or(STATUS_CMD_OK);

        ctx.parser().libdata_mut().status_count += 1;
        ctx.parser().set_last_statuses(Statuses::just(err_code));

        let errtext = errs.contents();
        if !errtext.is_empty() {
            report_error!(self, ctx, err_code, header, "%s", errtext);
        }
        result
    }

    fn get_argument_nodes(args: &ast::ArgumentList) -> AstArgsList<'_> {
        let mut result = AstArgsList::new();
        for arg in args {
            result.push(arg);
        }
        result
    }

    fn get_argument_nodes_no_redirs(args: &ast::ArgumentOrRedirectionList) -> AstArgsList<'_> {
        let mut result = AstArgsList::new();
        for arg in args {
            if arg.is_argument() {
                result.push(arg.argument());
            }
        }
        result
    }

    fn expand_arguments_from_nodes(
        &mut self,
        ctx: &mut OperationContext<'_>,
        argument_nodes: &AstArgsList<'_>,
        out_arguments: &mut Vec<WString>,
        glob_behavior: WildcardNoMatchBehavior,
    ) -> EndExecutionReason {
        // Get all argument nodes underneath the statement. We guess we'll have that many arguments (but
        // may have more or fewer, if there are wildcards involved).
        out_arguments.reserve(argument_nodes.len());
        for &arg_node in argument_nodes {
            // Expect all arguments to have source.
            assert!(arg_node.has_source(), "Argument should have source");

            // Expand this string.
            let mut errors = ParseErrorList::new();
            let mut arg_expanded = CompletionList::new();
            let expand_ret = expand_string(
                self.node_source_owned(arg_node),
                &mut arg_expanded,
                ExpandFlags::default(),
                ctx,
                Some(&mut errors),
            );
            parse_error_offset_source_start(&mut errors, arg_node.range().unwrap().start());
            match expand_ret.result {
                ExpandResultCode::Error | ExpandResultCode::Overflow => {
                    return self.report_errors(ctx, expand_ret.status, &errors);
                }
                ExpandResultCode::Cancel => {
                    return EndExecutionReason::Cancelled;
                }
                ExpandResultCode::WildcardNoMatch => {
                    if glob_behavior == WildcardNoMatchBehavior::Fail {
                        // For no_exec, ignore the error - this might work at runtime.
                        if no_exec() {
                            return EndExecutionReason::Ok;
                        }
                        // Report the unmatched wildcard error and stop processing.
                        return self.report_wildcard_error(ctx, arg_node);
                    }
                }
                ExpandResultCode::Ok => {}
            }

            // Now copy over any expanded arguments.
            if let Some(additional) =
                (out_arguments.len() + arg_expanded.len()).checked_sub(out_arguments.capacity())
            {
                out_arguments.reserve(additional);
            }
            for new_arg in arg_expanded {
                out_arguments.push(new_arg.completion);
            }
        }

        // We may have received a cancellation during this expansion.
        if let Some(ret) = self.check_end_execution(ctx) {
            return ret;
        }

        EndExecutionReason::Ok
    }

    // Determines the list of redirections for a node.
    fn determine_redirections(
        &self,
        ctx: &mut OperationContext<'_>,
        list: &ast::ArgumentOrRedirectionList,
        out_redirections: &mut RedirectionSpecList,
    ) -> EndExecutionReason {
        // Get all redirection nodes underneath the statement.
        for arg_or_redir in list {
            if !arg_or_redir.is_redirection() {
                continue;
            }
            let redir_node = arg_or_redir.redirection();

            let oper = match PipeOrRedir::try_from(self.node_source(&redir_node.oper)) {
                Ok(oper) if oper.is_valid() => oper,
                _ => {
                    // TODO: figure out if this can ever happen. If so, improve this error message.
                    return report_error!(
                        self,
                        ctx,
                        STATUS_INVALID_ARGS,
                        redir_node,
                        "Invalid redirection: %s",
                        self.node_source(redir_node)
                    );
                }
            };

            // PCA: I can't justify this skip_variables flag. It was like this when I got here.
            let mut target = self.node_source_owned(&redir_node.target);
            let target_expanded = expand_one(
                &mut target,
                if no_exec() {
                    ExpandFlags {
                        skip_variables: true,
                        ..Default::default()
                    }
                } else {
                    ExpandFlags::default()
                },
                ctx,
                None,
            );

            if !target_expanded || target.is_empty() {
                // TODO: Improve this error message.
                let error_ret = report_error!(
                    self,
                    ctx,
                    STATUS_INVALID_ARGS,
                    redir_node,
                    "Invalid redirection target: %s",
                    target
                );
                if oper.mode == RedirectionMode::Input && {
                    let redir_unexpanded = self.node_source(redir_node);
                    redir_unexpanded.starts_with(L!("<("))
                        && match locate_cmdsubst_range(
                            &redir_unexpanded[1..],
                            &mut 0,
                            false,
                            None,
                            None,
                        ) {
                            Ok(Some(cmdsub)) => cmdsub.opening_paren_offset() == 0,
                            _ => false,
                        }
                } {
                    eprintf!(
                        "If you wish to use process substitution, consider the psub command, see: `help %s`\n",
                        help_section!("cmds/psub")
                    );
                }
                return error_ret;
            }

            // Make a redirection spec from the redirect token.
            assert!(oper.is_valid(), "expected to have a valid redirection");
            let spec = RedirectionSpec::new(oper.fd, oper.mode, target);

            // Validate this spec.
            if spec.mode == RedirectionMode::Fd
                && !spec.is_close()
                && spec.get_target_as_fd().is_none()
            {
                return report_error!(
                    self,
                    ctx,
                    STATUS_INVALID_ARGS,
                    redir_node,
                    "Requested redirection to '%s', which is not a valid file descriptor",
                    &spec.target
                );
            }
            out_redirections.push(spec);

            if oper.stderr_merge {
                // This was a redirect like &> which also modifies stderr.
                // Also redirect stderr to stdout.
                out_redirections.push(get_stderr_merge());
            }
        }
        EndExecutionReason::Ok
    }

    fn run_job_leaf(
        &mut self,
        ctx: &mut OperationContext<'_>,
        job_node: &ast::JobPipeline,
        associated_block: Option<BlockId>,
        background_next_job: bool,
    ) -> JobStep {
        if let Some(ret) = self.check_end_execution(ctx) {
            return JobStep::Completed(ret);
        }

        // We definitely do not want to execute anything if we're told we're --no-execute!
        if no_exec() {
            return JobStep::Completed(EndExecutionReason::Ok);
        }

        // Increment the eval_level for the duration of this command.
        let _saved_eval_level = ctx.parser().push_scope(|s| s.eval_level += 1);

        // Save the executing node.
        // SAFETY: `job_node` must be a descendant of the `eval_node()`'s node, which is owned by our parsed source.
        let executing_node = unsafe { NodeRef::new(Arc::clone(self.pstree()), job_node) };
        let _saved_node = ctx
            .parser()
            .current_node()
            .scoped_replace(Some(executing_node));

        // Profiling support.
        let profile_item = {
            let profile_item_id = ctx.parser().create_profile_item();
            profile_item_id.map(|item_id| ProfileItemFinish {
                item_id,
                start_time: ProfileItem::now(),
                level: ctx.parser().scope().eval_level,
                skipped: false,
                command: WString::new(),
            })
        };

        let job_is_background = job_node.bg.is_some() || background_next_job;
        let _timer = {
            let wants_timing = job_node_wants_timing(job_node);
            // It's an error to have 'time' in a background job.
            if wants_timing && job_is_background {
                return JobStep::Completed(report_error!(
                    self,
                    ctx,
                    STATUS_INVALID_ARGS,
                    job_node,
                    ERROR_TIME_BACKGROUND
                ));
            }
            wants_timing.then(push_timer)
        };

        let mut props = JobProperties {
            initial_background: job_is_background,
            ..Default::default()
        };
        {
            let parser = ctx.parser();
            let sc = parser.scope();
            props.skip_notification =
                sc.is_subshell || parser.is_block() || sc.is_event || !parser.is_interactive();
            props.from_event_handler = sc.is_event;
        }

        let mut job = Job::new(props, self.node_source_owned(job_node));
        job.set_parsed_source(self.pstree().clone());

        // We are about to populate a job. One possible argument to the job is a command substitution
        // which may be interested in the job that's populating it, via '--on-job-exit caller'. Record
        // the job ID here.
        let _caller_id = ctx
            .parser()
            .push_scope(move |s| s.caller_id = job.internal_job_id);

        // Populate the job. This may fail for reasons like command_not_found. If this fails, an error
        // will have been printed.
        let pop_result = self.populate_job_from_job_node(ctx, &mut job, job_node, associated_block);
        drop(_caller_id);

        // Clean up the job on failure or cancellation.
        if pop_result == EndExecutionReason::Ok {
            self.setup_group(ctx, &mut job);
            assert!(job.group.is_some(), "Should have a group");
        }

        // Now that we're done mutating the Job, we can stick it in an Arc
        let job = Rc::new(job);

        if pop_result != EndExecutionReason::Ok {
            if let Some(mut profile_item) = profile_item {
                profile_item.command = job.command().to_owned();
                profile_item.skipped = true;
                finish_profile_item(ctx.parser(), profile_item);
            }
            return JobStep::Completed(pop_result);
        }

        let job_block_io = if job_is_background {
            crate::exec::background_job_io(&self.block_io)
        } else {
            self.block_io.clone()
        };
        let frame = JobFrame {
            job: Rc::clone(&job),
            node: unsafe { NodeRef::new(Arc::clone(self.pstree()), job_node) },
            block_io: job_block_io,
            parsed_source: self.pstree().clone(),
            profile_item,
            suspended_at: None,
            handoff: crate::tty_handoff::TtyHandoff::new(|| {}),
            stage_launch_index: 0,
            processes_launched: 0,
            pipe_read_for_next_stage: None,
            deferred_pipes: PartialPipes::default(),
            deferred_process: None,
            deferred_stage_launched: false,
            active_stage_is_deferred: false,
            active_process_index: None,
            pipeline_aborted: false,
            suspension_count: 0,
            end_reason: None,
            phase: JobPhase::Launch,
            started: false,
        };
        ctx.parser().job_add(Rc::clone(&job));
        JobStep::Frame(Box::new(Frame::Job(Box::new(frame))))
    }

    fn populate_job_from_job_node(
        &mut self,
        ctx: &mut OperationContext<'_>,
        j: &mut Job,
        job_node: &ast::JobPipeline,
        _associated_block: Option<BlockId>,
    ) -> EndExecutionReason {
        // We are going to construct Process structures for every statement in the job.
        // Create processes. Each one may fail. We know how many there are.
        let mut processes: Vec<Process> = Vec::new();
        processes.reserve_exact(1 + job_node.continuation.len());
        processes.push(Process::new());
        let mut result = self.populate_job_process(
            ctx,
            j,
            &mut processes[0],
            &job_node.statement,
            &job_node.variables,
        );

        // Construct Processes for job continuations (pipelines).
        for jc in &job_node.continuation {
            if result != EndExecutionReason::Ok {
                break;
            }
            // Handle the pipe, whose fd may not be the obvious stdout.
            let parsed_pipe = PipeOrRedir::try_from(self.node_source(&jc.pipe))
                .expect("Failed to parse valid pipe");
            if !parsed_pipe.is_valid() {
                result = report_error!(
                    self,
                    ctx,
                    STATUS_INVALID_ARGS,
                    &jc.pipe,
                    ILLEGAL_FD_ERR_MSG,
                    self.node_source(&jc.pipe)
                );
                break;
            }
            {
                let proc = processes.last_mut().unwrap();
                proc.pipe_write_fd = parsed_pipe.fd;
                if parsed_pipe.stderr_merge {
                    // This was a pipe like &| which redirects both stdout and stderr.
                    // Also redirect stderr to stdout.
                    let specs = proc.redirection_specs_mut();
                    specs.push(get_stderr_merge());
                }
            }

            // Store the new process (and maybe with an error).
            processes.push(Process::new());
            result = self.populate_job_process(
                ctx,
                j,
                processes.last_mut().unwrap(),
                &jc.statement,
                &jc.variables,
            );
        }

        // Inform our processes of who is first and last
        processes.first_mut().unwrap().is_first_in_job = true;
        processes.last_mut().unwrap().is_last_in_job = true;

        // Return what happened.
        if result == EndExecutionReason::Ok {
            // Every shell-evaluated stage of a background job records a task.
            if j.is_initially_background() {
                for proc in processes.iter_mut() {
                    let task_stage = match &proc.typ {
                        ProcessType::Function { props: Some(props) } => Some(TaskStage::Function {
                            props: Arc::clone(props),
                        }),
                        ProcessType::BlockNode(statement) => {
                            Some(TaskStage::Block(statement.clone()))
                        }
                        ProcessType::Builtin => Some(TaskStage::Builtin),
                        _ => None,
                    };
                    if let Some(task_stage) = task_stage {
                        proc.typ = ProcessType::Task(task_stage);
                    }
                }
            }

            // Link up the processes.
            assert!(!processes.is_empty());
            *j.processes_mut() = processes.into_boxed_slice();
        }
        result
    }

    // Assign a job group to the given job.
    fn setup_group(&self, ctx: &mut OperationContext<'_>, j: &mut Job) {
        // We can use the parent group if it's compatible and we're not backgrounded.
        if ctx
            .job_group
            .as_ref()
            .is_some_and(|job_group| job_group.has_job_id() || !j.wants_job_id())
            && !j.is_initially_background()
        {
            j.group.clone_from(&ctx.job_group);
            return;
        }

        if j.processes()[0].is_internal() || !self.use_job_control(ctx) {
            // This job either doesn't have a pgroup (e.g. a simple block), or lives in fish's pgroup.
            j.group = Some(JobGroup::create(j.command().to_owned(), j.wants_job_id()));
        } else {
            // This is a "real job" that gets its own pgroup.
            j.processes_mut()[0].leads_pgrp = true;
            let wants_terminal = !ctx.parser().scope().is_event;
            j.group = Some(JobGroup::create_with_job_control(
                j.command().to_owned(),
                wants_terminal,
            ));
        }
        j.group().is_foreground.store(!j.is_initially_background());
        j.flags_mut().is_group_root = true;
    }

    // Return whether we should apply job control to our processes.
    fn use_job_control(&self, ctx: &mut OperationContext<'_>) -> bool {
        if ctx.parser().is_command_substitution() {
            return false;
        }
        match get_job_control_mode() {
            JobControl::All => true,
            JobControl::Interactive => ctx.parser().is_interactive(),
            JobControl::None => false,
        }
    }
}

#[derive(PartialEq)]
enum WildcardNoMatchBehavior {
    /// failglob
    Fail,
    /// nullglob
    Allow,
}
type AstArgsList<'a> = Vec<&'a ast::Argument>;

/// Return the clause at `index`: 0 is the `if` clause, and `i` is `elseif_clauses[i - 1]`.
fn if_clause_of(statement: &ast::IfStatement, index: usize) -> Option<&ast::IfClause> {
    if index == 0 {
        Some(&statement.if_clause)
    } else {
        statement
            .elseif_clauses
            .get(index - 1)
            .map(|clause| &clause.if_clause)
    }
}

/// Record the completion of one profile entry.
fn finish_profile_item(parser: &mut Parser, profile_item: ProfileItemFinish) {
    let item = &mut parser.profile_items_mut()[profile_item.item_id];
    item.duration = ProfileItem::now() - profile_item.start_time;
    item.level = profile_item.level;
    item.cmd = profile_item.command;
    item.skipped = profile_item.skipped;
}

/// Record a finished job's profile entry, universal variables and child reaping.
fn finish_job_frame_record(
    parser: &mut Parser,
    frame: &mut JobFrame,
    block_io: Option<&IoChain>,
) -> Option<RawSignal> {
    if let Some(mut profile_item) = frame.profile_item.take() {
        profile_item.command = frame.job.command().to_owned();
        finish_profile_item(parser, profile_item);
    }
    parser.sync_uvars_and_fire(frame.job.has_external_proc());
    job_reap(parser, false, block_io);
    frame.job.group().get_cancel_signal()
}

/// Finish a job frame whose evaluation a cancellation discarded.
pub(crate) fn finish_abandoned_job_frame(parser: &mut Parser, frame: &mut JobFrame) {
    let _ = finish_job_frame_record(parser, frame, None);
}

/// Complete the process of a shell-evaluated stage whose evaluation a cancellation discarded.
pub(crate) fn abandon_stage_frame(parser: &mut Parser, frame: &mut StageFrame, signal: RawSignal) {
    if !frame.started {
        return;
    }
    let process_index = stage_process_index(&frame.stage);
    let process = &frame.job.processes()[process_index];
    process.status.set(ProcStatus::from_signal(signal));
    run_internal_process_or_short_circuit(
        parser,
        &frame.job,
        process,
        vec![],
        vec![],
        &IoChain::new(),
    );
}

fn statement_is_redirectable_block(node: &ast::Statement) -> bool {
    match node {
        Statement::Decorated(_) | Statement::Not(_) => false,
        Statement::Block(_) | Statement::Brace(_) | Statement::If(_) | Statement::Switch(_) => true,
    }
}

/// Get the name of a redirectable block, for profiling purposes.
fn profiling_cmd_name_for_redirectable_block(
    node: &ast::Statement,
    pstree: &ParsedSourceRef,
) -> WString {
    assert!(statement_is_redirectable_block(node));

    let source_range = node.try_source_range().expect("No source range for block");

    let src_end = match node {
        Statement::Block(block_statement) => {
            let block_header = &block_statement.header;
            match block_header {
                BlockStatementHeader::For(node) => node.semi_nl.source_range().start(),
                BlockStatementHeader::While(node) => node.condition.source_range().start(),
                BlockStatementHeader::Function(node) => node.semi_nl.source_range().start(),
                BlockStatementHeader::Begin(node) => node.kw_begin.source_range().start(),
            }
        }
        Statement::Brace(brace_statement) => brace_statement.left_brace.source_range().start(),
        Statement::If(ifstmt) => ifstmt.if_clause.condition.job.source_range().end(),
        Statement::Switch(switchstmt) => switchstmt.semi_nl.source_range().start(),
        _ => {
            panic!("Not a redirectable block_type");
        }
    };

    assert!(src_end >= source_range.start(), "Invalid source end");

    // Get the source for the block, and cut it at the next statement terminator.
    let mut result = pstree.src[source_range.start()..src_end].to_owned();
    result.push_utfstr(L!("..."));
    result
}

/// Get a redirection from stderr to stdout (i.e. 2>&1).
fn get_stderr_merge() -> RedirectionSpec {
    let stdout_fileno_str = L!("1").to_owned();
    RedirectionSpec::new(STDERR_FILENO, RedirectionMode::Fd, stdout_fileno_str)
}

/// Decide if a job node should be 'time'd.
/// For historical reasons the 'not' and 'time' prefix are "inside out". That is, it's
/// 'not time cmd'. Note that a time appearing anywhere in the pipeline affects the whole job.
/// `sleep 1 | not time true` will time the whole job!
fn job_node_wants_timing(job_node: &ast::JobPipeline) -> bool {
    // Does our job have the job-level time prefix?
    if job_node.time.is_some() {
        return true;
    }

    // Helper to return true if a node is 'not time ...' or 'not not time...' or...
    fn is_timed_not_statement(mut stat: &ast::Statement) -> bool {
        loop {
            match &stat {
                Statement::Not(ns) => {
                    if ns.time.is_some() {
                        return true;
                    }
                    stat = &ns.contents;
                }
                _ => return false,
            }
        }
    }

    // Do we have a 'not time ...' anywhere in our pipeline?
    if is_timed_not_statement(&job_node.statement) {
        return true;
    }
    for jc in &job_node.continuation {
        if is_timed_not_statement(&jc.statement) {
            return true;
        }
    }

    false
}

/// Return the descriptors a pending stage runs with.
fn stage_chain(stage: &PendingStage) -> &IoChain {
    match stage {
        PendingStage::Builtin { chain, .. }
        | PendingStage::Function { chain, .. }
        | PendingStage::Block { chain, .. } => chain,
    }
}

/// Return the index of the process a pending stage records.
pub(crate) fn stage_process_index(stage: &PendingStage) -> usize {
    match stage {
        PendingStage::Builtin { process_index, .. }
        | PendingStage::Function { process_index, .. }
        | PendingStage::Block { process_index, .. } => *process_index,
    }
}

/// Record that a shell-evaluated stage completed with `status`.
fn complete_stage_process(
    parser: &mut Parser,
    job: &Job,
    process_index: usize,
    chain: &IoChain,
    status: c_int,
) {
    let process = &job.processes()[process_index];
    process.status.set(ProcStatus::from_exit_code(status));
    run_internal_process_or_short_circuit(parser, job, process, vec![], vec![], chain);
}

fn remove_job(parser: &mut Parser, job: &Rc<Job>) -> bool {
    let jobs = parser.jobs_mut();
    let num_jobs = jobs.len();
    for i in 0..num_jobs {
        if Rc::ptr_eq(&jobs[i], job) {
            jobs.remove(i);
            return true;
        }
    }
    false
}

impl Parser {
    /// Run every runnable task until no task is runnable and none becomes ready.
    pub fn run_runnable_tasks(&mut self) {
        loop {
            while let Some(task_id) = self.take_runnable_task() {
                self.step_stage_task(task_id);
            }
            self.wake_ready_tasks();
            if !self.scheduler_has_runnable_tasks() {
                return;
            }
        }
    }

    /// Advance one stage task by one step; return whether the task finished.
    pub(crate) fn step_stage_task(&mut self, task_id: TaskId) -> bool {
        if self
            .task(task_id)
            .is_some_and(|task| task.lifecycle() == TaskLifecycle::Finished)
        {
            // `exec` in the task finished its evaluation; complete the stage with the command's
            // status rather than resuming frames that were discarded.
            let status = self.last_status();
            let previous_task = self.current_task_id();
            self.finish_stage_task(task_id, Some(status), previous_task);
            return true;
        }
        let Some(stage_task) = self.task(task_id).and_then(|task| task.stage()) else {
            return false;
        };
        let source = stage_task.parsed_source.clone();
        let io_chain = stage_task.io_chain.clone();
        let job_group = stage_task.job_group.clone();
        let job_id = stage_task.job;
        let variable_assignments = stage_task.variable_assignments.clone();
        let builtin_stage = self
            .job_with_internal_id(job_id)
            .and_then(|job| {
                job.processes()
                    .iter()
                    .find(|process| process.task_id.get() == Some(&task_id))
                    .map(|process| process.evaluates_builtin())
            })
            .unwrap_or(false);
        let Some(mut frames) = self.take_task_frames(task_id) else {
            return false;
        };

        let previous_task = self.current_task_id();
        self.set_current_task(task_id);
        if let Some(task) = self.task_mut(task_id) {
            let _ = task.transition_lifecycle_to(TaskLifecycle::Running);
        }
        if self
            .task(task_id)
            .is_some_and(|task| task.assignment_block().is_none())
            && !variable_assignments.is_empty()
        {
            let assignment_block = push_assignment_block(self, &variable_assignments);
            if let Some(block) = assignment_block
                && let Some(task) = self.task_mut(task_id)
            {
                task.set_assignment_block(block);
            }
        }

        if builtin_stage {
            let job = self
                .job_with_internal_id(job_id)
                .expect("the job of a builtin stage task is recorded");
            let process = job
                .processes()
                .iter()
                .find(|process| process.task_id.get() == Some(&task_id))
                .expect("the builtin stage records its process");
            let result =
                exec_builtin_process(self, &job, process, &io_chain, !process.is_last_in_job);
            if result.is_err() {
                process.mark_aborted_before_launch();
            }
            self.finish_stage_task(task_id, None, previous_task);
            return true;
        }

        let mut execution_ctx = ExecutionContext::new(source, io_chain);
        execution_ctx.job_group = job_group;
        let outcome = {
            let mut op_ctx = self.context();
            execution_ctx.evaluate_frame_stack(&mut op_ctx, &mut frames)
        };

        match outcome {
            EvalOutcome::Blocked(reason) => {
                if let Some(task) = self.task_mut(task_id) {
                    *task.frames_mut() = frames;
                    let _ = task.transition_lifecycle_to(TaskLifecycle::Blocked);
                }
                self.set_current_task(previous_task);
                self.register_blocked(task_id, reason);
                false
            }
            EvalOutcome::Completed(_reason) => {
                let status = self.last_status();
                // Control flow ends the task, not the tasks that run after it.
                {
                    let libdata = self.libdata_mut();
                    libdata.returning = false;
                    libdata.loop_status = LoopStatus::Normals;
                }
                self.set_current_task(previous_task);
                self.finish_stage_task(task_id, Some(status), previous_task);
                true
            }
        }
    }

    /// Finish a stage task, recording the status of `stage` when it has one.
    fn finish_stage_task(&mut self, task_id: TaskId, status: Option<c_int>, previous_task: TaskId) {
        if let Some(task) = self.task_mut(task_id) {
            let _ = task.transition_lifecycle_to(TaskLifecycle::Finished);
        }
        self.set_current_task(task_id);
        let assignment_block = self
            .task_mut(task_id)
            .and_then(|task| task.take_assignment_block());
        pop_assignment_block(self, assignment_block);
        self.set_current_task(previous_task);
        let Some(stage) = self.task_mut(task_id).and_then(|task| task.take_stage()) else {
            return;
        };
        if let Some(status) = status {
            self.complete_stage(task_id, stage, status);
        }
        let _ = self.remove_task(task_id);
    }

    /// Record the completion of the stage `stage` with `status`.
    fn complete_stage(&mut self, task_id: TaskId, stage: StageTask, status: c_int) {
        let Some(job) = self.job_with_internal_id(stage.job) else {
            return;
        };
        let Some(process) = job
            .processes()
            .iter()
            .find(|process| process.task_id.get() == Some(&task_id))
        else {
            return;
        };

        process.status.set(ProcStatus::from_exit_code(status));

        let mut io_chain = stage.io_chain;
        let mut buffered = vec![];
        if let Some(bufferfill) = stage.buffered_output {
            io_chain.remove(&*bufferfill);
            buffered = IoBufferfill::finish(bufferfill).newline_serialized();
        }
        run_internal_process_or_short_circuit(self, &job, process, buffered, vec![], &io_chain);
    }
}
