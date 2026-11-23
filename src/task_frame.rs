//! The resumable frames of a task's evaluation.
//!
//! A frame records one pending operation and the state that continues it. The evaluator keeps a
//! stack of frames: it advances the top frame, pushes a child frame when the operation begins a
//! nested construct, and resumes the parent with the child's outcome when that child completes.
//! A frame holds only node references, indices, block identities and the saved scoped data, so a
//! frame stack can be retained while a task waits.

use crate::ast;
use crate::env::Statuses;
use crate::event::Event;
use crate::parse_execution::EndExecutionReason;
use crate::parse_tree::{NodeRef, ParsedSourceRef};
use crate::parser::{BlockId, ScopedData};
use crate::timer::PrintElapsedOnDrop;
use fish_widestring::WString;

/// A task's stack of evaluation frames.
#[derive(Default)]
pub(crate) struct FrameStack(Vec<Frame>);

impl FrameStack {
    /// Return an empty frame stack.
    pub fn new() -> FrameStack {
        FrameStack(Vec::new())
    }

    /// Push a frame on top of the stack.
    pub fn push(&mut self, frame: Frame) {
        self.0.push(frame);
    }

    /// Pop the top frame.
    pub fn pop(&mut self) -> Option<Frame> {
        self.0.pop()
    }
}

/// One frame of a task's evaluation.
pub(crate) enum Frame {
    /// The remaining conjunctions of one command list.
    JobList(JobListFrame),
    /// The remaining jobs of one and-or list.
    AndorJobList(AndorJobListFrame),
    /// The remaining jobs of one conjunction.
    Conjunction(ConjunctionFrame),
    /// One statement, awaiting the control construct it selects.
    Statement(StatementFrame),
    /// One simple block job, awaiting the block statement it evaluates.
    SimpleBlockJob(Box<SimpleBlockJobFrame>),
    /// A begin or brace block, awaiting its body.
    Begin(BeginFrame),
    /// One if statement, at its current clause or body.
    If(IfFrame),
    /// One while statement, at its condition or body.
    While(WhileFrame),
    /// One for statement, at its current iteration.
    For(ForFrame),
    /// One switch statement, awaiting its matching case body.
    Switch(SwitchFrame),
}

impl Frame {
    /// Return the parsed source this frame evaluates.
    pub(crate) fn parsed_source(&self) -> ParsedSourceRef {
        match self {
            Frame::JobList(frame) => frame.jobs.parsed_source_ref(),
            Frame::AndorJobList(frame) => frame.jobs.parsed_source_ref(),
            Frame::Conjunction(frame) => frame.conjunction.parsed_source_ref(),
            Frame::Statement(frame) => frame.statement.parsed_source_ref(),
            Frame::SimpleBlockJob(frame) => frame.job_node.parsed_source_ref(),
            Frame::Begin(frame) => frame.contents.parsed_source_ref(),
            Frame::If(frame) => frame.statement.parsed_source_ref(),
            Frame::While(frame) => frame.header.parsed_source_ref(),
            Frame::For(frame) => frame.header.parsed_source_ref(),
            Frame::Switch(frame) => frame.statement.parsed_source_ref(),
        }
    }
}

/// The remaining conjunctions of one command list.
pub(crate) struct JobListFrame {
    /// The command list.
    pub(crate) jobs: NodeRef<ast::JobList>,

    /// The index of the next conjunction to evaluate.
    pub(crate) next_conjunction: usize,

    /// The block whose `run_1_job` association the conjunctions use.
    pub(crate) associated_block: Option<BlockId>,

    /// Whether the frame's entry checks have run.
    pub(crate) checked: bool,

    /// The outcome of the last conjunction.
    pub(crate) result: EndExecutionReason,
}

/// The remaining jobs of one and-or list.
pub(crate) struct AndorJobListFrame {
    /// The and-or list.
    pub(crate) jobs: NodeRef<ast::AndorJobList>,

    /// The index of the next job to evaluate.
    pub(crate) next_job: usize,

    /// The block whose `run_1_job` association the jobs use.
    pub(crate) associated_block: Option<BlockId>,

    /// The outcome of the last job.
    pub(crate) result: EndExecutionReason,
}

/// The remaining jobs of one conjunction.
pub(crate) struct ConjunctionFrame {
    /// The conjunction.
    pub(crate) conjunction: NodeRef<ast::JobConjunction>,

    /// The block whose `run_1_job` association the jobs use.
    pub(crate) associated_block: Option<BlockId>,

    /// The job to run next: 0 is the conjunction's own job, and `i` is continuation `i - 1`.
    pub(crate) next_job: usize,

    /// Whether the conjunction's own `and` or `or` decorator has been tested.
    pub(crate) tested_decorator: bool,

    /// Whether a child frame is evaluating the current job.
    pub(crate) awaiting_child: bool,

    /// The outcome of the last job.
    pub(crate) result: EndExecutionReason,
}

/// One statement, awaiting the control construct it selects.
pub(crate) struct StatementFrame {
    /// The statement.
    pub(crate) statement: NodeRef<ast::Statement>,

    /// The block whose `run_1_job` association the statement uses.
    pub(crate) associated_block: Option<BlockId>,

    /// Whether the selected construct has been pushed.
    pub(crate) started: bool,
}

/// One simple block job, awaiting the block statement it evaluates.
pub(crate) struct SimpleBlockJobFrame {
    /// The job whose statement is the simple block.
    pub(crate) job_node: NodeRef<ast::JobPipeline>,

    /// The job's statement.
    pub(crate) statement: NodeRef<ast::Statement>,

    /// The block whose `run_1_job` association the statement uses.
    pub(crate) associated_block: Option<BlockId>,

    /// Whether the job's setup has run.
    pub(crate) started: bool,

    /// The variable-assignment block to pop when the statement completes.
    pub(crate) assignment_block: Option<BlockId>,

    /// The scoped data to restore when the job completes.
    pub(crate) saved_scope: ScopedData,

    /// The current pipeline node to restore when the job completes.
    pub(crate) saved_node: Option<NodeRef<ast::JobPipeline>>,

    /// The timer whose drop prints the job's elapsed time.
    pub(crate) timer: Option<PrintElapsedOnDrop>,

    /// The profile entry to finish when the job completes.
    pub(crate) profile_item: Option<ProfileItemFinish>,
}

/// The profile entry a simple block job finishes when it completes.
pub(crate) struct ProfileItemFinish {
    /// The profile item's identity.
    pub(crate) item_id: usize,

    /// The time the job started.
    pub(crate) start_time: i64,

    /// The command name the profile item records.
    pub(crate) command: WString,
}

/// A begin or brace block, awaiting its body.
pub(crate) struct BeginFrame {
    /// The block's body.
    pub(crate) contents: NodeRef<ast::JobList>,

    /// The block pushed for the body.
    pub(crate) block_id: BlockId,

    /// Whether the body has been pushed.
    pub(crate) started: bool,
}

/// The stage of an if statement.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum IfStage {
    /// A clause condition is being evaluated.
    Condition,

    /// A clause's and-or tail is being evaluated.
    Andor,

    /// A clause body is being evaluated.
    Body,

    /// The else body is being evaluated.
    Else,
}

/// One if statement, at its current clause or body.
pub(crate) struct IfFrame {
    /// The if statement.
    pub(crate) statement: NodeRef<ast::IfStatement>,

    /// The block whose `run_1_job` association the conditions use.
    pub(crate) associated_block: Option<BlockId>,

    /// The clause index: 0 is the `if` clause, and `i` is `elseif_clauses[i - 1]`.
    pub(crate) clause_index: usize,

    /// The stage the frame evaluates.
    pub(crate) stage: IfStage,

    /// Whether a child frame is evaluating the current stage.
    pub(crate) awaiting_child: bool,

    /// The block pushed for a chosen body.
    pub(crate) body_block: Option<BlockId>,

    /// The if statement's outcome.
    pub(crate) result: EndExecutionReason,
}

/// The stage of a while statement.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum WhileStage {
    /// The condition conjunction is being evaluated.
    Condition,

    /// The condition's and-or tail is being evaluated.
    Andor,

    /// The loop body is being evaluated.
    Body,
}

/// One while statement, at its condition or body.
pub(crate) struct WhileFrame {
    /// The while header.
    pub(crate) header: NodeRef<ast::WhileHeader>,

    /// The loop body.
    pub(crate) contents: NodeRef<ast::JobList>,

    /// The block whose `run_1_job` association the condition uses.
    pub(crate) associated_block: Option<BlockId>,

    /// Whether the frame's setup has run.
    pub(crate) started: bool,

    /// Whether the coming condition is the first one.
    pub(crate) first_condition: bool,

    /// The statuses to restore when the condition fails.
    pub(crate) saved_status: Statuses,

    /// The condition conjunction's outcome.
    pub(crate) condition_result: EndExecutionReason,

    /// The stage the frame evaluates.
    pub(crate) stage: WhileStage,

    /// Whether a child frame is evaluating the current stage.
    pub(crate) awaiting_child: bool,

    /// The block pushed for the loop body.
    pub(crate) body_block: Option<BlockId>,

    /// The while statement's outcome.
    pub(crate) result: EndExecutionReason,
}

/// One for statement, at its current iteration.
pub(crate) struct ForFrame {
    /// The for header.
    pub(crate) header: NodeRef<ast::ForHeader>,

    /// The loop body.
    pub(crate) contents: NodeRef<ast::JobList>,

    /// The loop variable, set when the frame starts.
    pub(crate) var_name: Option<WString>,

    /// The values the loop iterates over.
    pub(crate) arguments: Vec<WString>,

    /// The variable-set event fired for each value.
    pub(crate) event: Option<Event>,

    /// The index of the value to evaluate next.
    pub(crate) next_value: usize,

    /// The block pushed for the loop body.
    pub(crate) body_block: Option<BlockId>,

    /// Whether the frame's setup has run.
    pub(crate) started: bool,

    /// The for statement's outcome.
    pub(crate) result: EndExecutionReason,
}

/// One switch statement, awaiting its matching case body.
pub(crate) struct SwitchFrame {
    /// The switch statement.
    pub(crate) statement: NodeRef<ast::SwitchStatement>,

    /// The block pushed for the case body.
    pub(crate) block_id: Option<BlockId>,

    /// Whether the case search has run.
    pub(crate) searched: bool,

    /// The outcome of the case search, or of the case body.
    pub(crate) result: EndExecutionReason,

    /// Whether a child frame is evaluating the case body.
    pub(crate) awaiting_child: bool,
}
