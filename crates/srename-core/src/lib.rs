//! srename-core: cross-platform batch rename engine.
//!
//! Clean-room reimplementation of the pure-renaming behavior of
//! SmartRename （拖把更名器）, based on a feature analysis of the original
//! program. No music-tag (ID3/RMVB) features, no shell integration.

pub mod engine;
pub mod ops;

pub use engine::{
    collect_paths, execute, is_hidden_path, load_pipeline, preview, preview_with_strategy,
    save_pipeline, undo, ConflictStrategy, ExecuteOptions, ItemResult, Pipeline, PreviewItem,
    PreviewStatus, UndoLog,
};
pub use ops::{NameParts, Operation};

use std::path::Path;

/// Split a file name into stem and extension (extension without the dot).
/// Dotfiles (e.g. ".gitignore") are treated as stem-only.
pub fn split_name(file_name: &str) -> NameParts {
    if let Some(rest) = file_name.strip_prefix('.') {
        if !rest.contains('.') {
            return NameParts {
                stem: file_name.to_string(),
                ext: None,
            };
        }
    }
    match file_name.rfind('.') {
        Some(idx) if idx > 0 => NameParts {
            stem: file_name[..idx].to_string(),
            ext: Some(file_name[idx + 1..].to_string()),
        },
        _ => NameParts {
            stem: file_name.to_string(),
            ext: None,
        },
    }
}

/// Replace characters that are unsafe in file names on Windows/macOS/Linux
/// with '_', trim trailing dots/spaces, and cap the length.
pub fn sanitize_name(raw: &str) -> String {
    let mut s: String = raw
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    let trimmed = s.trim().to_string();
    trimmed.chars().take(200).collect()
}

/// The file name of a path as a String (lossy).
pub fn file_name_string(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}
