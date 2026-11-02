//! Commands: tasks (Outlook's Tasks).

use mailcraft_model::pim::{Task, TaskPriority, TaskStatus};

use super::*;
use crate::session::pim::TasksOrder;

/// What a task's params look like (create and update; update also takes its `uid`).
const TASK_PARAMS: &str = r#"{"title": string, "notes"?: string, "start"?: "YYYY-MM-DD" | "" (none), "due"?: "YYYY-MM-DD" | "" (none), "status"?: "notStarted"|"inProgress"|"completed"|"waiting"|"deferred", "percent"?: 0-100, "priority"?: "low"|"normal"|"high", "categories"?: [string] | "Work, Clients", "reminder"?: "YYYY-MM-DDTHH:MM" | "none", "repeat"?: "daily"|"weekly"|"monthly"|"yearly"|RRULE|"none", "account"?: string}"#;

fn has_task(s: &Session) -> Option<&'static str> {
    if s.pim.values().any(|p| !p.tasks.is_empty()) { None } else { Some("there are no tasks") }
}

/// A task as commands answer it.
fn task_json(account: &str, t: &Task) -> Value {
    json!({
        "uid": t.uid, "account": account, "title": t.title, "notes": t.notes, "start": t.start, "due": t.due,
        "completed": t.completed, "status": t.status.code(), "percent": t.percent, "priority": t.priority.code(),
        "categories": t.categories, "reminder": t.reminder, "repeat": t.rrule, "modified": t.modified,
    })
}

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // Outlook's key for Tasks.
        CommandSpec::new("view.tasks", "Tasks", "Navigation", |s, _| {
            s.module = crate::Module::Tasks;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+4"),
        CommandSpec::new("task.list", "Tasks", "Tasks", |s, v| {
            let list: Vec<Value> = s.tasks(p::str(v, "query").unwrap_or("")).iter().map(|r| task_json(&r.account, &r.task)).collect();
            Ok(json!({"tasks": list}))
        })
        .params(r#"{"query"?: string (words of its title, notes or categories)}"#),
        CommandSpec::new("task.create", "New task", "Tasks", |s, v| {
            let t = task_from(s, v, Task::default())?;
            s.create_task(p::str(v, "account"), t)
        })
        .params(TASK_PARAMS)
        .when(has_account),
        CommandSpec::new("task.update", "Edit task", "Tasks", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let before = s.find_task(&uid).ok_or_else(|| CmdError::Params(format!("no task `{uid}`")))?.task;
            let t = task_from(s, v, before)?;
            s.update_task(&uid, |x| *x = t)
        })
        .params(r#"{"uid": string, … as task.create (what's not given stays)}"#)
        .when(has_task),
        CommandSpec::new("task.complete", "Mark complete", "Tasks", |s, v| {
            s.complete_task(p::req_str(v, "uid")?, p::bool(v, "done").unwrap_or(true))
        })
        .params(r#"{"uid": string, "done"?: bool (default true; false takes it back up)}"#)
        .when(has_task),
        CommandSpec::new("task.delete", "Delete task", "Tasks", |s, v| s.delete_task(p::req_str(v, "uid")?))
            .params(r#"{"uid": string}"#)
            .when(has_task),
        CommandSpec::new("task.arrange", "Arrange tasks", "Tasks › Arrange by", |s, v| {
            let by = p::req_str(v, "by")?;
            s.prefs.tasks_order =
                TasksOrder::from_code(by).ok_or_else(|| CmdError::Params(format!("`by` is due, priority, title or modified, not `{by}`")))?;
            Ok(json!({"by": s.prefs.tasks_order.code()}))
        })
        .params(r#"{"by": "due"|"priority"|"title"|"modified"}"#),
        CommandSpec::new("task.hideDone", "Hide completed tasks", "Tasks › View", |s, v| {
            s.prefs.tasks_hide_done = p::bool(v, "on").unwrap_or(!s.prefs.tasks_hide_done);
            Ok(json!({"on": s.prefs.tasks_hide_done}))
        })
        .params(r#"{"on"?: bool (default: the other way)}"#),
    ]
}

/// A day from params: `YYYY-MM-DD` (or a date and time: its local day), `""` or null for none;
/// `Err` for anything else. `None` when not given.
fn day_param(s: &Session, v: &Value, k: &str) -> Result<Option<Option<i64>>, CmdError> {
    match v.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(Value::String(x)) if x.trim().is_empty() || x.trim() == "none" => Ok(Some(None)),
        Some(Value::String(x)) => match parse_local(x, &s.clock) {
            Some((t, true)) => Ok(Some(Some(t))),
            Some((t, false)) => Ok(Some(Some(s.clock.local(t).div_euclid(86_400).saturating_mul(86_400)))),
            None => Err(CmdError::Params(format!("`{x}` is not a date (YYYY-MM-DD)"))),
        },
        Some(_) => Err(CmdError::Params(format!("`{k}` is a date (YYYY-MM-DD)"))),
    }
}

/// A task from command params, starting from `t` (what's not given stays).
pub(super) fn task_from(s: &Session, v: &Value, mut t: Task) -> Result<Task, CmdError> {
    if let Some(x) = p::str(v, "title") {
        t.title = x.chars().take(1_000).collect();
    }
    if let Some(x) = p::str(v, "notes") {
        t.notes = x.chars().take(100_000).collect();
    }
    if let Some(d) = day_param(s, v, "start")? {
        t.start = d;
    }
    if let Some(d) = day_param(s, v, "due")? {
        t.due = d;
    }
    if let Some(x) = p::str(v, "priority") {
        t.priority = TaskPriority::from_code(x).ok_or_else(|| CmdError::Params(format!("`{x}` is not low, normal or high")))?;
    }
    if let Some(n) = v.get("percent") {
        let n = n.as_u64().filter(|n| *n <= 100).ok_or_else(|| CmdError::Params("`percent` is 0 to 100".into()))?;
        t.percent = u8::try_from(n).unwrap_or(100);
        // Less than all of it: not done, whatever it was.
        if t.percent < 100 && t.is_done() {
            t.status = if t.percent > 0 { TaskStatus::InProgress } else { TaskStatus::NotStarted };
        }
    }
    if let Some(x) = p::str(v, "status") {
        let status = TaskStatus::from_code(x).ok_or_else(|| CmdError::Params(format!("`{x}` is not a task status")))?;
        // Taken out of done without a percent of its own: it starts again.
        if t.is_done() && status != TaskStatus::Completed && v.get("percent").is_none() {
            t.percent = 0;
        }
        t.status = status;
    }
    let list: Option<Vec<String>> = match v.get("categories") {
        Some(Value::String(x)) => Some(x.split([',', ';']).map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect()),
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).map(str::to_string).collect()),
        _ => None,
    };
    if let Some(mut list) = list {
        list.dedup();
        t.categories = list.into_iter().take(20).map(|c| c.chars().take(100).collect()).collect();
    }
    match v.get("reminder") {
        None => {}
        Some(Value::Null) => t.reminder = None,
        Some(Value::String(x)) if matches!(x.trim(), "" | "none") => t.reminder = None,
        Some(Value::String(x)) => match parse_local(x, &s.clock) {
            // A day alone reminds at nine in its morning, as Outlook's task reminders do by default.
            Some((day, true)) => t.reminder = Some(s.clock.utc(day.saturating_add(9 * 3600))),
            Some((at, false)) => t.reminder = Some(at),
            None => return Err(CmdError::Params(format!("`{x}` is not a date and time (YYYY-MM-DDTHH:MM)"))),
        },
        Some(_) => return Err(CmdError::Params("`reminder` is a date and time (YYYY-MM-DDTHH:MM) or none".into())),
    }
    match p::str(v, "repeat").map(str::trim) {
        Some("" | "none" | "never") => t.rrule = None,
        Some(r @ ("daily" | "weekly" | "monthly" | "yearly")) => t.rrule = Some(format!("FREQ={}", r.to_ascii_uppercase())),
        Some(r) if r.to_ascii_uppercase().starts_with("FREQ=") && r.len() < 500 && !r.contains(['\r', '\n']) => t.rrule = Some(r.to_string()),
        Some(r) => return Err(CmdError::Params(format!("`{r}` is not a repeat"))),
        None => {}
    }
    Ok(t)
}
