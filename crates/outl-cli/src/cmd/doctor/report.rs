//! The vocabulary every check writes into.
//!
//! A [`Finding`] is one sentence plus a [`Severity`], [`Builder`]
//! accumulates them in execution order while counting the two
//! severities a caller acts on, and [`DoctorReport`] is the serialized
//! shape the human printer, `--json` and the MCP shim all receive.
//!
//! No check lives here, on purpose. `super::collect_internal` owns the
//! *order* the checks run in; this file owns what they fill. Keeping
//! the two apart is what lets a new check be a function that takes
//! `&mut Builder` and knows nothing else about the run.

use serde::Serialize;

use super::RepairReport;

/// Severity of a single finding.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Healthy state — no action needed.
    Ok,
    /// Informational, not a problem.
    Info,
    /// Possible problem, user should look.
    Warn,
    /// Definite problem, blocks "integrity OK".
    Error,
}

/// One workspace check result.
#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    /// Severity bucket.
    pub severity: Severity,
    /// Human-readable description.
    pub message: String,
}

/// Aggregate of every finding from a doctor run.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    /// Workspace root path (display form).
    pub workspace: String,
    /// Actor id from `config.toml`.
    pub actor: String,
    /// Number of ops in the persisted log.
    pub op_count: usize,
    /// Every check emitted, in execution order.
    pub findings: Vec<Finding>,
    /// Convenience counts so callers don't have to count.
    pub error_count: usize,
    /// Number of warning findings.
    pub warn_count: usize,
    /// What `--repair` *would* do, in execution order. Filled on every
    /// run, so a read-only report already tells the user what is
    /// fixable and what is not.
    pub repairable: Vec<String>,
    /// What `--repair` actually did. `None` when the flag was off or
    /// there was nothing to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repair: Option<RepairReport>,
}

pub(super) struct Builder {
    workspace: String,
    actor: String,
    pub(super) op_count: usize,
    pub(super) findings: Vec<Finding>,
    errors: usize,
    warnings: usize,
}

impl Builder {
    pub(super) fn new(workspace: String, actor: String) -> Self {
        Self {
            workspace,
            actor,
            op_count: 0,
            findings: Vec::new(),
            errors: 0,
            warnings: 0,
        }
    }
    fn push(&mut self, severity: Severity, message: impl Into<String>) {
        if matches!(severity, Severity::Error) {
            self.errors += 1;
        }
        if matches!(severity, Severity::Warn) {
            self.warnings += 1;
        }
        self.findings.push(Finding {
            severity,
            message: message.into(),
        });
    }
    pub(super) fn ok(&mut self, msg: impl Into<String>) {
        self.push(Severity::Ok, msg);
    }
    pub(super) fn info(&mut self, msg: impl Into<String>) {
        self.push(Severity::Info, msg);
    }
    pub(super) fn warn(&mut self, msg: impl Into<String>) {
        self.push(Severity::Warn, msg);
    }
    pub(super) fn err(&mut self, msg: impl Into<String>) {
        self.push(Severity::Error, msg);
    }
    pub(super) fn into_report(
        self,
        repairable: Vec<String>,
        repair: Option<RepairReport>,
    ) -> DoctorReport {
        DoctorReport {
            workspace: self.workspace,
            actor: self.actor,
            op_count: self.op_count,
            findings: self.findings,
            error_count: self.errors,
            warn_count: self.warnings,
            repairable,
            repair,
        }
    }
}
