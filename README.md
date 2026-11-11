# SmartReName

A pure, fast, undoable batch file renamer — a clean-room reimplementation
of the classic 2004 Chinese freeware **拖把更名器 (SmartRename)**, rebuilt
from scratch in Rust. No original code; only its feature set, rethought.

The original was a great tool that stopped being maintained years ago —
it even opens with an error dialog you have to dismiss. This version keeps
what made it useful, drops what time has made pointless, and starts with
**zero dialogs, zero errors**.

## Features

- **Live preview** — every setting updates the preview as you type;
  conflicts and errors are flagged before anything touches disk
- **Six groups of operations**
  - *Overall* — case conversion, capitalize words, change extension,
    naming styles (camelCase / PascalCase / snake_case / kebab-case),
    one-click cleanup of download noise (`(1)`, `- 副本`, `[site.com]`,
    full-width characters), Traditional ⇄ Simplified Chinese
  - *Replace* — plain or **regular expressions** with teaching examples;
    a built-in library of 15 practical patterns lives under
    *Help → Regex Examples*, each with copy buttons
  - *Numbering* — sequence numbers with templates like `ep-<#>`
  - *Add* — prefix, suffix, insert at position
  - *Delete* — by range or by substring
  - *More* — 9 preset script commands (Chinese → pinyin, initials,
    reverse, alternating case, …), rename from parent folder,
    rename by date (EXIF / modified / created), set file times
- **Operation records** — stack operations into a pipeline, save / load
  them as JSON, replay the last run, and **undo** any executed rename
- **Safety** — two-phase execution (via temporary names), conflict
  detection with optional auto-numbering `(2)`, `(3)`, optional
  timestamp preservation, per-file failure reasons stay visible
- **Everyday polish** — drag & drop, multi-select file picker,
  name filter for long lists, sortable columns, keyboard shortcuts
  (⌘O, ⌘Z, ⇧⌘Z, Delete), settings remembered across launches,
  follows system light/dark theme, native macOS menu bar
- Hidden files are always skipped

Deliberately **not** included (things time made pointless): ID3 / RMVB
tag editing, Explorer/Finder context menus, meta-variables, free-form
scripting, and the legacy `.xto` record format.

## Build & run

Requires Rust 1.85+.

```sh
cargo run -p srename-app --release
```

On macOS, build a proper `SmartReName.app` (icon, bundle, ad-hoc
signature):

```sh
bash packaging/macos/build-app.sh
# or just double-click 编译Mac版.command in Finder
```

The bundle is ad-hoc signed and not notarized; on first launch use
right-click → Open.

## Development

```sh
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

GitHub Actions runs the core tests on Linux and the complete workspace
checks on macOS. See [CONTRIBUTING.md](CONTRIBUTING.md) before opening a
pull request.

## Project structure

- `crates/srename-core` — the platform-independent rename engine
  (pipeline, preview/conflict check, two-phase execution, undo,
  JSON operation records), with unit tests:
  `cargo test -p srename-core`
- `crates/srename-app` — the egui desktop UI (same stack as PureMap)

## Status

- **macOS first** (Apple Silicon), Windows planned — the engine is
  cross-platform
- The interface is currently Chinese; English localization is welcome

## 中文简介

SmartReName 是对 2004 年国产免费软件「拖把更名器」的净室复刻：只按功能
分析、从零用 Rust 实现，不使用原软件任何代码。只做纯粹的批量更名——
替换（含正则与教学示例库）、序号、添加、删除、扩展预设命令（汉字转
拼音等 9 个）、繁简转换、日期命名（EXIF/修改/创建时间）、按父文件夹
命名、一键净化；全程实时预览、冲突检查、可撤消，操作记录以 JSON
保存/载入/重演。原版启动弹错的老毛病在这里不存在：启动零弹窗。

## License

MIT — see [LICENSE](LICENSE).

By [Mr.Ben](https://mrben.net)
