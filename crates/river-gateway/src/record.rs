//! The turn record (wall ch. 10): `record/turns.jsonl`, one stream
//! for the whole life, append-only, one JSON object per line, written
//! by exactly one writer, fsynced on append. Every context message is
//! persisted at the moment it enters the context, exactly once, under
//! its turn number and tagged with the channel it concerns
//! (persist-once, wall ch. 01). Readers skip torn lines with a
//! warning — a crash mid-append never poisons the file.

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

use crate::jsonl_index::{JsonlIndex, Refresh, ensure_append_target};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecordRole {
    User,
    Assistant,
    Tool,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecordLine {
    pub id: String,
    pub turn: u64,
    pub channel: String,
    pub role: RecordRole,
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_calls: Option<Vec<crate::model::ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_call_id: Option<String>,
}

/// The single writer for the agent's turn record.
pub struct TurnRecord {
    path: PathBuf,
    file: File,
}

impl TurnRecord {
    pub fn open(workspace: &Path) -> anyhow::Result<Self> {
        let dir = workspace.join("record");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join("turns.jsonl");
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self { path, file })
    }

    /// Append one line and fsync. Returns the line's ULID.
    pub fn append(
        &mut self,
        turn: u64,
        channel: &str,
        role: RecordRole,
        content: Option<&str>,
    ) -> anyhow::Result<String> {
        self.append_full(turn, channel, role, content, None, None)
    }

    /// The full line shape (wall ch. 10): tool calls on assistant
    /// lines, tool_call_id on tool lines.
    pub fn append_full(
        &mut self,
        turn: u64,
        channel: &str,
        role: RecordRole,
        content: Option<&str>,
        tool_calls: Option<Vec<crate::model::ToolCall>>,
        tool_call_id: Option<String>,
    ) -> anyhow::Result<String> {
        ensure_append_target(&mut self.file, &self.path)?;
        let line = RecordLine {
            id: ulid::Ulid::new().to_string(),
            turn,
            channel: channel.to_string(),
            role,
            content: content.map(str::to_string),
            tool_calls,
            tool_call_id,
        };
        let mut json = serde_json::to_string(&line)?;
        json.push('\n');
        let before = self.file.metadata()?;
        self.file
            .write_all(json.as_bytes())
            .with_context(|| format!("appending to {}", self.path.display()))?;
        self.file
            .sync_data()
            .with_context(|| format!("fsyncing {}", self.path.display()))?;
        let after = self.file.metadata()?;
        note_record_append(&self.path, &line, json.as_bytes(), &before, &after);
        Ok(line.id)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

struct RecordIndex {
    file: JsonlIndex<RecordLine>,
    by_turn: BTreeMap<u64, Vec<RecordLine>>,
    by_channel: HashMap<String, std::collections::BTreeSet<u64>>,
}

impl RecordIndex {
    fn new(path: PathBuf) -> Self {
        Self {
            file: JsonlIndex::new(path, "record"),
            by_turn: BTreeMap::new(),
            by_channel: HashMap::new(),
        }
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        match self.file.refresh()? {
            Refresh::Unchanged => {}
            Refresh::Rebuilt => {
                self.by_turn.clear();
                self.by_channel.clear();
                for line in self.file.items() {
                    self.by_channel
                        .entry(line.channel.clone())
                        .or_default()
                        .insert(line.turn);
                    self.by_turn
                        .entry(line.turn)
                        .or_default()
                        .push(line.clone());
                }
            }
        }
        Ok(())
    }
}

fn record_indexes() -> &'static Mutex<HashMap<PathBuf, RecordIndex>> {
    static INDEXES: OnceLock<Mutex<HashMap<PathBuf, RecordIndex>>> = OnceLock::new();
    INDEXES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_record_index<R>(path: &Path, read: impl FnOnce(&RecordIndex) -> R) -> anyhow::Result<R> {
    let mut indexes = record_indexes().lock().expect("record indexes lock");
    let index = indexes
        .entry(path.to_path_buf())
        .or_insert_with(|| RecordIndex::new(path.to_path_buf()));
    index.refresh()?;
    Ok(read(index))
}

fn note_record_append(
    path: &Path,
    line: &RecordLine,
    serialized: &[u8],
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
) {
    let mut indexes = record_indexes().lock().expect("record indexes lock");
    let Some(index) = indexes.get_mut(path) else {
        return;
    };
    let keep = match index
        .file
        .apply_known_append(line.clone(), serialized, before, after)
    {
        Ok(true) => {
            index
                .by_channel
                .entry(line.channel.clone())
                .or_default()
                .insert(line.turn);
            index
                .by_turn
                .entry(line.turn)
                .or_default()
                .push(line.clone());
            true
        }
        Ok(false) => false,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "record index append failed; invalidating");
            false
        }
    };
    if !keep {
        indexes.remove(path);
    }
}

/// Read a record file through its incremental index, preserving
/// physical file order and torn-line tolerance.
#[cfg_attr(not(test), allow(dead_code))]
pub fn scan(path: &Path) -> anyhow::Result<Vec<RecordLine>> {
    with_record_index(path, |index| index.file.items().to_vec())
}

/// Read one turn without walking unrelated record entries.
pub fn scan_turn(path: &Path, turn: u64) -> anyhow::Result<Vec<RecordLine>> {
    with_record_index(path, |index| {
        index.by_turn.get(&turn).cloned().unwrap_or_default()
    })
}

/// Read an inclusive turn range in chronological turn order while
/// preserving physical order within each turn.
pub fn scan_turn_range(path: &Path, first: u64, last: u64) -> anyhow::Result<Vec<RecordLine>> {
    if first > last {
        return Ok(Vec::new());
    }
    with_record_index(path, |index| {
        index
            .by_turn
            .range(first..=last)
            .flat_map(|(_, lines)| lines.iter().cloned())
            .collect()
    })
}

/// Distinct recorded turns through `last`, already sorted.
pub fn turn_numbers_through(path: &Path, last: u64) -> anyhow::Result<Vec<u64>> {
    with_record_index(path, |index| {
        index
            .by_turn
            .range(..=last)
            .map(|(&turn, _)| turn)
            .collect()
    })
}

/// Whole turns touching `channel`, in chronological turn order, from
/// one index snapshot.
pub fn scan_channel_turns(
    path: &Path,
    channel: &str,
) -> anyhow::Result<Vec<(u64, Vec<RecordLine>)>> {
    with_record_index(path, |index| {
        index
            .by_channel
            .get(channel)
            .map(|turns| {
                turns
                    .iter()
                    .filter_map(|turn| index.by_turn.get(turn).map(|lines| (*turn, lines.clone())))
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// The physical record tail, used for channel resume derivation.
pub fn tail(path: &Path) -> anyhow::Result<Option<RecordLine>> {
    with_record_index(path, |index| index.file.items().last().cloned())
}

/// The highest turn number in the record, or 0 for none.
pub fn last_turn(path: &Path) -> anyhow::Result<u64> {
    with_record_index(path, |index| {
        index.by_turn.keys().next_back().copied().unwrap_or(0)
    })
}

/// A witness move (wall ch. 04): one line per turn in
/// `record/moves.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MoveLine {
    pub id: String,
    pub turn: u64,
    pub summary: String,
}

pub fn moves_path(workspace: &Path) -> PathBuf {
    workspace.join("record").join("moves.jsonl")
}

/// The single writer for the moves file (the witness's, wall ch. 04).
pub struct MovesFile {
    path: PathBuf,
    file: File,
}

impl MovesFile {
    pub fn open(workspace: &Path) -> anyhow::Result<Self> {
        let path = moves_path(workspace);
        std::fs::create_dir_all(path.parent().expect("moves path has a parent"))?;
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self { path, file })
    }

    /// Append one move line and fsync. The file stays append-only
    /// even when the witness backfills a gap, so regenerated moves
    /// can land out of turn order — readers sort by turn.
    pub fn append(&mut self, turn: u64, summary: &str) -> anyhow::Result<String> {
        ensure_append_target(&mut self.file, &self.path)?;
        let line = MoveLine {
            id: ulid::Ulid::new().to_string(),
            turn,
            summary: summary.to_string(),
        };
        let mut json = serde_json::to_string(&line)?;
        json.push('\n');
        let before = self.file.metadata()?;
        self.file
            .write_all(json.as_bytes())
            .with_context(|| format!("appending to {}", self.path.display()))?;
        self.file
            .sync_data()
            .with_context(|| format!("fsyncing {}", self.path.display()))?;
        let after = self.file.metadata()?;
        note_move_append(&self.path, &line, json.as_bytes(), &before, &after);
        Ok(line.id)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

struct MoveIndex {
    file: JsonlIndex<MoveLine>,
    by_turn: BTreeMap<u64, Vec<MoveLine>>,
}

impl MoveIndex {
    fn new(path: PathBuf) -> Self {
        Self {
            file: JsonlIndex::new(path, "move"),
            by_turn: BTreeMap::new(),
        }
    }

    fn refresh(&mut self) -> anyhow::Result<()> {
        match self.file.refresh()? {
            Refresh::Unchanged => {}
            Refresh::Rebuilt => {
                self.by_turn.clear();
                for move_line in self.file.items() {
                    self.by_turn
                        .entry(move_line.turn)
                        .or_default()
                        .push(move_line.clone());
                }
            }
        }
        Ok(())
    }
}

fn move_indexes() -> &'static Mutex<HashMap<PathBuf, MoveIndex>> {
    static INDEXES: OnceLock<Mutex<HashMap<PathBuf, MoveIndex>>> = OnceLock::new();
    INDEXES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_move_index<R>(path: &Path, read: impl FnOnce(&MoveIndex) -> R) -> anyhow::Result<R> {
    let mut indexes = move_indexes().lock().expect("move indexes lock");
    let index = indexes
        .entry(path.to_path_buf())
        .or_insert_with(|| MoveIndex::new(path.to_path_buf()));
    index.refresh()?;
    Ok(read(index))
}

fn note_move_append(
    path: &Path,
    line: &MoveLine,
    serialized: &[u8],
    before: &std::fs::Metadata,
    after: &std::fs::Metadata,
) {
    let mut indexes = move_indexes().lock().expect("move indexes lock");
    let Some(index) = indexes.get_mut(path) else {
        return;
    };
    let keep = match index
        .file
        .apply_known_append(line.clone(), serialized, before, after)
    {
        Ok(true) => {
            index
                .by_turn
                .entry(line.turn)
                .or_default()
                .push(line.clone());
            true
        }
        Ok(false) => false,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "move index append failed; invalidating");
            false
        }
    };
    if !keep {
        indexes.remove(path);
    }
}

/// All moves in physical file order, torn lines skipped.
#[cfg_attr(not(test), allow(dead_code))]
pub fn read_moves(path: &Path) -> anyhow::Result<Vec<MoveLine>> {
    with_move_index(path, |index| index.file.items().to_vec())
}

/// Moves in an inclusive turn range, sorted by turn while preserving
/// append order for duplicate entries at the same turn.
pub fn read_moves_range(path: &Path, first: u64, last: u64) -> anyhow::Result<Vec<MoveLine>> {
    if first > last {
        return Ok(Vec::new());
    }
    with_move_index(path, |index| {
        index
            .by_turn
            .range(first..=last)
            .flat_map(|(_, moves)| moves.iter().cloned())
            .collect()
    })
}

/// All moves in chronological turn order, preserving append order for
/// duplicates at the same turn.
pub fn read_moves_chronological(path: &Path) -> anyhow::Result<Vec<MoveLine>> {
    with_move_index(path, |index| {
        index
            .by_turn
            .values()
            .flat_map(|moves| moves.iter().cloned())
            .collect()
    })
}

/// Distinct move turn numbers, already sorted.
pub fn move_turns(path: &Path) -> anyhow::Result<Vec<u64>> {
    with_move_index(path, |index| index.by_turn.keys().copied().collect())
}

/// The witness cursor: the highest turn T such that every turn from
/// the file's first move through T has a move — the contiguous
/// compression frontier (wall chs. 03, 04). For an untouched file
/// this is the tail; a hand-deleted middle line pulls the cursor back
/// to just before the gap, so compaction cannot drop the uncompressed
/// turns and the witness regenerates them. No moves — the cursor is 0.
pub fn witness_cursor(path: &Path) -> anyhow::Result<u64> {
    let turns = move_turns(path)?;
    let Some(&first) = turns.first() else {
        return Ok(0);
    };
    let mut frontier = first;
    for &turn in &turns[1..] {
        if turn != frontier + 1 {
            break;
        }
        frontier = turn;
    }
    Ok(frontier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_scans_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path()).unwrap();
        record
            .append(
                1,
                "local_main",
                RecordRole::User,
                Some("[local_main] cass: hello"),
            )
            .unwrap();
        record
            .append(1, "local_main", RecordRole::Assistant, Some("hi"))
            .unwrap();

        let lines = scan(record.path()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].turn, 1);
        assert_eq!(lines[0].channel, "local_main");
        assert_eq!(lines[0].role, RecordRole::User);
        assert_eq!(lines[1].role, RecordRole::Assistant);
        // ULIDs order across milliseconds; within one, file order is
        // the truth — which is why readers never sort by id.
        assert_eq!(lines[0].id.len(), 26);
    }

    #[test]
    fn one_stream_holds_many_channels() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path()).unwrap();
        record
            .append(1, "discord_general", RecordRole::User, Some("a"))
            .unwrap();
        record
            .append(1, "local_main", RecordRole::Assistant, Some("b"))
            .unwrap();

        let lines = scan(record.path()).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.turn == 1), "one turn, one place");
    }

    #[test]
    fn torn_line_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path()).unwrap();
        record
            .append(1, "local_main", RecordRole::User, Some("a"))
            .unwrap();
        // Simulate a crash mid-append.
        {
            use std::io::Write;
            let mut f = OpenOptions::new()
                .append(true)
                .open(record.path())
                .unwrap();
            f.write_all(b"{\"id\":\"torn").unwrap();
            f.write_all(b"\n").unwrap();
        }
        let mut record = TurnRecord::open(dir.path()).unwrap();
        record
            .append(2, "local_main", RecordRole::User, Some("b"))
            .unwrap();

        let lines = scan(record.path()).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].turn, 2);
    }

    #[test]
    fn last_turn_reads_the_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut record = TurnRecord::open(dir.path()).unwrap();
        assert_eq!(last_turn(record.path()).unwrap(), 0);
        record
            .append(41, "local_main", RecordRole::User, Some("x"))
            .unwrap();
        assert_eq!(last_turn(record.path()).unwrap(), 41);
    }

    #[test]
    fn witness_cursor_is_the_contiguous_frontier() {
        let dir = tempfile::tempdir().unwrap();
        let mut moves = MovesFile::open(dir.path()).unwrap();
        assert_eq!(witness_cursor(moves.path()).unwrap(), 0, "no moves");
        for turn in [5u64, 6, 7] {
            moves.append(turn, "m").unwrap();
        }
        assert_eq!(witness_cursor(moves.path()).unwrap(), 7, "contiguous: the tail");

        // A gap (hand edit, or a backfill not yet done): the cursor
        // stops before it, whatever the file order says.
        moves.append(9, "m").unwrap();
        assert_eq!(witness_cursor(moves.path()).unwrap(), 7, "gap at 8");

        // The backfilled move closes the gap from the tail.
        moves.append(8, "m").unwrap();
        assert_eq!(witness_cursor(moves.path()).unwrap(), 9, "frontier recovered");
    }

    #[test]
    fn missing_file_scans_empty() {
        let dir = tempfile::tempdir().unwrap();
        let lines = scan(&dir.path().join("nope.jsonl")).unwrap();
        assert!(lines.is_empty());
    }

    #[test]
    fn cached_moves_rebuild_after_delete_and_place_regeneration_by_turn() {
        let dir = tempfile::tempdir().unwrap();
        let mut moves = MovesFile::open(dir.path()).unwrap();
        moves.append(1, "one").unwrap();
        moves.append(2, "two").unwrap();
        moves.append(3, "three").unwrap();

        // Prime both the physical and turn-keyed views.
        assert_eq!(read_moves(moves.path()).unwrap().len(), 3);
        assert_eq!(witness_cursor(moves.path()).unwrap(), 3);

        let text = std::fs::read_to_string(moves.path()).unwrap();
        let kept: Vec<_> = text
            .lines()
            .filter(|line| !line.contains("\"turn\":2"))
            .collect();
        std::fs::write(moves.path(), format!("{}\n", kept.join("\n"))).unwrap();
        assert_eq!(move_turns(moves.path()).unwrap(), vec![1, 3]);
        assert_eq!(witness_cursor(moves.path()).unwrap(), 1);

        // Regeneration appends physically after turn 3, but the turn
        // index restores chronological reads and the contiguous front.
        moves.append(2, "two, regenerated").unwrap();
        let chronological = read_moves_chronological(moves.path()).unwrap();
        assert_eq!(
            chronological
                .iter()
                .map(|line| line.turn)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(chronological[1].summary, "two, regenerated");
        assert_eq!(witness_cursor(moves.path()).unwrap(), 3);
    }

    #[test]
    fn duplicate_moves_preserve_append_order_but_not_cursor_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let mut moves = MovesFile::open(dir.path()).unwrap();
        moves.append(1, "one").unwrap();
        moves.append(2, "first two").unwrap();
        moves.append(2, "second two").unwrap();
        moves.append(3, "three").unwrap();

        let twos = read_moves_range(moves.path(), 2, 2).unwrap();
        assert_eq!(
            twos.iter()
                .map(|line| line.summary.as_str())
                .collect::<Vec<_>>(),
            vec!["first two", "second two"]
        );
        assert_eq!(witness_cursor(moves.path()).unwrap(), 3);
    }

    #[test]
    fn move_writer_reopens_after_file_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let mut moves = MovesFile::open(dir.path()).unwrap();
        moves.append(1, "old inode").unwrap();
        assert_eq!(read_moves(moves.path()).unwrap().len(), 1, "index primed");

        let replacement = moves.path().with_extension("replacement");
        std::fs::write(&replacement, "").unwrap();
        std::fs::rename(&replacement, moves.path()).unwrap();
        moves.append(2, "visible replacement").unwrap();

        let visible = read_moves(moves.path()).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].turn, 2);
        assert_eq!(visible[0].summary, "visible replacement");
    }
}
