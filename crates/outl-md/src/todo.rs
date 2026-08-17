//! Shared task-marker parsing.

/// Recognised task states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TodoState {
    /// Open task.
    Todo,
    /// Task in progress.
    Doing,
    /// Completed task.
    Done,
}

impl TodoState {
    /// Canonical marker spelling without its trailing space.
    pub fn as_str(self) -> &'static str {
        match self { Self::Todo => "TODO", Self::Doing => "DOING", Self::Done => "DONE" }
    }
    /// Lowercase spelling used by query wire formats.
    pub fn wire_str(self) -> &'static str {
        match self { Self::Todo => "todo", Self::Doing => "doing", Self::Done => "done" }
    }
}

/// Split a task marker from block text, accepting canonical and checkbox forms.
pub fn split_todo(raw: &str) -> (Option<TodoState>, &str) {
    const PREFIXES: [(&str, TodoState); 7] = [
        ("TODO ", TodoState::Todo), ("DOING ", TodoState::Doing),
        ("DONE ", TodoState::Done), ("[ ] ", TodoState::Todo),
        ("[/] ", TodoState::Doing), ("[x] ", TodoState::Done),
        ("[X] ", TodoState::Done),
    ];
    for (prefix, state) in PREFIXES {
        if let Some(rest) = raw.strip_prefix(prefix) { return (Some(state), rest); }
    }
    (None, raw)
}
