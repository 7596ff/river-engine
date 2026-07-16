//! Dynamic heartbeat landscape (spec 2026-07-15).
//!
//! This module observes the workspace; it does not interpret it. Runtime
//! failures degrade to a small permission-bearing prompt, while a missing
//! workspace is an ordinary opt-out at the call site.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsStr;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

pub const WORKSPACE_PREFIX: &str = "[workspace]";
pub const CLOSING: &str = "Nothing here is a task. Pick something, start something else, or rest.";

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
struct WakeState {
    #[serde(default)]
    last_observed_head: String,
    last_run: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ThreadEntry {
    pub at: String,
    pub status: String,
}

pub type ThreadStore = BTreeMap<String, Vec<ThreadEntry>>;

#[derive(Debug)]
struct Project {
    name: String,
    why: String,
    next: Option<String>,
    modified: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ChangeKind {
    Committed,
    Uncommitted,
}

#[derive(Debug)]
struct Change {
    path: String,
    kind: ChangeKind,
    modified: SystemTime,
}

/// Generate the heartbeat landscape. Runtime failures are contained and
/// rendered as the fallback shape; only a nonexistent workspace returns None.
pub fn generate(workspace_root: &Path, state_path: &Path) -> anyhow::Result<Option<String>> {
    if !workspace_root.exists() {
        return Ok(None);
    }

    let prior = match read_state(state_path) {
        Ok(state) => state,
        Err(error) => {
            tracing::warn!(error = %error, "wake landscape generation failed");
            return Ok(Some(fallback(None)));
        }
    };

    match generate_inner(workspace_root, state_path, prior.as_ref()) {
        Ok(prompt) => Ok(Some(prompt)),
        Err(error) => {
            tracing::warn!(error = %error, "wake landscape generation failed");
            Ok(Some(fallback(
                prior.as_ref().map(|state| state.last_run.as_str()),
            )))
        }
    }
}

fn generate_inner(
    workspace_root: &Path,
    state_path: &Path,
    prior: Option<&WakeState>,
) -> anyhow::Result<String> {
    let now = jiff::Timestamp::now();
    let now_text = now.to_string();
    let mut layers = vec![render_time(prior, now)?];

    let head = git_head(workspace_root)?;
    if let (Some(state), Some(current_head)) = (prior, head.as_deref()) {
        if !state.last_observed_head.is_empty() {
            if let Some(changes) =
                render_changes(workspace_root, &state.last_observed_head, current_head)?
            {
                layers.push(changes);
            }
        }
    }

    if let Some(projects) = render_projects(workspace_root)? {
        layers.push(projects);
    }
    if let Some(threads) = render_threads(workspace_root)? {
        layers.push(threads);
    }
    // External signals are deliberately stubbed for v1.
    debug_assert!(render_external_signals().is_none());
    layers.push(CLOSING.to_string());

    let next_state = WakeState {
        last_observed_head: head
            .or_else(|| prior.map(|state| state.last_observed_head.clone()))
            .unwrap_or_default(),
        last_run: now_text,
    };
    let json = serde_json::to_vec_pretty(&next_state)?;
    write_atomic(state_path, &json)?;
    Ok(layers.join("\n\n"))
}

fn read_state(path: &Path) -> anyhow::Result<Option<WakeState>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", path.display()))
            .map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn render_time(prior: Option<&WakeState>, now: jiff::Timestamp) -> anyhow::Result<String> {
    let delta = match prior {
        Some(state) => match state.last_run.parse::<jiff::Timestamp>() {
            Ok(last) => human_duration(last.duration_until(now)),
            Err(_) => "just now".to_string(),
        },
        None => "just now".to_string(),
    };
    let zoned = now.to_zoned(jiff::tz::TimeZone::system());
    let clock = zoned.strftime("%-I:%M %p %Z").to_string();
    Ok(format!("You last settled {delta}. It's {clock}."))
}

fn human_duration(duration: jiff::SignedDuration) -> String {
    let seconds = duration.as_secs().unsigned_abs();
    human_seconds(seconds)
}

fn human_since(time: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(time)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    human_seconds(seconds)
}

fn human_seconds(seconds: u64) -> String {
    match seconds {
        0..=59 => "just now".to_string(),
        60..=3599 => plural(seconds / 60, "minute"),
        3600..=86_399 => plural(seconds / 3600, "hour"),
        86_400..=2_592_000 => plural(seconds / 86_400, "day"),
        _ => plural(seconds / 2_592_000, "month"),
    }
}

fn plural(amount: u64, unit: &str) -> String {
    let suffix = if amount == 1 { "" } else { "s" };
    format!("{amount} {unit}{suffix} ago")
}

fn git_head(workspace: &Path) -> anyhow::Result<Option<String>> {
    let probe = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(workspace)
        .output()
        .context("running git repository probe")?;
    if !probe.status.success() || String::from_utf8_lossy(&probe.stdout).trim() != "true" {
        return Ok(None);
    }
    let output = git(workspace, &["rev-parse", "HEAD"])?;
    Ok(Some(output.trim().to_string()))
}

fn git(workspace: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace)
        .output()
        .with_context(|| format!("running git {}", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git output was not UTF-8")
}

fn render_changes(workspace: &Path, baseline: &str, head: &str) -> anyhow::Result<Option<String>> {
    let committed = git(
        workspace,
        &["diff", "--name-only", "-z", &format!("{baseline}..{head}")],
    )?;
    let status = git(
        workspace,
        &["status", "--short", "--untracked-files=all", "-z"],
    )?;
    let mut seen = HashSet::new();
    let mut changes = Vec::new();
    for path in committed.split('\0').filter(|path| !path.is_empty()) {
        push_change(
            workspace,
            path,
            ChangeKind::Committed,
            &mut seen,
            &mut changes,
        );
    }
    parse_status(&status, |path| {
        push_change(
            workspace,
            path,
            ChangeKind::Uncommitted,
            &mut seen,
            &mut changes,
        )
    });
    if changes.is_empty() {
        return Ok(None);
    }

    let mut grouped: HashMap<String, Vec<Change>> = HashMap::new();
    for change in changes {
        let group = change
            .path
            .split('/')
            .next()
            .unwrap_or(&change.path)
            .to_string();
        grouped.entry(group).or_default().push(change);
    }
    let mut groups: Vec<_> = grouped.into_iter().collect();
    groups.sort_by_key(|(_, changes)| {
        std::cmp::Reverse(
            changes
                .iter()
                .map(|c| c.modified)
                .max()
                .unwrap_or(SystemTime::UNIX_EPOCH),
        )
    });

    let mut lines = vec!["Changed since last wake:".to_string()];
    for (group, changes) in groups {
        let is_directory = changes.iter().any(|change| change.path.contains('/'));
        if !is_directory {
            for change in changes {
                lines.push(format!("  {} ({})", change.path, kind_label(change.kind)));
            }
            continue;
        }
        for kind in [ChangeKind::Committed, ChangeKind::Uncommitted] {
            let mut paths: Vec<_> = changes
                .iter()
                .filter(|change| change.kind == kind)
                .map(|change| change.path.as_str())
                .collect();
            if paths.is_empty() {
                continue;
            }
            paths.sort_unstable();
            if paths.len() > 3 {
                lines.push(format!(
                    "  {} files in {group}/ ({})",
                    paths.len(),
                    kind_label(kind)
                ));
            } else if paths.len() == 1 {
                lines.push(format!("  {} ({})", paths[0], kind_label(kind)));
            } else {
                let names = paths
                    .iter()
                    .map(|path| path.strip_prefix(&format!("{group}/")).unwrap_or(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("  {group}/: {names} ({})", kind_label(kind)));
            }
        }
    }
    Ok(Some(lines.join("\n")))
}

fn push_change(
    workspace: &Path,
    path: &str,
    kind: ChangeKind,
    seen: &mut HashSet<(String, ChangeKind)>,
    changes: &mut Vec<Change>,
) {
    let path = path.trim_start_matches("./").to_string();
    if path.is_empty()
        || path == "state/landscape-generator.json"
        || !seen.insert((path.clone(), kind))
    {
        return;
    }
    let modified = std::fs::metadata(workspace.join(&path))
        .and_then(|metadata| metadata.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    changes.push(Change {
        path,
        kind,
        modified,
    });
}

fn parse_status(mut status: &str, mut visit: impl FnMut(&str)) {
    while !status.is_empty() {
        let Some((entry, rest)) = status.split_once('\0') else {
            break;
        };
        status = rest;
        if entry.len() < 4 {
            continue;
        }
        let code = &entry[..2];
        let path = &entry[3..];
        visit(path);
        if code.contains('R') || code.contains('C') {
            // Porcelain v1 -z emits the destination first, then the old path.
            if let Some((_, rest)) = status.split_once('\0') {
                status = rest;
            } else {
                break;
            }
        }
    }
}

fn kind_label(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Committed => "since last wake",
        ChangeKind::Uncommitted => "uncommitted",
    }
}

fn render_projects(workspace: &Path) -> anyhow::Result<Option<String>> {
    let dir = workspace.join("projects");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", dir.display())),
    };
    let mut projects = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension() != Some(OsStr::new("md")) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        match parse_project(&text) {
            Ok(Some((name, why, next))) => projects.push(Project {
                name,
                why,
                next,
                modified: entry.metadata()?.modified()?,
            }),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(path = %path.display(), error = %error, "skipping project")
            }
        }
    }
    if projects.is_empty() {
        return Ok(None);
    }
    projects.sort_by_key(|project| std::cmp::Reverse(project.modified));
    let mut lines = vec!["Active projects:".to_string()];
    for project in projects {
        let separator = if project.why.ends_with(['.', '!', '?']) {
            ""
        } else {
            "."
        };
        lines.push(format!(
            "  {} — {}{} last touched {}. next: {}.",
            project.name,
            project.why,
            separator,
            human_since(project.modified),
            project
                .next
                .as_deref()
                .filter(|next| !next.trim().is_empty())
                .unwrap_or("—")
        ));
    }
    Ok(Some(lines.join("\n")))
}

fn parse_project(text: &str) -> anyhow::Result<Option<(String, String, Option<String>)>> {
    let normalized = text.replace("\r\n", "\n");
    let Some(rest) = normalized.strip_prefix("---\n") else {
        anyhow::bail!("missing YAML frontmatter");
    };
    let Some((frontmatter, body)) = rest.split_once("\n---\n") else {
        anyhow::bail!("unterminated YAML frontmatter");
    };
    if body.lines().any(is_tombstone) {
        return Ok(None);
    }
    let mut fields = HashMap::new();
    for line in frontmatter.lines() {
        if line.starts_with(char::is_whitespace) || line.trim().is_empty() || line.starts_with('#')
        {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            anyhow::bail!("malformed frontmatter line {line:?}");
        };
        fields.insert(key.trim(), unquote(value.trim()));
    }
    let name = fields
        .get("name")
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing required frontmatter field name"))?;
    let why = fields
        .get("why")
        .filter(|v| !v.is_empty())
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("missing required frontmatter field why"))?;
    let next = fields.get("next").cloned();
    Ok(Some((name, why, next)))
}

fn unquote(value: &str) -> String {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_string()
    } else {
        value.to_string()
    }
}

fn is_tombstone(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("dissolved ") else {
        return false;
    };
    let bytes = rest.as_bytes();
    bytes.len() >= 11
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b':'
        && bytes[..4]
            .iter()
            .chain(&bytes[5..7])
            .chain(&bytes[8..10])
            .all(u8::is_ascii_digit)
}

fn render_threads(workspace: &Path) -> anyhow::Result<Option<String>> {
    let mut threads = live_threads(&load_threads(&threads_path(workspace))?);
    if threads.is_empty() {
        return Ok(None);
    }
    threads.sort_by(|a, b| b.1.at.cmp(&a.1.at).then_with(|| a.0.cmp(&b.0)));
    let mut lines = vec!["Live threads:".to_string()];
    lines.extend(
        threads
            .into_iter()
            .map(|(slug, entry)| format!("  {slug} — {}.", entry.status)),
    );
    Ok(Some(lines.join("\n")))
}

fn render_external_signals() -> Option<String> {
    None
}

fn fallback(last_run: Option<&str>) -> String {
    format!(
        "You last settled at {}.\n\nThe landscape generator encountered an error and could not render the full map.\n\n{CLOSING}",
        last_run.unwrap_or("unknown")
    )
}

pub fn threads_path(workspace: &Path) -> PathBuf {
    workspace.join("projects").join("threads.json")
}

pub fn load_threads(path: &Path) -> anyhow::Result<ThreadStore> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

pub fn live_threads(store: &ThreadStore) -> Vec<(String, ThreadEntry)> {
    store
        .iter()
        .filter_map(|(slug, history)| {
            history
                .last()
                .filter(|entry| entry.status != "done")
                .map(|entry| (slug.clone(), entry.clone()))
        })
        .collect()
}

pub fn save_threads(path: &Path, store: &ThreadStore) -> anyhow::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(store)?)
}

/// Tempfile + fsync + rename. The tempfile lives beside the destination so
/// the rename is atomic on the destination filesystem.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{} has no parent", path.display()))?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating tempfile in {}", parent.display()))?;
    temp.write_all(bytes)
        .with_context(|| format!("writing tempfile for {}", path.display()))?;
    temp.as_file()
        .sync_all()
        .with_context(|| format!("fsyncing tempfile for {}", path.display()))?;
    temp.persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("renaming tempfile to {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn git_ok(workspace: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(workspace)
            .status()
            .unwrap();
        assert!(status.success(), "git {:?}", args);
    }

    fn init_git(workspace: &Path) {
        git_ok(workspace, &["init", "-q"]);
        git_ok(
            workspace,
            &["config", "user.email", "river@example.invalid"],
        );
        git_ok(workspace, &["config", "user.name", "River Test"]);
        git_ok(workspace, &["config", "commit.gpgsign", "false"]);
        write(&workspace.join("base.txt"), "base");
        git_ok(workspace, &["add", "."]);
        git_ok(workspace, &["commit", "-qm", "base"]);
    }

    #[test]
    fn first_wake_omits_layer_2_and_records_head_and_creates_parent() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        write(&dir.path().join("uncommitted.txt"), "new");
        let state = dir.path().join("state/landscape-generator.json");
        let prompt = generate(dir.path(), &state).unwrap().unwrap();
        assert!(!prompt.contains("Changed since last wake:"), "{prompt}");
        let saved: WakeState = serde_json::from_slice(&std::fs::read(state).unwrap()).unwrap();
        assert!(!saved.last_observed_head.is_empty());
    }

    #[test]
    fn change_group_boundary_lists_three_and_collapses_four() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        let state = dir.path().join("state/landscape-generator.json");
        generate(dir.path(), &state).unwrap();
        for i in 0..3 {
            write(&dir.path().join(format!("three/{i}.md")), "x");
        }
        for i in 0..4 {
            write(&dir.path().join(format!("four/{i}.md")), "x");
        }
        let prompt = generate(dir.path(), &state).unwrap().unwrap();
        assert!(
            prompt.contains("three/: 0.md, 1.md, 2.md (uncommitted)"),
            "{prompt}"
        );
        assert!(
            prompt.contains("4 files in four/ (uncommitted)"),
            "{prompt}"
        );
    }

    #[test]
    fn committed_and_uncommitted_changes_split_and_rename_delete_are_bare() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        write(&dir.path().join("mixed/old.md"), "old");
        write(&dir.path().join("mixed/delete.md"), "delete");
        git_ok(dir.path(), &["add", "."]);
        git_ok(dir.path(), &["commit", "-qm", "tracked files"]);
        let state = dir.path().join("state/landscape-generator.json");
        generate(dir.path(), &state).unwrap();

        write(&dir.path().join("mixed/committed.md"), "committed");
        git_ok(dir.path(), &["add", "mixed/committed.md"]);
        git_ok(dir.path(), &["commit", "-qm", "committed change"]);
        git_ok(dir.path(), &["mv", "mixed/old.md", "mixed/new.md"]);
        git_ok(dir.path(), &["rm", "-q", "mixed/delete.md"]);
        write(&dir.path().join("mixed/uncommitted.md"), "uncommitted");

        let prompt = generate(dir.path(), &state).unwrap().unwrap();
        assert!(
            prompt.contains("committed.md (since last wake)"),
            "{prompt}"
        );
        assert!(
            prompt.contains("new.md"),
            "rename destination only: {prompt}"
        );
        assert!(!prompt.contains("old.md"), "rename source hidden: {prompt}");
        assert!(
            prompt.contains("delete.md"),
            "deletion remains a bare path: {prompt}"
        );
        assert!(
            !prompt.contains(" -> ") && !prompt.contains(" D "),
            "no operation markers: {prompt}"
        );
        assert!(
            prompt.contains("(uncommitted)"),
            "source labels remain split: {prompt}"
        );
    }

    #[test]
    fn unchanged_head_and_tree_omit_changes_and_non_git_is_not_an_error() {
        let git_dir = tempfile::tempdir().unwrap();
        init_git(git_dir.path());
        let state = git_dir.path().join("state/landscape-generator.json");
        generate(git_dir.path(), &state).unwrap();
        let clean = generate(git_dir.path(), &state).unwrap().unwrap();
        assert!(!clean.contains("Changed since last wake:"), "{clean}");

        let plain = tempfile::tempdir().unwrap();
        let prompt = generate(plain.path(), &plain.path().join("state/s.json"))
            .unwrap()
            .unwrap();
        assert!(!prompt.contains("Changed since last wake:"), "{prompt}");
        assert!(prompt.ends_with(CLOSING));
    }

    #[test]
    fn project_tombstone_is_line_anchored_and_missing_next_is_dash() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("projects/gone.md"),
            "---\nname: Gone\nwhy: old\n---\ndissolved 2026-07-15: done\n",
        );
        write(
            &dir.path().join("projects/live.md"),
            "---\nname: Live\nwhy: contains dissolved marker\n---\nnot dissolved 2026-07-15: mid-line\n",
        );
        let prompt = generate(dir.path(), &dir.path().join("state/s.json"))
            .unwrap()
            .unwrap();
        assert!(!prompt.contains("Gone —"), "{prompt}");
        assert!(
            prompt.contains("Live — contains dissolved marker. last touched"),
            "{prompt}"
        );
        assert!(prompt.contains("next: —."), "{prompt}");
    }

    #[test]
    fn malformed_project_is_skipped_without_triggering_fallback() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("projects/bad.md"),
            "---\nname: Bad\nnever closed",
        );
        write(
            &dir.path().join("projects/good.md"),
            "---\nname: Good\nwhy: valid\nnext: continue\n---\nbody\n",
        );
        let prompt = generate(dir.path(), &dir.path().join("state/s.json"))
            .unwrap()
            .unwrap();
        assert!(prompt.contains("Good — valid"), "{prompt}");
        assert!(!prompt.contains("Bad —"), "{prompt}");
        assert!(!prompt.contains("encountered an error"), "{prompt}");
    }

    #[test]
    fn live_threads_latest_only_sorted_and_done_hidden() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("projects/threads.json"),
            r#"{
          "older": [{"at":"2026-07-14T00:00:00Z","status":"first"},{"at":"2026-07-15T00:00:00Z","status":"latest"}],
          "newer": [{"at":"2026-07-15T01:00:00Z","status":"fresh"}],
          "hidden": [{"at":"2026-07-15T02:00:00Z","status":"done"}]
        }"#,
        );
        let prompt = generate(dir.path(), &dir.path().join("state/s.json"))
            .unwrap()
            .unwrap();
        let newer = prompt.find("newer — fresh").unwrap();
        let older = prompt.find("older — latest").unwrap();
        assert!(newer < older, "{prompt}");
        assert!(!prompt.contains("first"), "{prompt}");
        assert!(!prompt.contains("hidden"), "{prompt}");
    }

    #[test]
    fn bad_git_baseline_returns_fallback_and_preserves_state() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        let state = dir.path().join("state/s.json");
        let original =
            br#"{"last_observed_head":"not-a-revision","last_run":"2026-07-15T00:00:00Z"}"#;
        write_atomic(&state, original).unwrap();
        let prompt = generate(dir.path(), &state).unwrap().unwrap();
        assert!(prompt.contains("encountered an error"), "{prompt}");
        assert_eq!(std::fs::read(state).unwrap(), original);
    }

    #[test]
    fn missing_workspace_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(
            generate(&dir.path().join("missing"), &dir.path().join("s.json"))
                .unwrap()
                .is_none()
        );
    }
}
