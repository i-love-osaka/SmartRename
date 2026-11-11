//! Preview -> check -> execute -> undo, plus operation-record (pipeline)
//! persistence in JSON format.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ops::{OpContext, Operation, TimeSpec};
use crate::{file_name_string, split_name};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewStatus {
    /// Will be renamed.
    Ready,
    /// Name does not change.
    Unchanged,
    /// This file cannot be processed.
    Error(String),
    /// The target name clashes with another file.
    Conflict(String),
}

#[derive(Debug, Clone)]
pub struct PreviewItem {
    pub old_path: PathBuf,
    pub new_path: PathBuf,
    pub new_name: String,
    pub is_dir: bool,
    pub status: PreviewStatus,
    /// Time changes requested by the pipeline (last SetTimes wins).
    pub times: Option<TimeSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoPair {
    /// Path after execution (where the file is now).
    pub current: PathBuf,
    /// Path before execution.
    pub original: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UndoLog {
    pub pairs: Vec<UndoPair>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExecuteOptions {
    /// Restore each file's original atime/mtime after renaming.
    pub preserve_timestamps: bool,
}

#[derive(Debug, Clone)]
pub struct ItemResult {
    pub path: PathBuf,
    pub success: bool,
    pub message: String,
}

/// An operation pipeline ("operation record") that can be saved/loaded.
pub type Pipeline = Vec<Operation>;

pub fn save_pipeline(pipeline: &Pipeline, path: &Path) -> Result<(), String> {
    let json = serde_json::to_string_pretty(pipeline).map_err(|e| e.to_string())?;
    std::fs::write(path, json).map_err(|e| format!("保存失败: {}", e))
}

pub fn load_pipeline(path: &Path) -> Result<Pipeline, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("读取失败: {}", e))?;
    serde_json::from_str(&text).map_err(|e| format!("解析失败: {}", e))
}

/// Collect files (and optionally folders) under a directory, sorted.
/// True when the path's file name is a hidden (dot) name, e.g. ".DS_Store".
/// Hidden files and folders are never collected into the rename list.
pub fn is_hidden_path(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

pub fn collect_paths(dir: &Path, recursive: bool, include_dirs: bool) -> Vec<PathBuf> {
    fn walk(dir: &Path, recursive: bool, include_dirs: bool, out: &mut Vec<PathBuf>) {
        let mut entries: Vec<PathBuf> = match std::fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(_) => return,
        };
        entries.sort();
        for path in entries {
            if is_hidden_path(&path) {
                continue;
            }
            let is_dir = path.is_dir();
            if is_dir {
                if include_dirs {
                    out.push(path.clone());
                }
                if recursive {
                    walk(&path, recursive, include_dirs, out);
                }
            } else {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, recursive, include_dirs, &mut out);
    out
}

/// What to do when two files would end up with the same name (or the
/// target already exists on disk).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStrategy {
    /// Mark the item as Conflict and let the user sort it out.
    Block,
    /// Append " (2)", " (3)", ... to the stem until the name is free.
    AutoNumber,
}

/// Run the pipeline over the given paths and compute the preview,
/// including conflict detection.
pub fn preview(paths: &[PathBuf], pipeline: &[Operation]) -> Vec<PreviewItem> {
    preview_with_strategy(paths, pipeline, ConflictStrategy::Block)
}

pub fn preview_with_strategy(
    paths: &[PathBuf],
    pipeline: &[Operation],
    strategy: ConflictStrategy,
) -> Vec<PreviewItem> {
    let mut items: Vec<PreviewItem> = Vec::with_capacity(paths.len());
    for (index, path) in paths.iter().enumerate() {
        let is_dir = path.is_dir();
        let old_name = file_name_string(path);
        let mut parts = split_name(&old_name);
        let ctx = OpContext {
            index,
            source_path: path,
            is_dir,
        };
        let mut status = PreviewStatus::Ready;
        let mut times: Option<TimeSpec> = None;
        for op in pipeline {
            if let Operation::SetTimes { spec } = op {
                times = Some(*spec);
            }
            if let Err(e) = op.apply(&mut parts, &ctx) {
                status = PreviewStatus::Error(e);
                break;
            }
        }
        let new_name = parts.full_name();
        if status == PreviewStatus::Ready {
            if parts.stem.is_empty() {
                status = PreviewStatus::Error("文件名为空".to_string());
            } else if new_name == old_name {
                status = PreviewStatus::Unchanged;
            }
        }
        let new_path = match path.parent() {
            Some(parent) => parent.join(&new_name),
            None => PathBuf::from(&new_name),
        };
        items.push(PreviewItem {
            old_path: path.clone(),
            new_path,
            new_name,
            is_dir,
            status,
            times,
        });
    }

    // Conflict detection (skipped for items already in Error state).
    // 1) Duplicate targets inside the batch (compared case-insensitively,
    //    because macOS/Windows default file systems are case-insensitive).
    let mut target_counts: HashMap<String, usize> = HashMap::new();
    for item in &items {
        if item.status == PreviewStatus::Ready {
            *target_counts
                .entry(item.new_path.to_string_lossy().to_lowercase())
                .or_insert(0) += 1;
        }
    }
    // Sources that will move away during this run.
    let moving_sources: std::collections::HashSet<String> = items
        .iter()
        .filter(|i| i.status == PreviewStatus::Ready)
        .map(|i| i.old_path.to_string_lossy().to_lowercase())
        .collect();
    for item in &mut items {
        if item.status != PreviewStatus::Ready {
            continue;
        }
        let key = item.new_path.to_string_lossy().to_lowercase();
        if target_counts.get(&key).copied().unwrap_or(0) > 1 {
            item.status = PreviewStatus::Conflict("多个文件改名后重名".to_string());
            continue;
        }
        // 2) Target already exists on disk and is not moving away itself.
        if item.new_path != item.old_path
            && item.new_path.exists()
            && !moving_sources.contains(&key)
        {
            item.status = PreviewStatus::Conflict("目标文件已存在".to_string());
        }
    }
    if strategy == ConflictStrategy::AutoNumber {
        resolve_conflicts(&mut items);
    }
    items
}

/// AutoNumber resolution: walk the items in order, keep a set of the
/// targets already handed out, and renumber any Ready/Conflict item
/// whose target is taken (by the batch or by a file staying on disk).
fn resolve_conflicts(items: &mut [PreviewItem]) {
    use std::collections::HashSet;
    let moving: HashSet<String> = items
        .iter()
        .filter(|i| i.status == PreviewStatus::Ready)
        .map(|i| i.old_path.to_string_lossy().to_lowercase())
        .collect();
    let mut assigned: HashSet<String> = HashSet::new();
    for item in items.iter_mut() {
        if !matches!(
            item.status,
            PreviewStatus::Ready | PreviewStatus::Conflict(_)
        ) {
            continue;
        }
        let base = item.new_path.clone();
        let mut candidate = base.clone();
        let mut n = 2usize;
        loop {
            let key = candidate.to_string_lossy().to_lowercase();
            let taken = assigned.contains(&key)
                || (candidate != item.old_path && candidate.exists() && !moving.contains(&key));
            if !taken || n > 999 {
                break;
            }
            let fname = file_name_string(&base);
            let parts = split_name(&fname);
            let numbered = format!("{} ({})", parts.stem, n);
            let full = match &parts.ext {
                Some(e) => format!("{}.{}", numbered, e),
                None => numbered,
            };
            candidate = match base.parent() {
                Some(p) => p.join(full),
                None => PathBuf::from(full),
            };
            n += 1;
        }
        if candidate != item.new_path {
            item.new_name = file_name_string(&candidate);
            item.new_path = candidate;
        }
        item.status = PreviewStatus::Ready;
        assigned.insert(item.new_path.to_string_lossy().to_lowercase());
    }
}

struct Stamp {
    atime: filetime::FileTime,
    mtime: filetime::FileTime,
}

fn read_stamp(path: &Path) -> Option<Stamp> {
    let md = std::fs::metadata(path).ok()?;
    let mtime = md
        .modified()
        .ok()
        .map(filetime::FileTime::from_system_time)
        .unwrap_or_else(filetime::FileTime::now);
    let atime = md
        .accessed()
        .ok()
        .map(filetime::FileTime::from_system_time)
        .unwrap_or(mtime);
    Some(Stamp { atime, mtime })
}

fn apply_times(path: &Path, spec: &TimeSpec, fallback: Option<&Stamp>) {
    let stamp = read_stamp(path);
    let mtime = spec
        .modified_unix
        .map(|s| filetime::FileTime::from_unix_time(s, 0))
        .or_else(|| stamp.as_ref().map(|s| s.mtime))
        .unwrap_or_else(filetime::FileTime::now);
    let atime = spec
        .accessed_unix
        .map(|s| filetime::FileTime::from_unix_time(s, 0))
        .or_else(|| fallback.map(|s| s.atime))
        .or_else(|| stamp.as_ref().map(|s| s.atime))
        .unwrap_or(mtime);
    let _ = filetime::set_file_times(path, atime, mtime);
    if let Some(created) = spec.created_unix {
        set_birthtime(path, created);
    }
}

/// Set the creation (birth) time. macOS supports it through
/// setattrlist(ATTR_CMN_CRTIME); the value must not be later than the
/// file's modification time, so callers set mtime first. Other
/// platforms have no portable API — silently skipped there.
#[cfg(target_os = "macos")]
fn set_birthtime(path: &Path, secs: i64) {
    use std::os::unix::ffi::OsStrExt;
    #[repr(C)]
    struct AttrList {
        bitmapcount: u16,
        reserved: u16,
        commonattr: u32,
        volattr: u32,
        dirattr: u32,
        fileattr: u32,
        forkattr: u32,
    }
    const ATTR_CMN_CRTIME: u32 = 0x0000_0200;
    let mut attrlist = AttrList {
        bitmapcount: 5,
        reserved: 0,
        commonattr: ATTR_CMN_CRTIME,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };
    let ts = libc::timespec {
        tv_sec: secs as libc::time_t,
        tv_nsec: 0,
    };
    let Ok(cpath) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return;
    };
    unsafe {
        libc::setattrlist(
            cpath.as_ptr(),
            &mut attrlist as *mut AttrList as *mut libc::c_void,
            &ts as *const libc::timespec as *mut libc::c_void,
            std::mem::size_of::<libc::timespec>(),
            0,
        );
    }
}

#[cfg(not(target_os = "macos"))]
fn set_birthtime(_path: &Path, _secs: i64) {}

/// Execute the renames for all Ready items, in two phases through
/// temporary names so chains and case-only changes are safe.
/// Returns per-item results and an undo log of what actually moved.
pub fn execute(items: &[PreviewItem], options: &ExecuteOptions) -> (Vec<ItemResult>, UndoLog) {
    let mut results: Vec<ItemResult> = Vec::new();
    let mut undo_log = UndoLog::default();

    let ready: Vec<&PreviewItem> = items
        .iter()
        .filter(|i| i.status == PreviewStatus::Ready)
        .collect();

    // Capture timestamps before anything moves.
    let stamps: HashMap<PathBuf, Stamp> = if options.preserve_timestamps {
        ready
            .iter()
            .filter_map(|i| read_stamp(&i.old_path).map(|s| (i.old_path.clone(), s)))
            .collect()
    } else {
        HashMap::new()
    };

    // Phase 1: source -> temp.
    let pid = std::process::id();
    let mut temps: Vec<(&PreviewItem, PathBuf)> = Vec::new();
    let mut failed: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for (n, item) in ready.iter().enumerate() {
        let dir = item.old_path.parent().unwrap_or_else(|| Path::new("."));
        let temp = dir.join(format!(".srename-tmp-{}-{}", pid, n));
        match std::fs::rename(&item.old_path, &temp) {
            Ok(()) => temps.push((item, temp)),
            Err(e) => {
                failed.insert(item.old_path.clone());
                results.push(ItemResult {
                    path: item.old_path.clone(),
                    success: false,
                    message: format!("改名失败: {}", e),
                });
            }
        }
    }
    // Phase 2: temp -> target.
    for (item, temp) in &temps {
        match std::fs::rename(temp, &item.new_path) {
            Ok(()) => {
                if let Some(stamp) = stamps.get(&item.old_path) {
                    let _ = filetime::set_file_times(&item.new_path, stamp.atime, stamp.mtime);
                }
                results.push(ItemResult {
                    path: item.old_path.clone(),
                    success: true,
                    message: "成功".to_string(),
                });
                undo_log.pairs.push(UndoPair {
                    current: item.new_path.clone(),
                    original: item.old_path.clone(),
                });
            }
            Err(e) => {
                // Roll this file back to its original name.
                let _ = std::fs::rename(temp, &item.old_path);
                failed.insert(item.old_path.clone());
                results.push(ItemResult {
                    path: item.old_path.clone(),
                    success: false,
                    message: format!("改名失败（已回滚）: {}", e),
                });
            }
        }
    }

    // Apply requested file-time changes to every processed item
    // (renamed items at their new path, others in place).
    for item in items {
        if let Some(spec) = &item.times {
            let target = if item.status == PreviewStatus::Ready && !failed.contains(&item.old_path)
            {
                item.new_path.clone()
            } else if item.status == PreviewStatus::Unchanged {
                item.old_path.clone()
            } else {
                continue;
            };
            apply_times(&target, spec, stamps.get(&item.old_path));
        }
    }

    (results, undo_log)
}

/// Undo a previous execution: rename everything back, also in two phases.
pub fn undo(log: &UndoLog) -> Vec<ItemResult> {
    let mut results = Vec::new();
    let pid = std::process::id();
    let mut temps: Vec<(&UndoPair, PathBuf)> = Vec::new();
    for (n, pair) in log.pairs.iter().enumerate() {
        let dir = pair.current.parent().unwrap_or_else(|| Path::new("."));
        let temp = dir.join(format!(".srename-undo-{}-{}", pid, n));
        match std::fs::rename(&pair.current, &temp) {
            Ok(()) => temps.push((pair, temp)),
            Err(e) => results.push(ItemResult {
                path: pair.current.clone(),
                success: false,
                message: format!("撤消失败: {}", e),
            }),
        }
    }
    for (pair, temp) in &temps {
        match std::fs::rename(temp, &pair.original) {
            Ok(()) => results.push(ItemResult {
                path: pair.original.clone(),
                success: true,
                message: "已撤消".to_string(),
            }),
            Err(e) => {
                let _ = std::fs::rename(temp, &pair.current);
                results.push(ItemResult {
                    path: pair.current.clone(),
                    success: false,
                    message: format!("撤消失败（已回滚）: {}", e),
                });
            }
        }
    }
    results
}
