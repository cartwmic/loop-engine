//! Read-only terminal rendering of the stored generic workflow graph and work.
//!
//! This renderer consumes the same provider-free show projection as the other
//! CLI views. It has no mutation, provider, or workflow-action path.

use crate::{Execution, EXIT_INVALID_INVOCATION};
use loop_core::{HistoryEntry, ShowProjection};
use std::io::{self, IsTerminal};

#[cfg(unix)]
use crate::{fan_out, EXIT_COMPLETED};
#[cfg(unix)]
use loop_core::{
    DurableEvaluationResult, HistoryAction, Lifecycle, ProjectedInvocationStatus,
    TransitionHistoryOutcome, Workflow,
};
#[cfg(unix)]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(unix)]
use std::io::Write;

#[cfg(unix)]
#[derive(Clone, Debug)]
struct ExplorerAssignment {
    slot_id: String,
    state_id: Option<String>,
    label: loop_core::AssignmentLabel,
    execution: String,
    conformance: String,
    dependencies: Option<Vec<String>>,
    stored_data: serde_json::Value,
}

#[cfg(unix)]
fn worker_definition(binding: &loop_core::WorkSlotBinding, id: &str) -> serde_json::Value {
    if binding.args.first().map(String::as_str) == Some("fan-out") {
        if let Ok(parsed) = fan_out::parse_fan_out_args(binding.args.iter().skip(1)) {
            for (index, worker) in parsed.workers.iter().enumerate() {
                if fan_out::assignment_id(index) == id {
                    return serde_json::to_value(worker).unwrap_or_default();
                }
            }
        }
    }
    serde_json::json!({"command": binding.command, "args": binding.args})
}

#[cfg(unix)]
#[derive(Clone, Debug)]
enum ListItem {
    State(loop_core::State),
    Assignment(ExplorerAssignment),
}

#[cfg(unix)]
impl ListItem {
    fn key(&self) -> String {
        match self {
            Self::State(state) => format!("state:{}", state.id),
            Self::Assignment(assignment) => format!(
                "assignment:{}:{}",
                assignment.slot_id, assignment.label.assignment_id
            ),
        }
    }

    #[cfg(test)]
    fn assignment_id(&self) -> Option<&str> {
        match self {
            Self::State(_) => None,
            Self::Assignment(assignment) => Some(&assignment.label.assignment_id),
        }
    }

    fn short_label(&self, model: &ExplorerModel) -> String {
        match self {
            Self::State(state) => {
                let current = state.id.as_str() == model.current_state;
                let visited = model.visited_states.contains(state.id.as_str());
                let requestable = if current {
                    format!(" [requestable={}]", model.requestable_events.len())
                } else {
                    String::new()
                };
                format!(
                    "state:{} {}{}{}{} — {}",
                    state.id,
                    if current { "[CURRENT] " } else { "" },
                    if visited { "[VISITED] " } else { "" },
                    if state.is_final { "[FINAL] " } else { "" },
                    requestable,
                    state.title
                )
            }
            Self::Assignment(assignment) => format!(
                "assignment:{} {} [{}]",
                assignment.label.assignment_id, assignment.label.title, assignment.execution
            ),
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Debug)]
struct ExplorerModel {
    run_id: String,
    current_state: String,
    lifecycle: Lifecycle,
    workflow: Option<Workflow>,
    requestable_events: Vec<loop_core::RequestableEvent>,
    latest_evaluations: Vec<loop_core::DurableEvaluation>,
    visited_states: BTreeSet<String>,
    items: Vec<ListItem>,
}

#[cfg(unix)]
impl ExplorerModel {
    fn from_projection(projection: &ShowProjection, history: &[HistoryEntry]) -> Self {
        let workflow = projection
            .workflow_graph
            .clone()
            .filter(|workflow| !workflow.states.is_empty() && !workflow.transitions.is_empty());
        let mut visited_states = BTreeSet::new();
        if let Some(workflow) = &workflow {
            visited_states.insert(workflow.initial_state.to_string());
        }
        for entry in history {
            let HistoryAction::Transition {
                transition,
                outcome,
            } = &entry.action
            else {
                continue;
            };
            if matches!(
                outcome,
                TransitionHistoryOutcome::Committed
                    | TransitionHistoryOutcome::Overridden { .. }
                    | TransitionHistoryOutcome::AdviceException { .. }
                    | TransitionHistoryOutcome::DriverAct { .. }
            ) {
                visited_states.insert(transition.source.to_string());
                visited_states.insert(transition.target.to_string());
            }
        }

        let slot_states = projection
            .work_slots
            .iter()
            .map(|slot| (slot.id.to_string(), slot.state.to_string()))
            .collect::<BTreeMap<_, _>>();
        let mut assignment_map = BTreeMap::<(String, String), ExplorerAssignment>::new();

        // Frozen direct fan-out bindings declare opaque display descriptors
        // before any worker output exists. They are a work list, not evidence
        // that an invocation ran.
        let engine_binary = std::env::current_exe().ok();
        for (slot_id, binding) in &projection.effective_bindings {
            let is_engine_fan_out = engine_binary.as_deref().is_some_and(|engine| {
                crate::same_executable_file(engine, std::path::Path::new(&binding.command))
            });
            if !is_engine_fan_out {
                continue;
            }
            let Some(labels) = fan_out::enumerate_bound_assignment_labels(binding) else {
                continue;
            };
            for label in labels {
                assignment_map.insert(
                    (slot_id.clone(), label.assignment_id.clone()),
                    ExplorerAssignment {
                        slot_id: slot_id.clone(),
                        state_id: slot_states.get(slot_id).cloned(),
                        label: label.clone(),
                        execution: "configured".to_owned(),
                        conformance: "unknown".to_owned(),
                        dependencies: None,
                        stored_data: worker_definition(binding, &label.assignment_id),
                    },
                );
            }
        }

        let mut invocations = projection.work_slot_invocations.iter().collect::<Vec<_>>();
        invocations.sort_by_key(|invocation| invocation.started_at);
        for invocation in invocations {
            let execution = invocation_status(invocation.status);
            let mut labels = invocation
                .assignment_labels
                .iter()
                .cloned()
                .map(|label| (label.assignment_id.clone(), label))
                .collect::<BTreeMap<_, _>>();
            for worker in &invocation.inner_workers {
                if worker.assignment_id.is_empty() {
                    continue;
                }
                labels
                    .entry(worker.assignment_id.clone())
                    .or_insert_with(|| loop_core::AssignmentLabel {
                        assignment_id: worker.assignment_id.clone(),
                        title: worker.assignment_id.clone(),
                        role: "worker".to_owned(),
                    });
            }
            for (assignment_id, label) in labels {
                let worker = invocation
                    .inner_workers
                    .iter()
                    .find(|worker| worker.assignment_id == assignment_id);
                let assignment_execution = worker
                    .map(|worker| {
                        if worker.started == Some(false) {
                            "not-started".to_owned()
                        } else if worker.exit_code != 0 {
                            "failed".to_owned()
                        } else {
                            "finished".to_owned()
                        }
                    })
                    .unwrap_or_else(|| {
                        if matches!(
                            invocation.status,
                            ProjectedInvocationStatus::Running | ProjectedInvocationStatus::Overrun
                        ) {
                            execution.clone()
                        } else {
                            "unknown".to_owned()
                        }
                    });
                let conformance = worker
                    .and_then(|worker| worker.conformance_status.clone())
                    .unwrap_or_else(|| "unknown".to_owned());
                assignment_map.insert(
                    (invocation.slot_id.to_string(), assignment_id),
                    ExplorerAssignment {
                        slot_id: invocation.slot_id.to_string(),
                        state_id: slot_states.get(invocation.slot_id.as_str()).cloned(),
                        label: label.clone(),
                        execution: assignment_execution,
                        conformance,
                        dependencies: worker.and_then(|worker| worker.dependencies.clone()),
                        stored_data: serde_json::json!({
                            "worker_definition": worker_definition(&invocation.binding, &label.assignment_id),
                            "invocation_id": invocation.invocation_id,
                            "capture_dir": invocation.capture_dir,
                            "subject": invocation.subject,
                            "note": "Stored CLI definition, not a claim about the delivered stdin. Original packet is retained at the capture directory."
                        }),
                    },
                );
            }
        }

        let assignments = assignment_map.into_values().collect::<Vec<_>>();
        let assignments_by_slot = assignments.iter().fold(
            BTreeMap::<String, Vec<ExplorerAssignment>>::new(),
            |mut by_slot, assignment| {
                by_slot
                    .entry(assignment.slot_id.clone())
                    .or_default()
                    .push(assignment.clone());
                by_slot
            },
        );
        let mut items = Vec::new();
        if let Some(workflow) = &workflow {
            for state in &workflow.states {
                items.push(ListItem::State(state.clone()));
                for slot in workflow
                    .work_slots
                    .iter()
                    .filter(|slot| slot.state == state.id)
                {
                    if let Some(slot_assignments) = assignments_by_slot.get(slot.id.as_str()) {
                        items.extend(slot_assignments.iter().cloned().map(ListItem::Assignment));
                    }
                }
            }
        } else {
            items.extend(assignments.iter().cloned().map(ListItem::Assignment));
        }
        // Keep descriptors reachable even if a legacy or partial graph omitted
        // the corresponding slot. No edge or dependency is inferred for them.
        let mut included = items
            .iter()
            .filter_map(|item| match item {
                ListItem::Assignment(assignment) => Some((
                    assignment.slot_id.clone(),
                    assignment.label.assignment_id.clone(),
                )),
                ListItem::State(_) => None,
            })
            .collect::<BTreeSet<_>>();
        for assignment in &assignments {
            if included.insert((
                assignment.slot_id.clone(),
                assignment.label.assignment_id.clone(),
            )) {
                items.push(ListItem::Assignment(assignment.clone()));
            }
        }

        Self {
            run_id: projection.run_id.to_string(),
            current_state: projection.current_state.to_string(),
            lifecycle: projection.lifecycle,
            workflow,
            requestable_events: projection.requestable_events.clone(),
            latest_evaluations: projection.latest_evaluations.clone(),
            visited_states,
            items,
        }
    }

    fn initial_selection(&self) -> Option<String> {
        let current = format!("state:{}", self.current_state);
        self.items
            .iter()
            .find(|item| item.key() == current)
            .or_else(|| self.items.first())
            .map(ListItem::key)
    }

    fn selected_index(&self, selected: &str) -> Option<usize> {
        self.items.iter().position(|item| item.key() == selected)
    }

    fn detail(&self, item: Option<&ListItem>) -> Vec<String> {
        let Some(item) = item else {
            return vec!["No assignments are configured.".to_owned()];
        };
        match item {
            ListItem::State(state) => self.state_detail(state),
            ListItem::Assignment(assignment) => self.assignment_detail(assignment),
        }
    }

    fn state_detail(&self, state: &loop_core::State) -> Vec<String> {
        let current = state.id.as_str() == self.current_state;
        let visited = self.visited_states.contains(state.id.as_str());
        let mut lines = vec![
            format!("State {} — {}", state.id, state.title),
            format!("current: {current}"),
            format!("visited: {visited} (history only; visited does not mean passed)"),
        ];
        let Some(workflow) = &self.workflow else {
            lines.push("No stored workflow graph; showing only available work.".to_owned());
            lines.push(format!("instructions: {}", state.instructions));
            return lines;
        };
        let slots = workflow
            .work_slots
            .iter()
            .filter(|slot| slot.state == state.id)
            .map(|slot| slot.id.to_string())
            .collect::<Vec<_>>();
        if !slots.is_empty() {
            lines.push(format!("stored work slots: {}", slots.join(", ")));
        }
        let outgoing = workflow
            .transitions
            .iter()
            .filter(|transition| transition.source == state.id)
            .collect::<Vec<_>>();
        if outgoing.is_empty() {
            lines.push("No outgoing transitions are stored.".to_owned());
        }
        for transition in outgoing {
            let requestable = current
                && self.requestable_events.iter().any(|event| {
                    event.event == transition.event && event.target == transition.target
                });
            let mut route = format!(
                "event {} -> {} [{}; {}]",
                transition.event,
                transition.target,
                match transition.kind {
                    loop_core::TransitionKind::Checked => "checked",
                    loop_core::TransitionKind::CheckFree => "check-free",
                },
                if requestable {
                    "requestable now"
                } else {
                    "not requestable from the current state"
                }
            );
            if transition.kind.is_checked() {
                if let Some(evaluation) = self.latest_evaluations.iter().find(|evaluation| {
                    evaluation.transition.same_lineage(transition)
                        && evaluation.transition.target == transition.target
                }) {
                    match &evaluation.result {
                        DurableEvaluationResult::Deny { feedback } => {
                            route.push_str("; checked-not-allowed (latest recorded denial)");
                            lines.push(route);
                            lines.push(format!(
                                "  denial: {} — {}",
                                feedback.code, feedback.message
                            ));
                            continue;
                        }
                        DurableEvaluationResult::Allow => {
                            route.push_str("; latest recorded check: allow");
                        }
                    }
                } else {
                    route.push_str("; no checked outcome recorded");
                }
            }
            lines.push(route);
        }
        lines.push(format!("instructions: {}", state.instructions));
        lines
    }

    fn stored_data(&self, item: Option<&ListItem>) -> serde_json::Value {
        match item {
            Some(ListItem::State(state)) => serde_json::json!({
                "state": state.id, "instructions": state.instructions,
                "action_guidance": state.action_guidance
            }),
            Some(ListItem::Assignment(assignment)) => assignment.stored_data.clone(),
            None => serde_json::Value::Null,
        }
    }

    fn assignment_detail(&self, assignment: &ExplorerAssignment) -> Vec<String> {
        let mut lines = vec![
            format!("Assignment {}", assignment.label.assignment_id),
            format!("title: {}", assignment.label.title),
            format!("role: {}", assignment.label.role),
            format!("slot: {}", assignment.slot_id),
            format!(
                "state: {}",
                assignment.state_id.as_deref().unwrap_or("unknown")
            ),
            format!("execution: {}", assignment.execution),
            format!("output conformance: {}", assignment.conformance),
            "acceptance: unknown (the navigator does not infer acceptance)".to_owned(),
        ];
        if let Some(dependencies) = &assignment.dependencies {
            lines.push(format!(
                "recorded invocation dependencies: {dependencies:?}"
            ));
        }
        lines
    }
}

#[cfg(unix)]
fn invocation_status(status: ProjectedInvocationStatus) -> String {
    match status {
        ProjectedInvocationStatus::Running => "running",
        ProjectedInvocationStatus::Succeeded => "succeeded",
        ProjectedInvocationStatus::Failed => "failed",
        ProjectedInvocationStatus::Overrun => "overrun",
    }
    .to_owned()
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalSize {
    rows: usize,
    columns: usize,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Key {
    Up,
    Down,
    First,
    Last,
    DetailUp,
    DetailDown,
    PageUp,
    PageDown,
    Focus,
    Toggle,
    Open,
    Close,
    Quit,
    Other,
}

#[cfg(unix)]
#[derive(Default)]
struct DetailTree {
    expanded: BTreeSet<String>,
    cursor: String,
    offset: usize,
}

#[cfg(unix)]
struct TreeRow {
    path: String,
    text: String,
    branch: bool,
}

#[cfg(unix)]
fn tree_rows(value: &serde_json::Value, tree: &DetailTree) -> Vec<TreeRow> {
    fn visit(
        value: &serde_json::Value,
        path: String,
        label: &str,
        depth: usize,
        tree: &DetailTree,
        rows: &mut Vec<TreeRow>,
    ) {
        // Long prose is folded too: opening a JSON object should not flood the view.
        let prose = value.as_str().filter(|s| s.chars().count() > 160);
        let branch = value.is_object() || value.is_array() || prose.is_some();
        let open = tree.expanded.contains(&path);
        let description = if let Some(v) = value.as_object() {
            format!("{} fields", v.len())
        } else if let Some(v) = value.as_array() {
            format!("{} items", v.len())
        } else if let Some(v) = prose {
            format!("{} characters", v.chars().count())
        } else {
            value.to_string()
        };
        rows.push(TreeRow {
            path: path.clone(),
            text: format!(
                "{}{} {label}: {description}",
                "  ".repeat(depth),
                if branch {
                    if open {
                        "v"
                    } else {
                        ">"
                    }
                } else {
                    " "
                }
            ),
            branch,
        });
        if open {
            if let Some(v) = value.as_object() {
                for (key, child) in v {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    visit(
                        child,
                        format!("{path}/{escaped}"),
                        key,
                        depth + 1,
                        tree,
                        rows,
                    );
                }
            } else if let Some(v) = value.as_array() {
                for (index, child) in v.iter().enumerate() {
                    visit(
                        child,
                        format!("{path}/{index}"),
                        &index.to_string(),
                        depth + 1,
                        tree,
                        rows,
                    );
                }
            } else if let Some(v) = prose {
                rows.push(TreeRow {
                    path: format!("{path}/text"),
                    text: v.to_owned(),
                    branch: false,
                });
            }
        }
    }
    let mut rows = Vec::new();
    visit(value, String::new(), "Stored data", 0, tree, &mut rows);
    rows
}

#[cfg(unix)]
struct ExplorerView {
    selected: String,
    detail_focus: bool,
    details: BTreeMap<String, DetailTree>,
    follow_cursor: bool,
}

#[cfg(unix)]
impl ExplorerView {
    fn new(selected: String) -> Self {
        Self {
            selected,
            detail_focus: false,
            details: BTreeMap::new(),
            follow_cursor: false,
        }
    }

    fn key(&mut self, model: &ExplorerModel, key: Key) {
        let index = model.selected_index(&self.selected).unwrap_or(0);
        let item = model.items.get(index);
        let tree = self.details.entry(self.selected.clone()).or_default();
        match key {
            Key::Focus => {
                self.detail_focus = !self.detail_focus;
                self.follow_cursor = self.detail_focus;
            }
            Key::DetailUp | Key::PageUp => tree.offset = tree.offset.saturating_sub(3),
            Key::DetailDown | Key::PageDown => tree.offset = tree.offset.saturating_add(3),
            Key::Up | Key::Down if self.detail_focus => {
                let nodes = tree_rows(&model.stored_data(item), tree);
                let at = nodes
                    .iter()
                    .position(|node| node.path == tree.cursor)
                    .unwrap_or(0);
                let next = if key == Key::Down {
                    (at + 1).min(nodes.len() - 1)
                } else {
                    at.saturating_sub(1)
                };
                tree.cursor = nodes[next].path.clone();
                self.follow_cursor = true;
            }
            Key::Toggle | Key::Open | Key::Close => {
                if !self.detail_focus {
                    self.detail_focus = true;
                }
                let nodes = tree_rows(&model.stored_data(item), tree);
                if let Some(node) = nodes.iter().find(|node| node.path == tree.cursor) {
                    if key == Key::Close {
                        if !tree.expanded.remove(&node.path) {
                            tree.cursor = node
                                .path
                                .rsplit_once('/')
                                .map(|(parent, _)| parent.to_owned())
                                .unwrap_or_default();
                        }
                    } else if node.branch
                        && (key != Key::Toggle || !tree.expanded.remove(&node.path))
                    {
                        tree.expanded.insert(node.path.clone());
                    }
                }
                self.follow_cursor = true;
            }
            Key::Up | Key::Down | Key::First | Key::Last if !self.detail_focus => {
                let next = match key {
                    Key::Up => index.saturating_sub(1),
                    Key::Down => (index + 1).min(model.items.len().saturating_sub(1)),
                    Key::First => 0,
                    _ => model.items.len().saturating_sub(1),
                };
                if let Some(item) = model.items.get(next) {
                    self.selected = item.key();
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn run(projection: &ShowProjection, history: &[HistoryEntry]) -> Execution {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return failed("explore requires an interactive terminal on stdin and stdout");
    }
    #[cfg(unix)]
    {
        run_unix(projection, history)
    }
    #[cfg(not(unix))]
    {
        let _ = (projection, history);
        failed("interactive terminal navigation is currently supported on Unix terminals")
    }
}

fn failed(message: &str) -> Execution {
    Execution {
        exit_code: EXIT_INVALID_INVOCATION,
        stdout: String::new(),
        stderr: format!("explore: {message}\n"),
    }
}

#[cfg(unix)]
fn run_unix(projection: &ShowProjection, history: &[HistoryEntry]) -> Execution {
    let model = ExplorerModel::from_projection(projection, history);
    let mut view = ExplorerView::new(
        model
            .initial_selection()
            .unwrap_or_else(|| "work-list".to_owned()),
    );
    let mut frame = 0u64;
    let mut last_size = None;
    let mut dirty = true;
    let _terminal = match RawTerminal::enter() {
        Ok(terminal) => terminal,
        Err(error) => return failed(&format!("could not enter raw terminal mode: {error}")),
    };
    let mut output = io::stdout().lock();

    loop {
        let size = match terminal_size(libc::STDOUT_FILENO) {
            Ok(size) => size,
            Err(error) => return failed(&format!("could not read terminal size: {error}")),
        };
        if last_size != Some(size) {
            last_size = Some(size);
            view.follow_cursor = view.detail_focus;
            dirty = true;
        }
        if dirty {
            frame = frame.saturating_add(1);
            let rendered = render_view(&model, &mut view, size, frame);
            if let Err(error) = output
                .write_all(rendered.as_bytes())
                .and_then(|_| output.flush())
            {
                return failed(&format!("could not render terminal view: {error}"));
            }
            dirty = false;
        }

        let key = match read_key(100) {
            Ok(Some(key)) => key,
            Ok(None) => continue,
            Err(error) => return failed(&format!("could not read terminal input: {error}")),
        };
        if key == Key::Quit {
            break;
        }
        view.key(&model, key);
        dirty = key != Key::Other;
    }
    let _ = output.write_all(b"\x1b[0m\x1b[?25h\r\n");
    let _ = output.flush();
    Execution {
        exit_code: EXIT_COMPLETED,
        stdout: String::new(),
        stderr: String::new(),
    }
}

#[cfg(all(test, unix))]
fn render_frame(
    model: &ExplorerModel,
    selected: &str,
    detail_offset: usize,
    size: TerminalSize,
    frame: u64,
) -> String {
    let mut view = ExplorerView::new(selected.to_owned());
    view.details.entry(selected.to_owned()).or_default().offset = detail_offset;
    render_view(model, &mut view, size, frame)
}

#[cfg(unix)]
fn render_view(
    model: &ExplorerModel,
    view: &mut ExplorerView,
    size: TerminalSize,
    frame: u64,
) -> String {
    let columns = size.columns.saturating_sub(1).max(1);
    let rows = size.rows.max(8);
    let selected = &view.selected;
    let selected_item = model.items.iter().find(|item| item.key() == *selected);
    let selection = match selected_item {
        Some(ListItem::State(state)) => format!("selected=state:{}", state.id),
        Some(ListItem::Assignment(a)) => format!(
            "selected_assignment_id={} slot={}",
            a.label.assignment_id, a.slot_id
        ),
        None => "selected=work-list".to_owned(),
    };
    // Colors use the terminal palette and default background, never RGB literals.
    let heading = "\x1b[1;36m";
    let muted = "\x1b[2m";
    let warning = "\x1b[33m";
    let highlight = "\x1b[1;36;7m";
    let mut lines = vec![
        (
            "Loop Engine — read-only workflow navigator".to_owned(),
            heading,
        ),
        (
            format!(
                "run={} current={} | {:?}",
                model.run_id, model.current_state, model.lifecycle
            ),
            heading,
        ),
        (
            format!("terminal-size={}x{} frame={frame}", size.rows, size.columns),
            muted,
        ),
        (selection, muted),
        (
            if model.workflow.is_some() {
                format!("Workflow graph — {} items", model.items.len())
            } else {
                format!(
                    "Work list — no stored workflow graph; {} items",
                    model.items.len()
                )
            },
            heading,
        ),
    ];
    let item_rows = (rows / 5).clamp(2, 6);
    let index = model.selected_index(selected).unwrap_or(0);
    let start = index
        .saturating_sub(item_rows / 2)
        .min(model.items.len().saturating_sub(item_rows));
    for at in start..(start + item_rows).min(model.items.len()) {
        let item = &model.items[at];
        let text = item.short_label(model);
        let style = if at == index { highlight } else { muted };
        lines.push((
            format!("{} {text}", if at == index { ">" } else { " " }),
            style,
        ));
    }
    if model.items.is_empty() {
        lines.push(("  (no configured work items)".to_owned(), muted));
    }
    lines.push((
        format!(
            "DETAILS [{}]",
            if view.detail_focus {
                "focused"
            } else {
                "Tab to focus"
            }
        ),
        heading,
    ));
    let visible = rows.saturating_sub(lines.len() + 2).max(1);
    let tree = view.details.entry(selected.clone()).or_default();
    let mut details: Vec<(String, &str)> = model
        .detail(selected_item)
        .into_iter()
        .flat_map(|line| {
            let style = if line.starts_with("acceptance:")
                || line.starts_with("visited:")
                || line.contains("checked-not-allowed")
            {
                warning
            } else {
                ""
            };
            wrap_line(&line, columns.saturating_sub(2).max(1))
                .into_iter()
                .map(move |s| (format!("  {s}"), style))
        })
        .collect();
    let mut cursor_line = details.len();
    for node in tree_rows(&model.stored_data(selected_item), tree) {
        if node.path == tree.cursor {
            cursor_line = details.len();
        }
        let active = view.detail_focus && node.path == tree.cursor;
        let style = if active {
            highlight
        } else if node.branch {
            heading
        } else {
            ""
        };
        let indent = (node.text.len() - node.text.trim_start().len()).min(columns / 2);
        for (part, line) in wrap_line(
            node.text.trim_start(),
            columns.saturating_sub(indent + 2).max(1),
        )
        .into_iter()
        .enumerate()
        {
            details.push((
                format!(
                    "{}{}{line}",
                    if active && part == 0 { "> " } else { "  " },
                    " ".repeat(indent)
                ),
                style,
            ));
        }
    }
    if view.follow_cursor {
        if cursor_line < tree.offset {
            tree.offset = cursor_line;
        } else if cursor_line >= tree.offset + visible {
            tree.offset = cursor_line + 1 - visible;
        }
        view.follow_cursor = false;
    }
    tree.offset = tree.offset.min(details.len().saturating_sub(visible));
    lines.extend(details.into_iter().skip(tree.offset).take(visible));
    while lines.len() < rows.saturating_sub(2) {
        lines.push((String::new(), ""));
    }
    lines.push((
        format!(
            "detail-offset={} | tree-node={} | Tab jk q",
            tree.offset, tree.cursor
        ),
        muted,
    ));
    lines.push((
        "Enter/Space toggle · h/l fold/open · [/] scroll".to_owned(),
        muted,
    ));
    lines.truncate(rows);
    format!(
        "\x1b[?25l\x1b[H\x1b[2J{}\r\n",
        lines
            .iter()
            .map(|(text, style)| format!("{style}{}\x1b[0m", clip_line(&sanitize(text), columns)))
            .collect::<Vec<_>>()
            .join("\r\n")
    )
}

#[cfg(unix)]
fn wrap_line(line: &str, width: usize) -> Vec<String> {
    let clean = sanitize(line);
    let mut wrapped = Vec::new();
    let mut current = String::new();
    for word in clean.split_whitespace() {
        let word_len = word.chars().count();
        if !current.is_empty() && current.chars().count() + 1 + word_len > width {
            wrapped.push(std::mem::take(&mut current));
        }
        if word_len > width {
            let mut chars = word.chars();
            while chars.clone().count() > width {
                wrapped.push(chars.by_ref().take(width).collect());
            }
            current.extend(chars);
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
    }
    if !current.is_empty() || wrapped.is_empty() {
        wrapped.push(current);
    }
    wrapped
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[cfg(unix)]
fn clip_line(line: &str, width: usize) -> String {
    line.chars().take(width).collect()
}

#[cfg(unix)]
struct RawTerminal {
    fd: libc::c_int,
    original: libc::termios,
}

#[cfg(unix)]
impl RawTerminal {
    fn enter() -> io::Result<Self> {
        let fd = libc::STDIN_FILENO;
        let mut original = std::mem::MaybeUninit::<libc::termios>::uninit();
        // SAFETY: `fd` is the process stdin TTY and the output pointer is
        // writable storage for the native termios structure.
        if unsafe { libc::tcgetattr(fd, original.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: tcgetattr initialized the termios value on success.
        let original = unsafe { original.assume_init() };
        let mut raw = original;
        // SAFETY: cfmakeraw mutates only the supplied termios value.
        unsafe { libc::cfmakeraw(&mut raw) };
        // SAFETY: `fd` is a TTY and `raw` is a valid initialized value.
        if unsafe { libc::tcsetattr(fd, libc::TCSANOW, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = io::stdout().write_all(b"\x1b[?25l");
        Ok(Self { fd, original })
    }
}

#[cfg(unix)]
impl Drop for RawTerminal {
    fn drop(&mut self) {
        // SAFETY: restore the saved terminal attributes to the same TTY.
        unsafe { libc::tcsetattr(self.fd, libc::TCSANOW, &self.original) };
        let _ = io::stdout().write_all(b"\x1b[0m\x1b[?25h");
        let _ = io::stdout().flush();
    }
}

#[cfg(unix)]
fn terminal_size(fd: libc::c_int) -> io::Result<TerminalSize> {
    let mut size = std::mem::MaybeUninit::<libc::winsize>::zeroed();
    // SAFETY: the ioctl writes one winsize structure to the provided pointer.
    if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, size.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: ioctl returned success and initialized the structure.
    let size = unsafe { size.assume_init() };
    if size.ws_row == 0 || size.ws_col == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "terminal reported zero rows or columns",
        ));
    }
    Ok(TerminalSize {
        rows: usize::from(size.ws_row),
        columns: usize::from(size.ws_col),
    })
}

#[cfg(unix)]
fn read_key(timeout_ms: i32) -> io::Result<Option<Key>> {
    let mut descriptor = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: poll receives one initialized descriptor for a bounded wait.
    let ready = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
    if ready == 0 {
        return Ok(None);
    }
    if ready < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(None);
        }
        return Err(error);
    }
    let mut first = 0u8;
    // SAFETY: reads one byte into valid writable storage from the polled TTY.
    let count = unsafe { libc::read(libc::STDIN_FILENO, (&mut first as *mut u8).cast(), 1) };
    if count == 0 {
        return Ok(None);
    }
    if count < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Ok(None);
        }
        return Err(error);
    }
    if first == 0x1b {
        let mut sequence = [0u8; 3];
        let mut length = 0usize;
        while length < sequence.len() {
            let mut next = libc::pollfd {
                fd: libc::STDIN_FILENO,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: bounded poll for the next byte of an escape sequence.
            if unsafe { libc::poll(&mut next, 1, 20) } <= 0 {
                break;
            }
            let mut byte = 0u8;
            // SAFETY: read exactly one byte so trailing key presses are retained.
            let count = unsafe { libc::read(libc::STDIN_FILENO, (&mut byte as *mut u8).cast(), 1) };
            if count <= 0 {
                break;
            }
            sequence[length] = byte;
            length += 1;
            if matches!(
                &sequence[..length],
                b"[A"
                    | b"[B"
                    | b"[C"
                    | b"[D"
                    | b"[H"
                    | b"[F"
                    | b"OA"
                    | b"OB"
                    | b"OC"
                    | b"OD"
                    | b"OH"
                    | b"OF"
                    | b"[5~"
                    | b"[6~"
            ) {
                break;
            }
        }
        return Ok(Some(match &sequence[..length] {
            b"[A" | b"OA" => Key::Up,
            b"[B" | b"OB" => Key::Down,
            b"[C" | b"OC" => Key::Open,
            b"[D" | b"OD" => Key::Close,
            b"[H" | b"OH" => Key::First,
            b"[F" | b"OF" => Key::Last,
            b"[5~" => Key::PageUp,
            b"[6~" => Key::PageDown,
            _ => Key::Other,
        }));
    }
    Ok(Some(match first {
        b'j' => Key::Down,
        b'k' => Key::Up,
        b'g' => Key::First,
        b'G' => Key::Last,
        b'[' => Key::DetailUp,
        b']' => Key::DetailDown,
        b'\t' => Key::Focus,
        b' ' | b'\r' | b'\n' => Key::Toggle,
        b'l' => Key::Open,
        b'h' => Key::Close,
        b'q' | b'Q' | 0x03 => Key::Quit,
        _ => Key::Other,
    }))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use loop_core::{
        RequestableEvent, State, Transition, TransitionKind, WorkSlot, Workflow, WorkflowId,
    };
    use serde_json::json;

    fn empty_projection(workflow: Option<Workflow>) -> ShowProjection {
        let workflow_id = workflow
            .as_ref()
            .map(|workflow| workflow.id.clone())
            .unwrap_or_else(|| WorkflowId::from("legacy"));
        ShowProjection {
            override_summary: loop_core::OverrideSummary::default(),
            run_id: "explorer-test".into(),
            label: None,
            workflow_id,
            workflow_graph: workflow,
            lifecycle: Lifecycle::Active,
            current_state: "middle".into(),
            current_state_title: "Middle".to_owned(),
            current_state_instructions: String::new(),
            action_guidance: None,
            initial_input: json!({}),
            state_visit: 0,
            binding_amendments: Vec::new(),
            effective_bindings: BTreeMap::new(),
            context: Vec::new(),
            requestable_events: Vec::new(),
            latest_evaluations: Vec::new(),
            evaluation_history: Vec::new(),
            work_slots: Vec::new(),
            change_report: loop_core::operations::RunChangeReport {
                assignments: Vec::new(),
                plan_task_results: Vec::new(),
            },
            work_slot_invocations: Vec::new(),
        }
    }

    #[test]
    fn explorer_is_an_interactive_read_only_command_not_a_json_action() {
        assert!(matches!(
            crate::parse_args(["explore", "run-1"]).unwrap(),
            crate::ParsedRequest::TerminalExplore { .. }
        ));
        assert_eq!(
            crate::parse_args(["--json", "explore", "run-1"])
                .unwrap_err()
                .code,
            "invalid-invocation"
        );
        assert!(crate::parse_args(["explore", "run-1", "--timeout-ms", "1"]).is_err());
    }

    #[test]
    fn initial_selection_uses_current_state_instead_of_graph_head() {
        let workflow = Workflow::new(
            "three-state",
            "head",
            vec![
                State::new("head", "Graph head", "", false),
                State::new("middle", "Current state", "", false),
                State::new("end", "End", "", true),
            ],
            vec![Transition::check_free("head", "next", "middle")],
        );
        let model = ExplorerModel::from_projection(&empty_projection(Some(workflow)), &[]);
        assert_eq!(model.initial_selection().as_deref(), Some("state:middle"));
    }

    #[test]
    fn graphless_projection_is_a_work_list_without_fabricated_barriers() {
        let mut projection = empty_projection(None);
        projection.work_slots = vec![WorkSlot::new("tasks", "middle", "inspect")];
        projection.effective_bindings.insert(
            "tasks".to_owned(),
            loop_core::WorkSlotBinding::new(
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                vec![
                    "fan-out".to_owned(),
                    "--worker".to_owned(),
                    serde_json::to_string(&json!({
                        "command":"true","args":[],"title":"Task one","role":"worker"
                    }))
                    .unwrap(),
                ],
            ),
        );
        let model = ExplorerModel::from_projection(&projection, &[]);
        assert_eq!(
            model.initial_selection().as_deref(),
            Some("assignment:tasks:worker-0")
        );
        let frame = render_frame(
            &model,
            &model.initial_selection().unwrap(),
            0,
            TerminalSize {
                rows: 24,
                columns: 80,
            },
            1,
        );
        assert!(frame.contains("Work list — no stored workflow graph"));
        assert!(frame.contains("assignment:worker-0"));
        assert!(!frame.to_ascii_lowercase().contains("barrier"));
    }

    #[test]
    fn checked_denial_stays_distinct_from_requestability_and_visitation() {
        let transition = Transition::new("middle", "advance", "end", TransitionKind::Checked);
        let mut projection = empty_projection(Some(Workflow::new(
            "workflow",
            "head",
            vec![
                State::new("head", "Head", "", false),
                State::new("middle", "Middle", "", false),
                State::new("end", "End", "", true),
            ],
            vec![transition.clone()],
        )));
        projection.requestable_events = vec![RequestableEvent::from_transition(&transition)];
        projection.latest_evaluations = vec![loop_core::DurableEvaluation::deny(
            transition,
            loop_core::EvaluationFeedback::new("not-ready", "Evidence is missing"),
            loop_core::SemanticSequence::new(2),
            loop_core::Timestamp::from_unix_millis(2),
        )];
        let model = ExplorerModel::from_projection(
            &projection,
            &[HistoryEntry::run_created(
                loop_core::SemanticSequence::new(1),
                loop_core::Timestamp::from_unix_millis(1),
            )],
        );
        let detail = model.detail(model.items.get(1));
        let joined = detail.join("\n");
        assert!(joined.contains("requestable now"));
        assert!(joined.contains("checked-not-allowed"));
        assert!(joined.contains("visited: false"));
        assert!(joined.contains("visited does not mean passed"));
    }

    #[test]
    fn stable_assignment_identity_includes_its_frozen_slot() {
        let mut projection = empty_projection(Some(
            Workflow::new(
                "workflow",
                "middle",
                vec![State::new("middle", "Middle", "", false)],
                vec![Transition::check_free("middle", "inspect", "middle")],
            )
            .with_work_slots(vec![WorkSlot::new("tasks", "middle", "inspect")]),
        ));
        projection.work_slots = projection
            .workflow_graph
            .as_ref()
            .unwrap()
            .work_slots
            .clone();
        projection.effective_bindings.insert(
            "tasks".to_owned(),
            loop_core::WorkSlotBinding::new(
                std::env::current_exe()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                vec![
                    "fan-out".to_owned(),
                    "--worker".to_owned(),
                    serde_json::to_string(&json!({
                        "command":"true","args":[],"title":"Task one","role":"worker"
                    }))
                    .unwrap(),
                ],
            ),
        );
        let model = ExplorerModel::from_projection(&projection, &[]);
        assert_eq!(model.items[1].key(), "assignment:tasks:worker-0");
        assert_eq!(model.items[1].assignment_id(), Some("worker-0"));
    }
}
