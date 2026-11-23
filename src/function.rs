// Functions for storing and retrieving function information. These functions also take care of
// autoloading functions in the $fish_function_path. Actual function evaluation is taken care of by
// the parser and to some degree the builtin handling library.

use crate::{
    ast::{self, Node as _},
    autoload::{Autoload, AutoloadResult},
    common::valid_func_name,
    complete::complete_wrap_map,
    env::Environment,
    event::{self, EventDescription},
    global_safety::RelaxedAtomicBool,
    parse_tree::NodeRef,
    parser::Parser,
    parser_keywords::parser_keywords_is_reserved,
    prelude::*,
    proc::Pid,
    wutil::dir_iter::DirIter,
};
use fish_common::{FilenameRef, assert_sync, escape};
use fish_widestring::wcs2bytes;
use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU32,
    sync::{Arc, LazyLock, Mutex, MutexGuard},
};

#[derive(Clone)]
pub struct FunctionProperties {
    /// Reference to the node, along with the parsed source.
    pub func_node: NodeRef<ast::BlockStatement>,

    /// List of all named arguments for this function.
    pub named_arguments: Vec<WString>,

    /// Description of the function.
    pub description: LocalizableString,

    /// Mapping of all variables that were inherited from the function definition scope to their
    /// values, as (key, values) pairs.
    pub inherit_vars: Box<[(WString, Vec<WString>)]>,

    /// Set to true if invoking this function shadows the variables of the underlying function.
    pub shadow_scope: bool,

    /// Whether the function was autoloaded.
    /// This is the only field which is mutated after the properties are created.
    pub is_autoload: RelaxedAtomicBool,

    /// The file from which the function was created, or None if not from a file.
    pub definition_file: Option<FilenameRef>,

    /// If the function was created by copying an existing function node, the 1-based line number
    // of the copy statement.
    pub copy_definition_lineno: Option<NonZeroU32>,
}

/// FunctionProperties are safe to share between threads.
const _: () = assert_sync::<FunctionProperties>();

/// One frame of the function namespace: the definitions and tombstones a store adds over the frame
/// it extends.
#[derive(Clone)]
pub(crate) struct FunctionFrame {
    /// The definitions this frame holds.
    funcs: HashMap<WString, Arc<FunctionProperties>>,

    /// Names whose autoloading this frame prevents.
    autoload_tombstones: HashSet<WString>,

    /// The frame this one extends, or `None` for the shell's root.
    parent: Option<Arc<FunctionFrame>>,
}

impl FunctionFrame {
    /// Return a frame with no definitions, extending `parent`.
    fn new(parent: Option<Arc<FunctionFrame>>) -> Self {
        FunctionFrame {
            funcs: HashMap::new(),
            autoload_tombstones: HashSet::new(),
            parent,
        }
    }

    /// Return the definition `name` resolves to in this frame chain.
    fn get_props(&self, name: &wstr) -> Option<Arc<FunctionProperties>> {
        let mut frame = self;
        loop {
            if let Some(props) = frame.funcs.get(name) {
                return Some(Arc::clone(props));
            }
            if frame.autoload_tombstones.contains(name) {
                return None;
            }
            frame = frame.parent.as_deref()?;
        }
    }

    /// Return whether this frame chain tombstones `name`.
    fn tombstones(&self, name: &wstr) -> bool {
        let mut frame = self;
        loop {
            if frame.autoload_tombstones.contains(name) {
                return true;
            }
            match frame.parent.as_deref() {
                Some(parent) => frame = parent,
                None => return false,
            }
        }
    }

    /// Add a definition to this frame, replacing one it already holds.
    fn add(&mut self, name: WString, props: Arc<FunctionProperties>) {
        self.autoload_tombstones.remove(&name);
        self.funcs.insert(name, props);
    }

    /// Remove a definition from this frame and tombstone the name.
    fn remove(&mut self, name: &wstr) -> bool {
        if self.funcs.remove(name).is_some() {
            self.autoload_tombstones.insert(name.to_owned());
            true
        } else {
            false
        }
    }

    /// Append every definition name in this frame chain, respecting the tombstones of nearer
    /// frames.
    fn add_names(&self, names: &mut HashSet<WString>) {
        names.extend(self.funcs.keys().cloned());
        if let Some(parent) = self.parent.as_deref() {
            let mut parent_names = HashSet::new();
            parent.add_names(&mut parent_names);
            for name in parent_names {
                if !self.autoload_tombstones.contains(&name) {
                    names.insert(name);
                }
            }
        }
    }

    /// Drop the autoloaded definitions in this frame chain, copying a frame that a task shares.
    fn remove_autoloaded(&mut self) {
        self.funcs.retain(|_, props| !props.is_autoload.load());
        if let Some(parent) = self.parent.as_mut() {
            Arc::make_mut(parent).remove_autoloaded();
        }
    }
}

/// The function namespace a store evaluates against.
#[derive(Default)]
pub(crate) enum FunctionStore {
    /// The shell's namespace, shared by the process.
    #[default]
    Shell,
    /// A task's namespace: its own frame over the chain captured when the task started.
    #[cfg_attr(not(test), allow(dead_code))]
    Task(Arc<FunctionFrame>),
}

impl FunctionStore {
    /// Return the shell's namespace.
    pub fn shell() -> Self {
        FunctionStore::Shell
    }

    /// Return a store for a task started from this one: an empty frame over the namespace this
    /// store holds now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn copy_for_task(&self) -> Self {
        FunctionStore::Task(Arc::new(FunctionFrame::new(Some(self.current_frame()))))
    }

    /// Return the frame this store's namespace currently extends.
    #[cfg_attr(not(test), allow(dead_code))]
    fn current_frame(&self) -> Arc<FunctionFrame> {
        match self {
            FunctionStore::Shell => Arc::clone(&function_set().root),
            FunctionStore::Task(frame) => Arc::clone(frame),
        }
    }

    /// Return the properties for `name`, without autoloading.
    pub fn get_props(&self, name: &wstr) -> Option<Arc<FunctionProperties>> {
        if parser_keywords_is_reserved(name) {
            return None;
        }
        match self {
            FunctionStore::Shell => function_set().root.get_props(name),
            FunctionStore::Task(frame) => frame.get_props(name),
        }
    }

    /// Add or replace a function definition in this store.
    pub fn add(&mut self, name: WString, props: Arc<FunctionProperties>) {
        if name.is_empty() {
            return;
        }
        match self {
            FunctionStore::Shell => {
                let mut set = function_set();
                props
                    .is_autoload
                    .store(set.autoloader.autoload_in_progress(&name));
                let frame = Arc::make_mut(&mut set.root);
                if frame.funcs.contains_key(&name) {
                    event::remove_function_handlers(&name);
                }
                frame.add(name, props);
            }
            FunctionStore::Task(frame) => {
                props
                    .is_autoload
                    .store(function_set().autoloader.autoload_in_progress(&name));
                let frame = Arc::make_mut(frame);
                if frame.funcs.contains_key(&name) {
                    event::remove_function_handlers(&name);
                }
                frame.add(name, props);
            }
        }
    }

    /// Remove the function `name` from this store, preventing its autoloading.
    pub fn remove(&mut self, name: &wstr) {
        match self {
            FunctionStore::Shell => {
                let mut set = function_set();
                let frame = Arc::make_mut(&mut set.root);
                let removed = frame.remove(name);
                frame.autoload_tombstones.insert(name.to_owned());
                if removed {
                    event::remove_function_handlers(name);
                }
            }
            FunctionStore::Task(frame) => {
                let frame = Arc::make_mut(frame);
                frame.remove(name);
                frame.autoload_tombstones.insert(name.to_owned());
            }
        }
    }

    /// Append every function name this store knows, including autoloadable and embedded names.
    pub fn get_names(&self, get_hidden: bool, vars: &dyn Environment) -> Vec<WString> {
        let mut names = HashSet::<WString>::new();
        autoload_names(&mut names, vars, get_hidden);
        match self {
            FunctionStore::Shell => function_set().root.add_names(&mut names),
            FunctionStore::Task(frame) => frame.add_names(&mut names),
        }
        for name in crate::autoload::Asset::iter() {
            let Some(bname) = name.strip_prefix("functions/") else {
                continue;
            };
            if !get_hidden && (bname.is_empty() || bname.starts_with('_')) {
                continue;
            }
            let Some(fname) = bname.strip_suffix(".fish") else {
                continue;
            };
            names.insert(fname.into());
        }
        if !get_hidden {
            names.retain(|name| !name.is_empty() && name.char_at(0) != '_');
        }
        names.into_iter().collect()
    }

    /// Drop the autoloaded definitions this store holds.
    pub fn invalidate_path(&mut self) {
        match self {
            FunctionStore::Shell => {
                let mut set = function_set();
                Arc::make_mut(&mut set.root).remove_autoloaded();
                set.autoloader.clear();
            }
            FunctionStore::Task(frame) => {
                Arc::make_mut(frame).remove_autoloaded();
            }
        }
    }

    /// Insert a definition into this store's own frame without changing whether it is an autoload.
    fn insert_definition(&mut self, name: WString, props: Arc<FunctionProperties>) {
        match self {
            FunctionStore::Shell => Arc::make_mut(&mut function_set().root).add(name, props),
            FunctionStore::Task(frame) => Arc::make_mut(frame).add(name, props),
        }
    }

    /// Copy the definition `name` to `new_name`, leaving an existing definition untouched.
    fn copy_definition(
        &mut self,
        name: &wstr,
        new_name: WString,
        definition_file: Option<FilenameRef>,
        copy_definition_lineno: Option<NonZeroU32>,
    ) -> bool {
        let Some(props) = self.get_props(name) else {
            return false;
        };
        if self.get_props(&new_name).is_some() {
            return true;
        }
        let mut new_props = (*props).clone();
        new_props.is_autoload.store(false);
        new_props.definition_file = definition_file;
        new_props.copy_definition_lineno = copy_definition_lineno;
        self.insert_definition(new_name, Arc::new(new_props));
        true
    }
}

/// The shell's function namespace along with the autoloader that fills it.
struct FunctionSet {
    /// The shell's root frame.
    root: Arc<FunctionFrame>,

    /// The autoloader for our functions.
    autoloader: Autoload,
}

impl FunctionSet {
    fn new() -> Self {
        FunctionSet {
            root: Arc::new(FunctionFrame::new(None)),
            autoloader: Autoload::new(L!("fish_function_path")),
        }
    }
}

/// The big set of all shell functions.
static FUNCTION_SET: LazyLock<Mutex<FunctionSet>> =
    LazyLock::new(|| Mutex::new(FunctionSet::new()));

fn function_set() -> MutexGuard<'static, FunctionSet> {
    FUNCTION_SET.lock().unwrap()
}

/// Return whether `name` may be autoloaded into `store`.
fn allow_autoload(set: &FunctionSet, store: &FunctionStore, name: &wstr) -> bool {
    // Prohibit autoloading if we have a non-autoload (explicit) function, or if the function is
    // tombstoned. The frame is read directly because the set lock is already held.
    let (props, tombstoned) = match store {
        FunctionStore::Shell => (set.root.get_props(name), set.root.tombstones(name)),
        FunctionStore::Task(frame) => (frame.get_props(name), frame.tombstones(name)),
    };
    let has_explicit_func = props.is_some_and(|props| !props.is_autoload.load());
    !has_explicit_func && !tombstoned
}

/// Make sure that if the specified function is a dynamically loaded function, it has been fully
/// loaded. Note this executes fish script code.
pub fn load(name: &wstr, parser: &mut Parser) -> bool {
    let mut path_to_autoload = None;
    let store_lacks_props = parser.functions().get_props(name).is_none();
    // Note we can't autoload while holding the funcset lock.
    // Lock around a local region.
    {
        let mut set = function_set();
        if allow_autoload(&set, parser.functions(), name) {
            match set.autoloader.resolve_command(name, parser.vars()) {
                AutoloadResult::Path(path) => path_to_autoload = Some(path),
                AutoloadResult::Loaded if store_lacks_props => {
                    // The autoloader recorded a file as loaded into another store, so load it
                    // into this store as well.
                    set.autoloader.forget_autoloaded(name);
                    if let AutoloadResult::Path(path) =
                        set.autoloader.resolve_command(name, parser.vars())
                    {
                        path_to_autoload = Some(path);
                    }
                }
                _ => {}
            }
        }
    }

    // Release the lock and perform any autoload, then reacquire the lock and clean up.
    if let Some(path_to_autoload) = path_to_autoload.as_ref() {
        // Crucially, the lock is acquired after perform_autoload().
        Autoload::perform_autoload(path_to_autoload, parser);
        function_set().autoloader.mark_autoload_finished(name);
    }
    path_to_autoload.is_some()
}

/// Insert a list of all dynamically loaded functions into the specified list.
fn autoload_names(names: &mut HashSet<WString>, vars: &dyn Environment, get_hidden: bool) {
    let Some(path_var) = vars.get_unless_empty(L!("fish_function_path")) else {
        return;
    };
    let path_list = path_var.as_list();
    for ndir_str in path_list {
        let Ok(mut dir) = DirIter::new(ndir_str) else {
            continue;
        };
        while let Some(entry) = dir.next() {
            let Ok(entry) = entry else {
                continue;
            };
            let func: &WString = &entry.name;
            if !get_hidden && func.char_at(0) == '_' {
                continue;
            }
            let suffix: Option<usize> = func.chars().rposition(|x| x == '.');
            // We need a ".fish" *suffix*, it can't be the entire name.
            if let Some(suffix) = suffix {
                if suffix > 0 && entry.name.slice_from(suffix) == ".fish" {
                    // Also ignore directories.
                    if !entry.is_dir() {
                        let name = entry.name.slice_to(suffix).to_owned();
                        names.insert(name);
                    }
                }
            }
        }
    }
}

/// Return the properties for a function, or None, perhaps triggering autoloading.
pub fn get_props_autoload(name: &wstr, parser: &mut Parser) -> Option<Arc<FunctionProperties>> {
    if parser_keywords_is_reserved(name) {
        return None;
    }
    load(name, parser);
    parser.functions().get_props(name)
}

/// Returns true if the function named `cmd` exists.
/// This may autoload.
pub fn exists(cmd: &wstr, parser: &mut Parser) -> bool {
    if !valid_func_name(cmd) {
        return false;
    }
    get_props_autoload(cmd, parser).is_some()
}

/// Returns true if the shell's function `cmd` either is loaded, or exists on disk in an autoload
/// directory. Task-local definitions are not consulted.
pub fn exists_no_autoload(cmd: &wstr) -> bool {
    if !valid_func_name(cmd) {
        return false;
    }
    if parser_keywords_is_reserved(cmd) {
        return false;
    }
    let mut set = function_set();
    // Check if we either have the function, or it could be autoloaded.
    let root = Arc::clone(&set.root);
    let tombstoned = root.tombstones(cmd);
    if root.get_props(cmd).is_some() || (!tombstoned && set.autoloader.can_autoload(cmd)) {
        return true;
    }

    let narrow = wcs2bytes(cmd);
    if let Ok(cmdstr) = std::str::from_utf8(&narrow) {
        let cmd = "functions/".to_owned() + cmdstr + ".fish";
        crate::autoload::has_asset(&cmd)
    } else {
        false
    }
}

/// Sets the description of the function with the name \c name.
/// This triggers autoloading.
pub(crate) fn set_desc(name: &wstr, desc: WString, parser: &mut Parser) {
    load(name, parser);
    if let Some(props) = parser.functions().get_props(name) {
        // Note the description is immutable, as it may be accessed on another thread, so we copy
        // the properties to modify it.
        let mut new_props = (*props).clone();
        // Translations will only be available if the function description has been extracted into
        // the translation files available to fish.
        new_props.description = LocalizableString::from_external_source(desc);
        parser
            .functions_mut()
            .insert_definition(name.to_owned(), Arc::new(new_props));
    }
}

/// Creates a new function using the same definition as the specified function. Returns true if copy
/// is successful.
pub fn copy(name: &wstr, new_name: WString, parser: &mut Parser) -> bool {
    let definition_file = parser.current_filename();
    let copy_definition_lineno = parser.lineno();
    parser
        .functions_mut()
        .copy_definition(name, new_name, definition_file, copy_definition_lineno)
}

/// Observes that fish_function_path has changed.
pub fn invalidate_path() {
    // Remove all autoloaded functions and update the autoload path.
    FunctionStore::Shell.invalidate_path();
}

// Return the body of a function (everything after the header, up to but not including the 'end').
fn get_function_body_source(props: &FunctionProperties) -> &wstr {
    // We want to preserve comments that the AST attaches to the header (#5285).
    // Take everything from the end of the header to the 'end' keyword.
    let Some(header_source) = props.func_node.header.try_source_range() else {
        return L!("");
    };
    let Some(end_kw_source) = props.func_node.end.try_source_range() else {
        return L!("");
    };
    let body_start = header_source.start as usize + header_source.length as usize;
    let body_end = end_kw_source.start as usize;
    assert!(
        body_start <= body_end,
        "end keyword should come after header"
    );
    props
        .func_node
        .parsed_source()
        .src
        .slice_to(body_end)
        .slice_from(body_start)
}

impl FunctionProperties {
    /// Return a reference to the function's definition file, or None if it was defined interactively or copied.
    pub fn definition_file(&self) -> Option<&wstr> {
        self.definition_file.as_ref().map(|f| f.as_utfstr())
    }

    /// Return a reference to the vars that this function has inherited from its definition scope.
    pub fn inherit_vars(&self) -> &[(WString, Vec<WString>)] {
        &self.inherit_vars
    }

    /// Return the 1-based line number of the function's definition.
    pub fn definition_lineno(&self) -> i32 {
        if let Some(captured_lineno) = self.copy_definition_lineno {
            return i32::try_from(captured_lineno.get()).unwrap();
        }
        // Return one plus the number of newlines at offsets less than the start of our function's
        // statement (which includes the header).
        let Some(source_range) = self.func_node.try_source_range() else {
            panic!("Function has no source range");
        };
        let func_start = source_range.start as usize;
        let source = &self.func_node.parsed_source().src;
        assert!(
            func_start <= source.char_count(),
            "function start out of bounds"
        );
        1 + source
            .slice_to(func_start)
            .chars()
            .filter(|&c| c == '\n')
            .count() as i32
    }

    /// Return a definition of the function, annotated with properties like event handlers and wrap
    /// targets. This is to support the 'functions' builtin.
    /// Note callers must provide the function name, since the function does not know its own name.
    pub fn annotated_definition(&self, name: &wstr) -> WString {
        let mut out = WString::new();
        let desc = self.description.localize();
        let def = get_function_body_source(self);
        let handlers = event::get_function_handlers(name);

        out.push_str("function ");
        // Typically we prefer to specify the function name first, e.g. "function foo --description bar"
        // But if the function name starts with a -, we'll need to output it after all the options.
        let defer_function_name = name.char_at(0) == '-';
        if !defer_function_name {
            out.push_utfstr(&escape(name));
        }

        // Output wrap targets.
        if let Some(wrapped_cmds) = complete_wrap_map().get(name) {
            for wrapped_cmd in wrapped_cmds {
                out.push_str(" --wraps=");
                out.push_utfstr(&escape(wrapped_cmd));
            }
        }

        if !desc.is_empty() {
            out.push_str(" --description ");
            out.push_utfstr(&escape(desc));
        }

        if !self.shadow_scope {
            out.push_str(" --no-scope-shadowing");
        }

        for handler in handlers {
            let d = &handler.desc;
            match d {
                EventDescription::Signal { signal } => {
                    sprintf!(=> &mut out, " --on-signal %s", signal.name());
                }
                EventDescription::Variable { name } => {
                    sprintf!(=> &mut out, " --on-variable %s", name);
                }
                EventDescription::ProcessExit { pid } => {
                    let pid = pid.as_ref().map_or(0, Pid::get);
                    sprintf!(=> &mut out, " --on-process-exit %d", pid);
                }
                EventDescription::JobExit { pid, .. } => {
                    let pid = pid.as_ref().map_or(0, Pid::get);
                    sprintf!(=> &mut out, " --on-job-exit %d", pid);
                }
                EventDescription::CallerExit { .. } => {
                    out.push_str(" --on-job-exit caller");
                }
                EventDescription::Generic { param } => {
                    sprintf!(=> &mut out, " --on-event %s", param);
                }
                EventDescription::Any => {
                    panic!("Unexpected event handler type");
                }
            }
        }

        let named = &self.named_arguments;
        if !named.is_empty() {
            sprintf!(=> &mut out, " --argument-names");
            for name in named {
                sprintf!(=> &mut out, " %s", name);
            }
        }

        // Output the function name if we deferred it.
        if defer_function_name {
            out.push_str(" -- ");
            out.push_utfstr(&escape(name));
        }

        // Output any inherited variables as `set -l` lines.
        for (name, values) in self.inherit_vars.iter() {
            // We don't know what indentation style the function uses,
            // so we do what fish_indent would.
            sprintf!(=> &mut out, "\n    set -l %s", name);
            for arg in values {
                out.push(' ');
                out.push_utfstr(&escape(arg));
            }
        }
        out.push('\n');
        out.push_utfstr(def);

        // Append a newline before the 'end', unless there already is one there.
        if !def.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("end\n");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::FunctionFrame;
    use crate::io::IoChain;
    use crate::prelude::*;
    use crate::tests::prelude::*;
    use fish_widestring::L;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    #[serial]
    fn test_task_function_store_isolates_definitions() {
        test_init();
        let mut parser = TestParser::new();
        let _ = parser.eval(
            L!("function task_function_probe; echo parent; end"),
            &IoChain::new(),
        );
        let parent_props = parser
            .functions()
            .get_props(L!("task_function_probe"))
            .expect("the parent definition");

        let mut task_functions = parser.functions().copy_for_task();

        // The task sees the definition it captured.
        let captured_props = task_functions
            .get_props(L!("task_function_probe"))
            .expect("the captured definition");
        assert!(Arc::ptr_eq(&captured_props, &parent_props));

        // A definition the task adds is invisible to the shell.
        task_functions.add(L!("task_local_probe").into(), Arc::clone(&parent_props));
        assert!(task_functions.get_props(L!("task_local_probe")).is_some());
        assert!(
            parser
                .functions()
                .get_props(L!("task_local_probe"))
                .is_none()
        );

        // A later shell definition is invisible to the task.
        let mut later_props = (*parent_props).clone();
        later_props.description = LocalizableString::from_external_source(L!("later").to_owned());
        let later_props = Arc::new(later_props);
        parser
            .functions_mut()
            .add(L!("task_function_probe").into(), Arc::clone(&later_props));
        let shell_props = parser
            .functions()
            .get_props(L!("task_function_probe"))
            .expect("the later definition");
        assert!(Arc::ptr_eq(&shell_props, &later_props));
        let captured_props = task_functions
            .get_props(L!("task_function_probe"))
            .expect("the captured definition");
        assert!(Arc::ptr_eq(&captured_props, &parent_props));

        // Erasing a captured definition in the task hides it there and leaves the shell intact.
        task_functions.remove(L!("task_function_probe"));
        assert!(
            task_functions
                .get_props(L!("task_function_probe"))
                .is_none()
        );
        assert!(
            parser
                .functions()
                .get_props(L!("task_function_probe"))
                .is_some()
        );

        // A tombstoned name is not listed even though a captured parent defines it.
        let mut parent_frame = FunctionFrame::new(None);
        parent_frame.funcs.insert(
            L!("task_function_probe").to_owned(),
            Arc::clone(&parent_props),
        );
        let mut task_frame = FunctionFrame::new(Some(Arc::new(parent_frame)));
        task_frame
            .autoload_tombstones
            .insert(L!("task_function_probe").to_owned());
        let mut names = HashSet::new();
        task_frame.add_names(&mut names);
        assert!(!names.contains(&L!("task_function_probe").to_owned()));

        parser.functions_mut().remove(L!("task_function_probe"));
        parser.functions_mut().remove(L!("task_local_probe"));
    }
}
