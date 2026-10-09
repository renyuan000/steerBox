use crate::domain::GoalContract;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriftDecision {
    Allow,
    Warn,
    RequireCheckpoint,
    RequireHumanApproval,
    Block,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffRiskSummary {
    pub changed_files: Vec<String>,
    pub changed_lines: u32,
    pub deleted_lines: u32,
    pub stable_artifacts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriftReport {
    pub decision: DriftDecision,
    pub out_of_scope_files: Vec<String>,
    pub stable_artifacts_touched: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy)]
pub struct DriftGuard {
    pub max_changed_files: usize,
    pub max_changed_lines: u32,
}

impl Default for DriftGuard {
    fn default() -> Self {
        Self {
            max_changed_files: 5,
            max_changed_lines: 300,
        }
    }
}

impl DriftGuard {
    pub fn evaluate(&self, goal: &GoalContract, diff: &DiffRiskSummary) -> DriftReport {
        let out_of_scope_files: Vec<_> = diff
            .changed_files
            .iter()
            .filter(|path| !goal.is_within_scope(path))
            .cloned()
            .collect();
        let stable_artifacts_touched: Vec<_> = diff
            .changed_files
            .iter()
            .filter(|path| diff.stable_artifacts.iter().any(|stable| stable == *path))
            .cloned()
            .collect();
        let (decision, reason) = if !out_of_scope_files.is_empty() {
            (DriftDecision::Block, "changed files exceed goal scope")
        } else if !stable_artifacts_touched.is_empty() {
            (
                DriftDecision::RequireHumanApproval,
                "stable artifact was modified",
            )
        } else if diff.changed_files.len() > self.max_changed_files
            || diff.changed_lines > self.max_changed_lines
        {
            (
                DriftDecision::RequireCheckpoint,
                "diff exceeds configured size threshold",
            )
        } else if diff.deleted_lines > 100 {
            (DriftDecision::Warn, "large deletion requires review")
        } else {
            (DriftDecision::Allow, "diff is within configured boundaries")
        };
        DriftReport {
            decision,
            out_of_scope_files,
            stable_artifacts_touched,
            reason: reason.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoalAlignmentReport {
    pub aligned: bool,
    pub out_of_scope_files: Vec<String>,
    pub missing_stage_goal: bool,
    pub reason: String,
}

pub fn check_goal_alignment(
    goal: &GoalContract,
    stage_goal: Option<&str>,
    changed_files: &[String],
) -> GoalAlignmentReport {
    let out_of_scope_files = changed_files
        .iter()
        .filter(|path| !goal.is_within_scope(path))
        .cloned()
        .collect::<Vec<_>>();
    let missing_stage_goal = stage_goal.map(str::trim).is_none_or(str::is_empty);
    let aligned = out_of_scope_files.is_empty() && !missing_stage_goal;
    let reason = if !out_of_scope_files.is_empty() {
        "changed files exceed goal scope"
    } else if missing_stage_goal {
        "stage goal is missing"
    } else {
        "changed files and stage goal are within root goal boundary"
    };
    GoalAlignmentReport {
        aligned,
        out_of_scope_files,
        missing_stage_goal,
        reason: reason.into(),
    }
}
