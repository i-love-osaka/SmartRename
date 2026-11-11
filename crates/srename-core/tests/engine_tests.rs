use srename_core::ops::*;
use srename_core::*;
use std::fs;
use std::path::{Path, PathBuf};

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("srename-test-{}-{}", std::process::id(), tag));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn touch(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    fs::write(&p, b"hello").unwrap();
    p
}

#[test]
fn case_and_extension() {
    let dir = tmpdir("case");
    let f = touch(&dir, "Report.TXT");
    let pipeline = vec![
        Operation::Case {
            name: CaseKind::Lower,
            ext: CaseKind::Lower,
        },
        Operation::ChangeExtension {
            new_ext: Some("md".into()),
        },
    ];
    let items = preview(&[f], &pipeline);
    assert_eq!(items[0].new_name, "report.md");
    assert_eq!(items[0].status, PreviewStatus::Ready);
}

#[test]
fn regex_replace_and_examples() {
    let dir = tmpdir("regex");
    let f = touch(&dir, "001 - My Song.mp3");
    let pipeline = vec![Operation::Replace {
        find: r"^\d+[\s._-]*".into(),
        replacement: "".into(),
        use_regex: true,
        case_sensitive: true,
        include_ext: false,
    }];
    let items = preview(&[f], &pipeline);
    assert_eq!(items[0].new_name, "My Song.mp3");

    let f2 = touch(&dir, "2024-1-2 note.txt");
    let pipeline2 = vec![Operation::Replace {
        find: r"(\d{4})-(\d)-(\d)".into(),
        replacement: "$1-0$2-0$3".into(),
        use_regex: true,
        case_sensitive: true,
        include_ext: false,
    }];
    let items2 = preview(&[f2], &pipeline2);
    assert_eq!(items2[0].new_name, "2024-01-02 note.txt");
}

#[test]
fn numbering_numeric_and_alpha() {
    let dir = tmpdir("num");
    let a = touch(&dir, "a.txt");
    let b = touch(&dir, "b.txt");
    let pipeline = vec![Operation::Numbering {
        start: 1,
        step: 1,
        width: 3,
        style: NumberStyle::Numeric,
        placement: NumberPlacement::Prefix,
        template: String::new(),
    }];
    let items = preview(&[a, b], &pipeline);
    assert_eq!(items[0].new_name, "001a.txt");
    assert_eq!(items[1].new_name, "002b.txt");

    let pipeline_alpha = vec![Operation::Numbering {
        start: 1,
        step: 1,
        width: 0,
        style: NumberStyle::AlphaUpper,
        placement: NumberPlacement::Template,
        template: "ep-<#>".into(),
    }];
    let items = preview(
        &[touch(&dir, "x.txt"), touch(&dir, "y.txt")],
        &pipeline_alpha,
    );
    assert_eq!(items[0].new_name, "ep-A.txt");
    assert_eq!(items[1].new_name, "ep-B.txt");
}

#[test]
fn insert_delete_and_scripts() {
    let dir = tmpdir("edit");
    let f = touch(&dir, "abcdef.txt");
    let pipeline = vec![
        Operation::DeleteRange { start: 2, count: 2 },
        Operation::InsertAt {
            position: 2,
            text: "XY".into(),
        },
    ];
    let items = preview(std::slice::from_ref(&f), &pipeline);
    assert_eq!(items[0].new_name, "aXYdef.txt");

    let rev = vec![Operation::Script {
        preset: ScriptPreset::Reverse,
        target: ScriptTarget::Stem,
    }];
    let items = preview(&[f], &rev);
    assert_eq!(items[0].new_name, "fedcba.txt");
}

#[test]
fn pinyin_and_simplified() {
    let dir = tmpdir("pinyin");
    let f = touch(&dir, "中文名.txt");
    let pipeline = vec![Operation::Script {
        preset: ScriptPreset::ToPinyin,
        target: ScriptTarget::Stem,
    }];
    let items = preview(&[f], &pipeline);
    assert_eq!(items[0].new_name, "zhongwenming.txt");

    let g = touch(&dir, "繁體.txt");
    let pipeline2 = vec![Operation::Encoding {
        kind: EncodingConvert::TraditionalToSimplified,
    }];
    let items2 = preview(&[g], &pipeline2);
    assert_eq!(items2[0].new_name, "繁体.txt");
}

#[test]
fn conflict_detection() {
    let dir = tmpdir("conflict");
    let a = touch(&dir, "same.txt");
    let b = touch(&dir, "other.txt");
    // Rename both to the same target via template numbering is unique, so
    // use a plain replace that collapses both stems.
    let pipeline = vec![Operation::Numbering {
        start: 1,
        step: 0,
        width: 1,
        style: NumberStyle::Numeric,
        placement: NumberPlacement::Template,
        template: "fixed".into(),
    }];
    let items = preview(&[a, b], &pipeline);
    assert!(matches!(items[0].status, PreviewStatus::Conflict(_)));
    assert!(matches!(items[1].status, PreviewStatus::Conflict(_)));
}

#[test]
fn execute_and_undo_roundtrip() {
    let dir = tmpdir("exec");
    let a = touch(&dir, "one.txt");
    let b = touch(&dir, "two.txt");
    let pipeline = vec![Operation::AddPrefix { text: "x-".into() }];
    let items = preview(&[a.clone(), b.clone()], &pipeline);
    let (results, undo_log) = execute(&items, &ExecuteOptions::default());
    assert!(results.iter().all(|r| r.success));
    assert!(dir.join("x-one.txt").exists());
    assert!(dir.join("x-two.txt").exists());

    let undo_results = undo(&undo_log);
    assert!(undo_results.iter().all(|r| r.success));
    assert!(a.exists() && b.exists());
}

#[test]
fn pipeline_json_roundtrip() {
    let pipeline = vec![
        Operation::AddPrefix { text: "p".into() },
        Operation::Numbering {
            start: 5,
            step: 2,
            width: 2,
            style: NumberStyle::Numeric,
            placement: NumberPlacement::Suffix,
            template: String::new(),
        },
    ];
    let dir = tmpdir("json");
    let path = dir.join("ops.json");
    save_pipeline(&pipeline, &path).unwrap();
    let loaded = load_pipeline(&path).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].describe(), pipeline[0].describe());
}

#[test]
fn collect_paths_skips_hidden() {
    let dir = tmpdir("hidden");
    touch(&dir, "visible.txt");
    touch(&dir, ".hidden.txt");
    touch(&dir, ".DS_Store");
    let sub = dir.join("sub");
    fs::create_dir_all(&sub).unwrap();
    touch(&sub, "ok.txt");
    touch(&sub, ".inner-hidden.txt");
    let hdir = dir.join(".hdir");
    fs::create_dir_all(&hdir).unwrap();
    touch(&hdir, "inner.txt");

    let names: Vec<String> = collect_paths(&dir, true, true)
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"visible.txt".to_string()));
    assert!(names.contains(&"ok.txt".to_string()));
    assert!(names.contains(&"sub".to_string()));
    assert!(
        !names.iter().any(|n| n.starts_with('.')),
        "hidden leaked: {:?}",
        names
    );
    assert!(
        !names.contains(&"inner.txt".to_string()),
        "hidden dir was walked"
    );
    assert_eq!(names.len(), 3);
}

#[test]
fn set_times_only_pipeline() {
    // A pipeline that only changes file times: the name stays the same,
    // so the item is Unchanged — but execute() must still apply the
    // times in place. (Regression: the app counted only Ready items,
    // so a times-only run looked like "nothing to do".)
    let dir = tmpdir("times");
    let f = touch(&dir, "photo.jpg");
    let target = 1_600_000_000i64; // 2020-09-13
    let pipeline = vec![Operation::SetTimes {
        spec: TimeSpec {
            modified_unix: Some(target),
            accessed_unix: Some(target),
            created_unix: None,
        },
    }];
    let items = preview(std::slice::from_ref(&f), &pipeline);
    assert_eq!(items[0].status, PreviewStatus::Unchanged);
    assert!(items[0].times.is_some());
    let (results, _log) = execute(
        &items,
        &ExecuteOptions {
            preserve_timestamps: false,
        },
    );
    assert!(results.is_empty(), "no renames should happen");
    let md = fs::metadata(&f).unwrap();
    let mtime = md
        .modified()
        .unwrap()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert_eq!(mtime, target);
}

#[test]
fn cleanup_and_styles() {
    let dir = tmpdir("cleanup");
    let f = touch(&dir, "My Song (1) [www.example.com].mp3");
    let items = preview(&[f], &[Operation::CleanupName]);
    assert_eq!(items[0].new_name, "My Song.mp3");

    let g = touch(&dir, "报告 - 副本.pdf");
    let items = preview(&[g], &[Operation::CleanupName]);
    assert_eq!(items[0].new_name, "报告.pdf");

    let h = touch(&dir, "ｆｉｌｅ　ｎａｍｅ.txt");
    let items = preview(&[h], &[Operation::CleanupName]);
    assert_eq!(items[0].new_name, "file name.txt");

    let s = touch(&dir, "my-http_file name.rs");
    let snake = preview(
        std::slice::from_ref(&s),
        &[Operation::NameStyle {
            style: NameStyle::Snake,
        }],
    );
    assert_eq!(snake[0].new_name, "my_http_file_name.rs");
    let camel = preview(
        std::slice::from_ref(&s),
        &[Operation::NameStyle {
            style: NameStyle::Camel,
        }],
    );
    assert_eq!(camel[0].new_name, "myHttpFileName.rs");
    let pascal = preview(
        std::slice::from_ref(&s),
        &[Operation::NameStyle {
            style: NameStyle::Pascal,
        }],
    );
    assert_eq!(pascal[0].new_name, "MyHttpFileName.rs");
    let kebab = preview(
        &[s],
        &[Operation::NameStyle {
            style: NameStyle::Kebab,
        }],
    );
    assert_eq!(kebab[0].new_name, "my-http-file-name.rs");
}

#[test]
fn name_from_parent() {
    let dir = tmpdir("parent");
    let sub = dir.join("旅行照片");
    fs::create_dir_all(&sub).unwrap();
    let f = touch(&sub, "IMG_0001.jpg");
    let items = preview(
        std::slice::from_ref(&f),
        &[Operation::NameFromParent { numbered: false }],
    );
    assert_eq!(items[0].new_name, "旅行照片.jpg");
    let items = preview(&[f], &[Operation::NameFromParent { numbered: true }]);
    assert_eq!(items[0].new_name, "旅行照片_001.jpg");
}

/// Minimal JPEG carrying one EXIF field: DateTimeOriginal
/// "2024:08:15 14:30:22". TIFF layout: IFD0 at 8 with one entry
/// (ExifIFD pointer -> 26); ExifIFD at 26 with one entry (tag 0x9003,
/// ASCII x20 -> string at 44).
fn jpeg_with_exif_datetime() -> Vec<u8> {
    let mut tiff: Vec<u8> = Vec::new();
    tiff.extend_from_slice(b"II");
    tiff.extend_from_slice(&42u16.to_le_bytes());
    tiff.extend_from_slice(&8u32.to_le_bytes()); // IFD0 offset
    tiff.extend_from_slice(&1u16.to_le_bytes()); // IFD0: 1 entry
    tiff.extend_from_slice(&0x8769u16.to_le_bytes()); // ExifIFD pointer
    tiff.extend_from_slice(&4u16.to_le_bytes()); // LONG
    tiff.extend_from_slice(&1u32.to_le_bytes());
    tiff.extend_from_slice(&26u32.to_le_bytes()); // -> ExifIFD at 26
    tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD
    tiff.extend_from_slice(&1u16.to_le_bytes()); // ExifIFD: 1 entry
    tiff.extend_from_slice(&0x9003u16.to_le_bytes()); // DateTimeOriginal
    tiff.extend_from_slice(&2u16.to_le_bytes()); // ASCII
    tiff.extend_from_slice(&20u32.to_le_bytes());
    tiff.extend_from_slice(&44u32.to_le_bytes()); // -> string at 44
    tiff.extend_from_slice(&0u32.to_le_bytes()); // next IFD
    tiff.extend_from_slice(b"2024:08:15 14:30:22\0");
    assert_eq!(tiff.len(), 64);

    let mut jpg: Vec<u8> = Vec::new();
    jpg.extend_from_slice(&[0xFF, 0xD8]); // SOI
    jpg.extend_from_slice(&[0xFF, 0xE1]); // APP1
    let len = (2 + 6 + tiff.len()) as u16;
    jpg.extend_from_slice(&len.to_be_bytes());
    jpg.extend_from_slice(b"Exif\0\0");
    jpg.extend_from_slice(&tiff);
    jpg.extend_from_slice(&[0xFF, 0xD9]); // EOI
    jpg
}

#[test]
fn date_name_exif_and_modified() {
    let dir = tmpdir("datename");
    let photo = dir.join("IMG_9999.jpg");
    fs::write(&photo, jpeg_with_exif_datetime()).unwrap();
    let items = preview(
        std::slice::from_ref(&photo),
        &[Operation::DateName {
            source: DateSource::Exif,
            format: DateFormat::Compact,
            mode: DateMode::Replace,
        }],
    );
    assert_eq!(items[0].new_name, "20240815_143022.jpg");
    let items = preview(
        &[photo],
        &[Operation::DateName {
            source: DateSource::Exif,
            format: DateFormat::Dashed,
            mode: DateMode::Prefix,
        }],
    );
    assert_eq!(items[0].new_name, "2024-08-15_14-30-22_IMG_9999.jpg");

    // Modified-time source: set a known mtime through the engine, then
    // the expected stamp is that instant in local time.
    let f = touch(&dir, "note.txt");
    let target = 1_600_000_000i64;
    let set = preview(
        std::slice::from_ref(&f),
        &[Operation::SetTimes {
            spec: TimeSpec {
                modified_unix: Some(target),
                accessed_unix: Some(target),
                created_unix: None,
            },
        }],
    );
    execute(
        &set,
        &ExecuteOptions {
            preserve_timestamps: false,
        },
    );
    let odt = time::OffsetDateTime::from_unix_timestamp(target).unwrap();
    let off = time::UtcOffset::local_offset_at(odt).unwrap_or(time::UtcOffset::UTC);
    let local = odt.to_offset(off);
    let expected = format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}.txt",
        local.year(),
        local.month() as u32,
        local.day(),
        local.hour(),
        local.minute(),
        local.second()
    );
    let items = preview(
        &[f],
        &[Operation::DateName {
            source: DateSource::Modified,
            format: DateFormat::Compact,
            mode: DateMode::Replace,
        }],
    );
    assert_eq!(items[0].new_name, expected);
}

#[test]
fn auto_number_resolves_conflicts() {
    let dir = tmpdir("autonum");
    // Two files renamed to the same target.
    let a = touch(&dir, "ab.txt");
    let b = touch(&dir, "a b.txt");
    let pipeline = vec![Operation::Replace {
        find: " ".into(),
        replacement: "".into(),
        use_regex: false,
        case_sensitive: true,
        include_ext: false,
    }];
    let blocked = preview(&[a.clone(), b.clone()], &pipeline);
    assert!(blocked
        .iter()
        .any(|i| matches!(i.status, PreviewStatus::Conflict(_))));
    let numbered = preview_with_strategy(&[a, b], &pipeline, ConflictStrategy::AutoNumber);
    let names: Vec<&str> = numbered.iter().map(|i| i.new_name.as_str()).collect();
    assert_eq!(names, vec!["ab.txt", "ab (2).txt"]);
    // "ab.txt" itself needs no rename (Unchanged); the other is Ready.
    assert_eq!(numbered[0].status, PreviewStatus::Unchanged);
    assert_eq!(numbered[1].status, PreviewStatus::Ready);

    // Target exists on disk outside the batch.
    let e = touch(&dir, "e.txt");
    let _external = touch(&dir, "e_.txt");
    let pipeline2 = vec![Operation::AddSuffix { text: "_".into() }];
    let numbered = preview_with_strategy(&[e], &pipeline2, ConflictStrategy::AutoNumber);
    assert_eq!(numbered[0].new_name, "e_ (2).txt");
}
