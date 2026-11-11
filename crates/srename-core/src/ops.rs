//! Rename operations. An `Operation` transforms a file name (stem + ext).
//! Operations are chained into a pipeline and applied in order, mirroring
//! the original program's "operation record" model.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::{file_name_string, split_name};

/// A file name split into stem and extension (extension without dot).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameParts {
    pub stem: String,
    pub ext: Option<String>,
}

impl NameParts {
    pub fn full_name(&self) -> String {
        match &self.ext {
            Some(ext) if !ext.is_empty() => format!("{}.{}", self.stem, ext),
            _ => self.stem.clone(),
        }
    }
}

/// Context handed to each operation while previewing one file.
pub struct OpContext<'a> {
    /// Sequence number among the files being processed (0-based).
    /// Numbering counts from this.
    pub index: usize,
    /// Source path of the file being processed.
    pub source_path: &'a Path,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseKind {
    Keep,
    Upper,
    Lower,
}

impl CaseKind {
    fn apply(&self, s: &str) -> String {
        match self {
            CaseKind::Keep => s.to_string(),
            CaseKind::Upper => s.to_uppercase(),
            CaseKind::Lower => s.to_lowercase(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WordSeparator {
    /// Words are split on spaces only.
    SpaceOnly,
    /// Words are split on any non-alphanumeric character.
    NonAlphanumeric,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberStyle {
    Numeric,
    AlphaUpper,
    AlphaLower,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumberPlacement {
    Prefix,
    Suffix,
    /// The whole stem is replaced by the template result.
    Template,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptPreset {
    /// 汉字转拼音
    ToPinyin,
    /// 拼音首字母大写
    PinyinCapitalized,
    /// 第一个汉字的拼音声母
    FirstPinyinInitial,
    /// 字符转数值 (Unicode code points, space separated)
    CharToCode,
    /// 数值转字符
    CodeToChar,
    /// 字符顺序颠倒
    Reverse,
    /// 大小写翻转
    SwapCase,
    /// 英文字母一大一小 (ABAB...)
    AltUpperFirst,
    /// 英文字母一小一大 (aBaB...)
    AltLowerFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScriptTarget {
    Stem,
    /// Stem + "." + ext as one string; the extension is re-split afterwards.
    WholeName,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncodingConvert {
    /// 繁体 -> 简体
    TraditionalToSimplified,
    /// 简体 -> 繁体
    SimplifiedToTraditional,
}

/// Set file times (unix seconds). Fields left as None are untouched.
/// Creation time is stored for completeness; most platforms cannot set it,
/// so execution silently skips it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeSpec {
    pub modified_unix: Option<i64>,
    pub accessed_unix: Option<i64>,
    /// Creation (birth) time. Settable on macOS via setattrlist;
    /// silently skipped on platforms without an API for it.
    #[serde(default)]
    pub created_unix: Option<i64>,
}

/// Where a DateName operation gets its timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DateSource {
    /// EXIF DateTimeOriginal (photos); falls back to modified time.
    Exif,
    Modified,
    Created,
}

/// How a date stamp is written into the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DateFormat {
    /// 20240815_143022
    Compact,
    /// 2024-08-15_14-30-22
    Dashed,
}

/// Where the date stamp goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DateMode {
    /// The stamp becomes the whole stem.
    Replace,
    Prefix,
    Suffix,
}

/// camelCase / PascalCase / snake_case / kebab-case conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NameStyle {
    Camel,
    Pascal,
    Snake,
    Kebab,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Operation {
    /// 整体：大小写 (name part and extension part independently).
    Case { name: CaseKind, ext: CaseKind },
    /// 整体：单词首字母大写.
    CapitalizeWords { separator: WordSeparator },
    /// 整体：更改扩展名. None or empty removes the extension.
    ChangeExtension { new_ext: Option<String> },
    /// 替换：plain text or regex ($1, $2 ... group references with regex).
    Replace {
        find: String,
        replacement: String,
        use_regex: bool,
        case_sensitive: bool,
        include_ext: bool,
    },
    /// 序号：numbering. `template` may contain "<#>" as the number slot;
    /// with placement Prefix/Suffix and an empty template the number is
    /// simply attached to the stem.
    Numbering {
        start: i64,
        step: i64,
        width: usize,
        style: NumberStyle,
        placement: NumberPlacement,
        template: String,
    },
    /// 添加：前缀.
    AddPrefix { text: String },
    /// 添加：后缀 (inserted before the extension).
    AddSuffix { text: String },
    /// 添加：在第 position 个字符处插入 (1-based, char-indexed; clamps).
    InsertAt { position: usize, text: String },
    /// 删除：从第 start 个字符起删 count 个字符 (1-based, char-indexed).
    DeleteRange { start: usize, count: usize },
    /// 删除：删除指定字串 (all occurrences, stem only).
    DeleteText { text: String, case_sensitive: bool },
    /// 扩展更名：built-in script presets.
    Script {
        preset: ScriptPreset,
        target: ScriptTarget,
    },
    /// 文件名编码/文字转换：繁简互转.
    Encoding { kind: EncodingConvert },
    /// 按日期命名：拍摄时间（EXIF）或文件时间，整名替换/前缀/后缀.
    DateName {
        source: DateSource,
        format: DateFormat,
        mode: DateMode,
    },
    /// 一键净化：去下载噪音（(1)、- 副本、[网站]）、全角转半角、合并空格.
    CleanupName,
    /// 命名风格互转：camelCase / PascalCase / snake_case / kebab-case.
    NameStyle { style: NameStyle },
    /// 以父文件夹命名（可带序号）.
    NameFromParent { numbered: bool },
    /// 更改文件日期 (does not change the name).
    SetTimes { spec: TimeSpec },
}

fn format_number(value: i64, width: usize, style: NumberStyle) -> String {
    match style {
        NumberStyle::Numeric => {
            let body = value.abs().to_string();
            let padded = if body.len() < width {
                format!("{}{}", "0".repeat(width - body.len()), body)
            } else {
                body
            };
            if value < 0 {
                format!("-{}", padded)
            } else {
                padded
            }
        }
        NumberStyle::AlphaUpper | NumberStyle::AlphaLower => {
            // Spreadsheet-style letters: 1->A, 26->Z, 27->AA. Non-positive
            // values fall back to their decimal form.
            if value < 1 {
                return value.to_string();
            }
            let mut n = value as u64;
            let mut out = String::new();
            while n > 0 {
                n -= 1;
                let ch = (b'A' + (n % 26) as u8) as char;
                out.insert(0, ch);
                n /= 26;
            }
            if style == NumberStyle::AlphaLower {
                out.to_lowercase()
            } else {
                out
            }
        }
    }
}

fn capitalize_words(s: &str, separator: WordSeparator) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at_word_start = true;
    for c in s.chars() {
        if at_word_start && c.is_alphanumeric() {
            out.extend(c.to_uppercase());
            at_word_start = false;
            continue;
        }
        out.push(c);
        at_word_start = match separator {
            WordSeparator::SpaceOnly => c == ' ',
            WordSeparator::NonAlphanumeric => !c.is_alphanumeric(),
        };
    }
    out
}

fn insert_at_char(s: &str, position_1based: usize, text: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let idx = position_1based.saturating_sub(1).min(chars.len());
    let mut out: String = chars[..idx].iter().collect();
    out.push_str(text);
    out.extend(&chars[idx..]);
    out
}

fn delete_range_chars(s: &str, start_1based: usize, count: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let start = start_1based.saturating_sub(1).min(chars.len());
    let end = (start + count).min(chars.len());
    let mut out: String = chars[..start].iter().collect();
    out.extend(&chars[end..]);
    out
}

fn pinyin_plain(c: char) -> Option<String> {
    use pinyin::ToPinyin;
    c.to_pinyin().map(|p| p.plain().to_string())
}

fn script_apply(preset: ScriptPreset, text: &str) -> Result<String, String> {
    Ok(match preset {
        ScriptPreset::Reverse => text.chars().rev().collect(),
        ScriptPreset::SwapCase => text
            .chars()
            .map(|c| {
                if c.is_uppercase() {
                    c.to_lowercase().collect::<String>()
                } else if c.is_lowercase() {
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_string()
                }
            })
            .collect(),
        ScriptPreset::AltUpperFirst | ScriptPreset::AltLowerFirst => {
            let mut upper_next = preset == ScriptPreset::AltUpperFirst;
            text.chars()
                .map(|c| {
                    if c.is_alphabetic() {
                        let out = if upper_next {
                            c.to_uppercase().collect::<String>()
                        } else {
                            c.to_lowercase().collect::<String>()
                        };
                        upper_next = !upper_next;
                        out
                    } else {
                        c.to_string()
                    }
                })
                .collect()
        }
        ScriptPreset::CharToCode => text
            .chars()
            .map(|c| (c as u32).to_string())
            .collect::<Vec<_>>()
            .join(" "),
        ScriptPreset::CodeToChar => {
            let mut out = String::new();
            for token in text.split_whitespace() {
                let code: u32 = token
                    .parse()
                    .map_err(|_| format!("无法解析的数值: {}", token))?;
                let c = char::from_u32(code).ok_or_else(|| format!("无效的字符编码: {}", code))?;
                out.push(c);
            }
            out
        }
        ScriptPreset::ToPinyin => {
            let mut out = String::new();
            for c in text.chars() {
                match pinyin_plain(c) {
                    Some(py) => out.push_str(&py),
                    None => out.push(c),
                }
            }
            out
        }
        ScriptPreset::PinyinCapitalized => {
            let mut out = String::new();
            for c in text.chars() {
                match pinyin_plain(c) {
                    Some(py) => {
                        let mut chars = py.chars();
                        if let Some(first) = chars.next() {
                            out.extend(first.to_uppercase());
                            out.extend(chars);
                        }
                    }
                    None => out.push(c),
                }
            }
            out
        }
        ScriptPreset::FirstPinyinInitial => {
            for c in text.chars() {
                if let Some(py) = pinyin_plain(c) {
                    if let Some(first) = py.chars().next() {
                        return Ok(first.to_uppercase().collect());
                    }
                }
            }
            // No Chinese character found: keep the name unchanged.
            text.to_string()
        }
    })
}

impl Operation {
    /// Apply this operation to a name. Returns an error message when the
    /// operation cannot be applied to this file (e.g. a file-only
    /// operation aimed at a folder).
    pub fn apply(&self, parts: &mut NameParts, ctx: &OpContext) -> Result<(), String> {
        match self {
            Operation::Case { name, ext } => {
                parts.stem = name.apply(&parts.stem);
                if let Some(e) = &parts.ext {
                    parts.ext = Some(ext.apply(e));
                }
            }
            Operation::CapitalizeWords { separator } => {
                parts.stem = capitalize_words(&parts.stem, *separator);
            }
            Operation::ChangeExtension { new_ext } => {
                parts.ext = match new_ext {
                    Some(e) if !e.trim().is_empty() => {
                        Some(e.trim().trim_start_matches('.').to_string())
                    }
                    _ => None,
                };
            }
            Operation::Replace {
                find,
                replacement,
                use_regex,
                case_sensitive,
                include_ext,
            } => {
                if find.is_empty() {
                    return Ok(());
                }
                let apply_to = |text: &str| -> Result<String, String> {
                    if *use_regex {
                        let pattern = if *case_sensitive {
                            find.clone()
                        } else {
                            format!("(?i){}", find)
                        };
                        let re = regex::Regex::new(&pattern)
                            .map_err(|e| format!("正则表达式无效: {}", e))?;
                        Ok(re.replace_all(text, replacement.as_str()).into_owned())
                    } else if *case_sensitive {
                        Ok(text.replace(find.as_str(), replacement))
                    } else {
                        let lower_text = text.to_lowercase();
                        let lower_find = find.to_lowercase();
                        let mut out = String::with_capacity(text.len());
                        let mut rest = text;
                        let mut lower_rest = lower_text.as_str();
                        while let Some(pos) = lower_rest.find(&lower_find) {
                            out.push_str(&rest[..pos]);
                            out.push_str(replacement);
                            rest = &rest[pos + find.len()..];
                            lower_rest = &lower_rest[pos + lower_find.len()..];
                        }
                        out.push_str(rest);
                        Ok(out)
                    }
                };
                if *include_ext {
                    let full = parts.full_name();
                    let replaced = apply_to(&full)?;
                    *parts = split_name(&replaced);
                } else {
                    parts.stem = apply_to(&parts.stem)?;
                }
            }
            Operation::Numbering {
                start,
                step,
                width,
                style,
                placement,
                template,
            } => {
                let value = start + step * ctx.index as i64;
                let num = format_number(value, *width, *style);
                if !template.is_empty() {
                    let rendered = template.replace("<#>", &num);
                    match placement {
                        NumberPlacement::Template => parts.stem = rendered,
                        NumberPlacement::Prefix => {
                            parts.stem = format!("{}{}", rendered, parts.stem)
                        }
                        NumberPlacement::Suffix => {
                            parts.stem = format!("{}{}", parts.stem, rendered)
                        }
                    }
                } else {
                    match placement {
                        NumberPlacement::Prefix | NumberPlacement::Template => {
                            parts.stem = format!("{}{}", num, parts.stem)
                        }
                        NumberPlacement::Suffix => parts.stem = format!("{}{}", parts.stem, num),
                    }
                }
            }
            Operation::AddPrefix { text } => {
                parts.stem = format!("{}{}", text, parts.stem);
            }
            Operation::AddSuffix { text } => {
                parts.stem = format!("{}{}", parts.stem, text);
            }
            Operation::InsertAt { position, text } => {
                parts.stem = insert_at_char(&parts.stem, *position, text);
            }
            Operation::DeleteRange { start, count } => {
                parts.stem = delete_range_chars(&parts.stem, *start, *count);
            }
            Operation::DeleteText {
                text,
                case_sensitive,
            } => {
                if !text.is_empty() {
                    let rep = Operation::Replace {
                        find: text.clone(),
                        replacement: String::new(),
                        use_regex: false,
                        case_sensitive: *case_sensitive,
                        include_ext: false,
                    };
                    rep.apply(parts, ctx)?;
                }
            }
            Operation::Script { preset, target } => match target {
                ScriptTarget::Stem => {
                    parts.stem = script_apply(*preset, &parts.stem)?;
                }
                ScriptTarget::WholeName => {
                    let full = parts.full_name();
                    let done = script_apply(*preset, &full)?;
                    *parts = split_name(&done);
                }
            },
            Operation::Encoding { kind } => {
                let convert = |s: &str| -> String {
                    match kind {
                        EncodingConvert::TraditionalToSimplified => {
                            zhconv::zhconv(s, zhconv::Variant::ZhHans)
                        }
                        EncodingConvert::SimplifiedToTraditional => {
                            zhconv::zhconv(s, zhconv::Variant::ZhHant)
                        }
                    }
                };
                parts.stem = convert(&parts.stem);
                if let Some(e) = &parts.ext {
                    parts.ext = Some(convert(e));
                }
            }
            Operation::DateName {
                source,
                format,
                mode,
            } => {
                let stamp = date_stamp(ctx.source_path, *source, *format)?;
                parts.stem = match mode {
                    DateMode::Replace => stamp,
                    DateMode::Prefix => format!("{}_{}", stamp, parts.stem),
                    DateMode::Suffix => format!("{}_{}", parts.stem, stamp),
                };
            }
            Operation::CleanupName => {
                let cleaned = cleanup_stem(&parts.stem);
                if cleaned.is_empty() {
                    return Err("净化后文件名为空".to_string());
                }
                parts.stem = cleaned;
            }
            Operation::NameStyle { style } => {
                parts.stem = restyle_stem(&parts.stem, *style);
            }
            Operation::NameFromParent { numbered } => {
                let parent = ctx
                    .source_path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| "没有父文件夹名".to_string())?;
                parts.stem = if *numbered {
                    format!("{}_{:03}", parent, ctx.index + 1)
                } else {
                    parent
                };
            }
            Operation::SetTimes { .. } => {
                // Name unchanged; applied at execution time.
            }
        }
        Ok(())
    }

    /// Short Chinese description, used in the operation-record list.
    pub fn describe(&self) -> String {
        match self {
            Operation::Case { name, ext } => {
                let f = |k: &CaseKind| match k {
                    CaseKind::Keep => "不变",
                    CaseKind::Upper => "大写",
                    CaseKind::Lower => "小写",
                };
                format!("大小写：名称{} / 扩展名{}", f(name), f(ext))
            }
            Operation::CapitalizeWords { separator } => match separator {
                WordSeparator::SpaceOnly => "首字母大写（按空格分词）".to_string(),
                WordSeparator::NonAlphanumeric => "首字母大写（按非字母数字分词）".to_string(),
            },
            Operation::ChangeExtension { new_ext } => match new_ext {
                Some(e) if !e.is_empty() => format!("更改扩展名为 {}", e),
                _ => "删除扩展名".to_string(),
            },
            Operation::Replace {
                find,
                replacement,
                use_regex,
                ..
            } => {
                if *use_regex {
                    format!("正则替换：{} → {}", find, replacement)
                } else {
                    format!("替换：{} → {}", find, replacement)
                }
            }
            Operation::Numbering {
                start,
                step,
                width,
                style,
                ..
            } => {
                let s = match style {
                    NumberStyle::Numeric => "数字",
                    NumberStyle::AlphaUpper => "大写字母",
                    NumberStyle::AlphaLower => "小写字母",
                };
                format!("序号：{} 起，步长 {}，{} 位，{}", start, step, width, s)
            }
            Operation::AddPrefix { text } => format!("加前缀：{}", text),
            Operation::AddSuffix { text } => format!("加后缀：{}", text),
            Operation::InsertAt { position, text } => {
                format!("在第 {} 个字符处插入：{}", position, text)
            }
            Operation::DeleteRange { start, count } => {
                format!("删除：第 {} 个字符起 {} 个", start, count)
            }
            Operation::DeleteText { text, .. } => format!("删除字串：{}", text),
            Operation::Script { preset, .. } => {
                let name = match preset {
                    ScriptPreset::ToPinyin => "汉字转拼音",
                    ScriptPreset::PinyinCapitalized => "拼音首字母大写",
                    ScriptPreset::FirstPinyinInitial => "首汉字拼音声母",
                    ScriptPreset::CharToCode => "字符转数值",
                    ScriptPreset::CodeToChar => "数值转字符",
                    ScriptPreset::Reverse => "字符顺序颠倒",
                    ScriptPreset::SwapCase => "大小写翻转",
                    ScriptPreset::AltUpperFirst => "字母一大一小",
                    ScriptPreset::AltLowerFirst => "字母一小一大",
                };
                format!("扩展命令：{}", name)
            }
            Operation::Encoding { kind } => match kind {
                EncodingConvert::TraditionalToSimplified => "繁体转简体".to_string(),
                EncodingConvert::SimplifiedToTraditional => "简体转繁体".to_string(),
            },
            Operation::DateName { source, mode, .. } => {
                let s = match source {
                    DateSource::Exif => "拍摄时间",
                    DateSource::Modified => "修改时间",
                    DateSource::Created => "创建时间",
                };
                let m = match mode {
                    DateMode::Replace => "整名",
                    DateMode::Prefix => "前缀",
                    DateMode::Suffix => "后缀",
                };
                format!("日期命名：{}（{}）", s, m)
            }
            Operation::CleanupName => "一键净化".to_string(),
            Operation::NameStyle { style } => {
                let s = match style {
                    NameStyle::Camel => "camelCase",
                    NameStyle::Pascal => "PascalCase",
                    NameStyle::Snake => "snake_case",
                    NameStyle::Kebab => "kebab-case",
                };
                format!("命名风格：{}", s)
            }
            Operation::NameFromParent { numbered } => {
                if *numbered {
                    "以父文件夹命名+序号".to_string()
                } else {
                    "以父文件夹命名".to_string()
                }
            }
            Operation::SetTimes { spec } => {
                if spec.created_unix.is_some() {
                    "更改创建时间".to_string()
                } else {
                    "更改修改时间".to_string()
                }
            }
        }
    }
}

/// The original file name of a path, for messages.
pub fn original_name(path: &Path) -> String {
    file_name_string(path)
}

// ---- Date naming -----------------------------------------------------------

struct DtParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

fn format_stamp(p: &DtParts, format: DateFormat) -> String {
    match format {
        DateFormat::Compact => format!(
            "{:04}{:02}{:02}_{:02}{:02}{:02}",
            p.year, p.month, p.day, p.hour, p.minute, p.second
        ),
        DateFormat::Dashed => format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            p.year, p.month, p.day, p.hour, p.minute, p.second
        ),
    }
}

/// EXIF DateTimeOriginal, parsed by collecting the digit runs of the
/// stored value ("2024:08:15 14:30:22" -> 2024 08 15 14 30 22). The
/// EXIF wall-clock time is already local time; no conversion applies.
fn exif_datetime(path: &Path) -> Option<DtParts> {
    let file = std::fs::File::open(path).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let exif = exif::Reader::new().read_from_container(&mut reader).ok()?;
    let field = exif.get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)?;
    let text = field.display_value().to_string();
    let runs: Vec<u32> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    if runs.len() != 6 {
        return None;
    }
    Some(DtParts {
        year: runs[0] as i32,
        month: runs[1],
        day: runs[2],
        hour: runs[3],
        minute: runs[4],
        second: runs[5],
    })
}

/// Unix seconds -> local wall-clock parts (system timezone; UTC if the
/// local offset cannot be determined).
fn epoch_to_local_parts(secs: i64) -> DtParts {
    let utc =
        time::OffsetDateTime::from_unix_timestamp(secs).unwrap_or(time::OffsetDateTime::UNIX_EPOCH);
    let offset = time::UtcOffset::local_offset_at(utc).unwrap_or(time::UtcOffset::UTC);
    let local = utc.to_offset(offset);
    DtParts {
        year: local.year(),
        month: local.month() as u32,
        day: local.day() as u32,
        hour: local.hour() as u32,
        minute: local.minute() as u32,
        second: local.second() as u32,
    }
}

fn file_time_parts(path: &Path, created: bool) -> Result<DtParts, String> {
    let md = std::fs::metadata(path).map_err(|e| format!("读取文件信息失败: {}", e))?;
    let t = if created {
        md.created()
            .map_err(|_| "该文件系统不提供创建时间".to_string())?
    } else {
        md.modified()
            .map_err(|e| format!("读取修改时间失败: {}", e))?
    };
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Ok(epoch_to_local_parts(secs))
}

fn date_stamp(path: &Path, source: DateSource, format: DateFormat) -> Result<String, String> {
    let parts = match source {
        DateSource::Exif => match exif_datetime(path) {
            Some(p) => p,
            // No usable EXIF (not a photo, or no DateTimeOriginal):
            // fall back to the file's modified time.
            None => file_time_parts(path, false)?,
        },
        DateSource::Modified => file_time_parts(path, false)?,
        DateSource::Created => file_time_parts(path, true)?,
    };
    Ok(format_stamp(&parts, format))
}

// ---- One-click cleanup -----------------------------------------------------

/// Strip download noise from a stem: full-width -> half-width, trailing
/// "(1)" / "- 副本" / " copy" suffixes, trailing [domain] brackets, and
/// collapsed whitespace.
pub fn cleanup_stem(stem: &str) -> String {
    // Full-width ASCII and ideographic space -> half-width.
    let mut s: String = stem
        .chars()
        .map(|c| match c {
            '\u{3000}' => ' ',
            c if ('\u{FF01}'..='\u{FF5E}').contains(&c) => {
                char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
            }
            c => c,
        })
        .collect();
    loop {
        let before = s.clone();
        s = s.trim_end().to_string();
        // Trailing [domain-like] bracket group, e.g. "name[abc.com]".
        if s.ends_with(']') {
            if let Some(open) = s.rfind('[') {
                let inner = &s[open + 1..s.len() - 1];
                if inner.contains('.') || inner.to_lowercase().contains("www") {
                    s.truncate(open);
                    continue;
                }
            }
        }
        // Trailing "(12)" copy counter, with or without a leading space.
        if s.ends_with(')') {
            if let Some(open) = s.rfind('(') {
                let inner = &s[open + 1..s.len() - 1];
                if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) {
                    s.truncate(open);
                    continue;
                }
            }
        }
        // Copy word suffixes.
        let lower = s.to_lowercase();
        for suf in ["- 副本", " - copy", "- copy", " copy", "副本"] {
            if lower.ends_with(suf) {
                s.truncate(s.len() - suf.len());
                break;
            }
        }
        if s == before {
            break;
        }
    }
    // Collapse whitespace runs and trim stray dots/spaces at the edges.
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.trim_matches(|c| c == ' ' || c == '.').to_string()
}

// ---- Naming styles ---------------------------------------------------------

/// Split a stem into lowercase words: separators are any non-alphanumeric
/// runs; case boundaries split too ("myHTTPFile" -> my, http, file).
fn split_words(stem: &str) -> Vec<String> {
    let chars: Vec<char> = stem.chars().collect();
    let mut words: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            let prev = chars[i - 1];
            let next_lower = chars.get(i + 1).map(|n| n.is_lowercase()).unwrap_or(false);
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower) {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.push(*c);
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words.iter().map(|w| w.to_lowercase()).collect()
}

fn capitalize(word: &str) -> String {
    let mut cs = word.chars();
    match cs.next() {
        Some(first) => first.to_uppercase().collect::<String>() + cs.as_str(),
        None => String::new(),
    }
}

pub fn restyle_stem(stem: &str, style: NameStyle) -> String {
    let words = split_words(stem);
    if words.is_empty() {
        return stem.to_string();
    }
    match style {
        NameStyle::Snake => words.join("_"),
        NameStyle::Kebab => words.join("-"),
        NameStyle::Pascal => words.iter().map(|w| capitalize(w)).collect(),
        NameStyle::Camel => {
            let mut out = words[0].clone();
            for w in &words[1..] {
                out.push_str(&capitalize(w));
            }
            out
        }
    }
}
