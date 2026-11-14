//! SmartRename desktop app (egui). macOS first, Windows later — the rename
//! engine lives in srename-core and is shared.

// On Windows, run as a GUI app: no console window behind the UI.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use srename_core::ops::{
    CaseKind, DateFormat, DateMode, DateSource, EncodingConvert, NameStyle, NumberPlacement,
    NumberStyle, Operation, ScriptPreset, ScriptTarget, TimeSpec, WordSeparator,
};
use srename_core::{
    collect_paths, execute, is_hidden_path, load_pipeline, preview_with_strategy,
    save_pipeline, undo, ConflictStrategy, ExecuteOptions, PreviewItem, PreviewStatus, UndoLog,
};

fn main() -> eframe::Result<()> {
    // Embed the app icon so the Dock/taskbar shows it even when the
    // bare binary runs (cargo run, VSCode) — without this, egui's
    // default black "e" appears. The .app bundle additionally carries
    // AppIcon.icns via the packaging script.
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
        .unwrap_or_default();
    // `mut` is only exercised by the macOS title tweak below.
    #[cfg_attr(not(target_os = "macos"), allow(unused_mut))]
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1240.0, 780.0])
        .with_min_inner_size([1020.0, 620.0])
        .with_icon(icon)
        .with_drag_and_drop(true);
    // macOS: hide the window title text — the name already sits in the
    // system menu bar (the SmartReName app menu). Keep the standard
    // title bar with its traffic-light buttons, just no text.
    #[cfg(target_os = "macos")]
    {
        viewport = viewport.with_title_shown(false);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "SmartReName",
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            let mut app = SmartRenameApp::default();
            // Restore the small set of persisted settings (theme
            // choice + toolbar toggles). Window geometry comes back
            // through egui's own persisted memory.
            let mut dark = matches!(cc.egui_ctx.system_theme(), Some(egui::Theme::Dark));
            if let Some(storage) = cc.storage {
                if let Some(s) = eframe::get_value::<Settings>(storage, SETTINGS_KEY) {
                    app.follow_system_theme = s.follow_system_theme;
                    app.recursive = s.recursive;
                    app.include_dirs = s.include_dirs;
                    app.preserve_timestamps = s.preserve_timestamps;
                    app.auto_number = s.auto_number;
                    if !s.follow_system_theme {
                        dark = s.dark_mode;
                    }
                }
            }
            apply_style(&cc.egui_ctx, dark);
            app.dark_mode = dark;
            Ok(Box::new(app))
        }),
    )
}

/// Storage key for the persisted settings (see `Settings`).
const SETTINGS_KEY: &str = "smartrename.settings";

/// The settings that survive a restart: theme choice and the four
/// toolbar toggles. Everything else is session state.
#[derive(serde::Serialize, serde::Deserialize)]
struct Settings {
    follow_system_theme: bool,
    dark_mode: bool,
    recursive: bool,
    include_dirs: bool,
    preserve_timestamps: bool,
    auto_number: bool,
}

/// Accent color used across the UI (deep blue).
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x1D, 0x4E, 0xD8);
/// Standard height for toolbar / bottom-bar buttons.
const BTN_H: f32 = 30.0;

/// Load a CJK system font so Chinese labels render properly.
/// egui's bundled fonts contain no Chinese glyphs; on macOS we use
/// PingFang / Hiragino, on Linux Noto Sans CJK. Failure is non-fatal.
fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "cjk".to_owned(),
                egui::FontData::from_owned(bytes).into(),
            );
            // CJK goes in as a FALLBACK (appended), not first: egui's own
            // fonts are metric-matched to its widgets, so Latin text and
            // digits stay properly centered in inputs and next to icons;
            // Chinese glyphs fall back to the CJK font per glyph.
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                if let Some(list) = fonts.families.get_mut(&family) {
                    list.push("cjk".to_owned());
                }
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

/// One consistent visual system: same type scale, same button padding and
/// heights everywhere, generous but uniform spacing, rounded widgets, one
/// accent color. `dark` selects the dark palette; the app follows the OS
/// theme by default and can be overridden in the header.
fn apply_style(ctx: &egui::Context, dark: bool) {
    let mut style = (*ctx.style()).clone();
    use egui::{FontFamily, FontId, TextStyle};
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::new(14.5, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::new(14.5, FontFamily::Proportional));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::new(12.5, FontFamily::Proportional));
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style.spacing.scroll.bar_width = 10.0;

    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.panel_fill = egui::Color32::from_rgb(0x1A, 0x1C, 0x20);
        visuals.window_fill = egui::Color32::from_rgb(0x1A, 0x1C, 0x20);
        visuals.extreme_bg_color = egui::Color32::from_rgb(0x10, 0x11, 0x14);
        visuals.faint_bg_color = egui::Color32::from_rgb(0x23, 0x25, 0x2A);
    } else {
        visuals.panel_fill = egui::Color32::from_rgb(0xF4, 0xF5, 0xF7);
        visuals.window_fill = egui::Color32::from_rgb(0xF4, 0xF5, 0xF7);
        visuals.extreme_bg_color = egui::Color32::WHITE;
        visuals.faint_bg_color = egui::Color32::from_rgb(0xEA, 0xEC, 0xF0);
    }
    visuals.selection.bg_fill = ACCENT;
    visuals.hyperlink_color = ACCENT;
    let radius = egui::CornerRadius::same(7);
    visuals.widgets.noninteractive.corner_radius = radius;
    visuals.widgets.inactive.corner_radius = radius;
    visuals.widgets.hovered.corner_radius = radius;
    visuals.widgets.active.corner_radius = radius;
    visuals.widgets.open.corner_radius = radius;
    visuals.window_corner_radius = egui::CornerRadius::same(12);
    style.visuals = visuals;
    ctx.set_style(style);
}

/// A hand-drawn text button: rounded rect from the widget visuals (or
/// the accent fill), label galley centered with the CJK ink correction
/// (LABEL_INK_NUDGE) so the text sits on the button's true center line.
fn text_button_enabled(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    label: &str,
    accent: bool,
    enabled: bool,
) -> bool {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    let (fill, stroke, text_color) = if accent {
        let fill = if !enabled {
            ACCENT.linear_multiply(0.45)
        } else if response.is_pointer_button_down_on() {
            ACCENT.linear_multiply(0.8)
        } else if response.hovered() {
            ACCENT.linear_multiply(1.12)
        } else {
            ACCENT
        };
        (fill, egui::Stroke::NONE, egui::Color32::WHITE)
    } else {
        // Enabled buttons get full-strength text (pure white on dark,
        // near-black on light) — the theme's regular text color read
        // as "greyed out" on the button fill.
        let tc = if enabled {
            if ui.visuals().dark_mode {
                egui::Color32::WHITE
            } else {
                egui::Color32::from_rgb(25, 25, 25)
            }
        } else {
            visuals.text_color()
        };
        (visuals.bg_fill, visuals.bg_stroke, tc)
    };
    // Laid out AFTER the color is known: the galley carries its color.
    let galley = label_galley_colored(ui, label, text_color);
    painter.rect(
        rect,
        egui::CornerRadius::same(7),
        fill,
        stroke,
        egui::StrokeKind::Inside,
    );
    let text_pos = egui::Pos2::new(
        rect.center().x - galley.size().x / 2.0,
        rect.center().y - galley.size().y / 2.0 + LABEL_INK_NUDGE,
    );
    painter.galley(text_pos, galley, text_color);
    enabled && response.clicked()
}

fn text_button(ui: &mut egui::Ui, size: egui::Vec2, label: &str, accent: bool) -> bool {
    text_button_enabled(ui, size, label, accent, true)
}

/// The standard-height, auto-width button used across the panels
/// (hand-drawn — see text_button_enabled).
fn std_btn(ui: &mut egui::Ui, text: &str) -> bool {
    let galley = label_galley(ui, text);
    let size = egui::vec2(galley.size().x + 24.0, BTN_H);
    text_button(ui, size, text, false)
}

/// The apply button at the bottom of each settings section: one fixed
/// size for every tab so the panel looks uniform. It just says 应用
/// (the section title carries the meaning) and stays grey while the
/// section has nothing armed — so it can never queue a no-op
/// operation into the record.
fn add_op_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    text_button_enabled(ui, egui::vec2(150.0, 32.0), "应用", false, enabled)
}

/// Downward shift (points) for the label of the custom drawn controls
/// (check/radio rows, text buttons). CJK fonts carry a very tall line
/// box (large hhea line gap), so egui's stock widgets center the line
/// box while the glyph ink itself rides several points too high.
/// The custom controls draw the label with this correction so ink
/// and control share one center.
const LABEL_INK_NUDGE: f32 = 3.2;

fn label_galley(ui: &egui::Ui, text: &str) -> std::sync::Arc<egui::Galley> {
    label_galley_colored(ui, text, ui.visuals().text_color())
}

/// Same, with an explicit text color. The color is baked into the
/// galley at layout time — Painter::galley's fallback color does NOT
/// override it, so any widget that wants non-default label colors
/// (white on the blue action button, full-strength text on regular
/// buttons) must lay the galley out with its final color.
fn label_galley_colored(
    ui: &egui::Ui,
    text: &str,
    color: egui::Color32,
) -> std::sync::Arc<egui::Galley> {
    let font = ui
        .style()
        .text_styles
        .get(&egui::TextStyle::Button)
        .cloned()
        .unwrap_or_else(|| egui::FontId::proportional(14.5));
    ui.fonts(|f| f.layout_no_wrap(text.to_string(), font, color))
}

/// Shared layout for the two custom labeled rows: returns the allocated
/// rect, the click response and where the label galley goes.
fn labeled_row_layout(
    ui: &mut egui::Ui,
    galley: &std::sync::Arc<egui::Galley>,
) -> (egui::Rect, egui::Response, egui::Pos2) {
    let icon = 18.0;
    let gap = 8.0;
    let height = galley.size().y.max(icon);
    let width = icon + gap + galley.size().x + 4.0;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect.expand2(egui::vec2(4.0, 1.0)),
            egui::CornerRadius::same(6),
            ui.visuals().widgets.hovered.bg_fill,
        );
    }
    let text_pos = egui::Pos2::new(
        rect.left() + icon + gap,
        rect.center().y - galley.size().y / 2.0 + LABEL_INK_NUDGE,
    );
    (rect, response, text_pos)
}

/// Checkbox with the indicator and label ink on one center line
/// (replaces egui's Checkbox for labeled rows — see LABEL_INK_NUDGE).
fn check_row(ui: &mut egui::Ui, value: &mut bool, label: &str) -> bool {
    let galley = label_galley(ui, label);
    let (rect, response, text_pos) = labeled_row_layout(ui, &galley);
    let mut changed = false;
    if response.clicked() {
        *value = !*value;
        changed = true;
    }
    let center = egui::Pos2::new(rect.left() + 9.0, rect.center().y);
    let box_rect = egui::Rect::from_center_size(center, egui::Vec2::splat(15.0));
    let painter = ui.painter();
    if *value {
        painter.rect_filled(box_rect, egui::CornerRadius::same(4), ACCENT);
        let s = egui::Stroke::new(1.8_f32, egui::Color32::WHITE);
        painter.add(egui::Shape::line(
            vec![
                egui::Pos2::new(center.x - 4.2, center.y + 0.2),
                egui::Pos2::new(center.x - 1.4, center.y + 3.4),
                egui::Pos2::new(center.x + 4.6, center.y - 3.6),
            ],
            s,
        ));
    } else {
        painter.rect(
            box_rect,
            egui::CornerRadius::same(4),
            ui.visuals().widgets.inactive.bg_fill,
            ui.visuals().widgets.inactive.bg_stroke,
            egui::StrokeKind::Inside,
        );
    }
    painter.galley(text_pos, galley, ui.visuals().text_color());
    changed
}

/// Radio button with the indicator and label ink on one center line
/// (replaces egui's RadioButton for labeled rows — see LABEL_INK_NUDGE).
fn radio_row<T: PartialEq>(
    ui: &mut egui::Ui,
    current: &mut T,
    value: T,
    label: &str,
) -> bool {
    let checked = *current == value;
    let galley = label_galley(ui, label);
    let (rect, response, text_pos) = labeled_row_layout(ui, &galley);
    let mut changed = false;
    if response.clicked() && !checked {
        *current = value;
        changed = true;
    }
    let center = egui::Pos2::new(rect.left() + 9.0, rect.center().y);
    let painter = ui.painter();
    let visuals = ui.style().interact(&response);
    painter.circle(
        center,
        8.0,
        visuals.bg_fill,
        egui::Stroke::new(1.2_f32, visuals.bg_stroke.color),
    );
    if checked {
        painter.circle_filled(center, 3.6, ACCENT);
    }
    painter.galley(text_pos, galley, ui.visuals().text_color());
    changed
}

/// A number field that is ALWAYS a text box. (egui::DragValue only
/// turns into an editor after a click.) The
/// edit buffer lives in egui temp memory: while focused it may hold
/// not-yet-parseable text ("-", ""); the value only changes on a
/// successful parse, and the buffer resyncs when focus leaves.
fn num_input_i64(ui: &mut egui::Ui, id: &str, value: &mut i64, range: std::ops::RangeInclusive<i64>) {
    let wid = egui::Id::new(("num_input", id));
    let mut text = ui
        .data_mut(|d| d.get_temp::<String>(wid))
        .unwrap_or_else(|| value.to_string());
    let resp = ui.add_sized(
        [76.0, BTN_H],
        egui::TextEdit::singleline(&mut text)
            .id(wid)
            .vertical_align(egui::Align::Center),
    );
    if resp.changed() {
        if text.trim().is_empty() {
            // Clearing the field means zero (clamped into range).
            *value = 0.clamp(*range.start(), *range.end());
        } else if let Ok(v) = text.trim().parse::<i64>() {
            *value = v.clamp(*range.start(), *range.end());
        }
    }
    if !resp.has_focus() {
        text = value.to_string();
    }
    ui.data_mut(|d| d.insert_temp(wid, text));
}

fn num_input_usize(
    ui: &mut egui::Ui,
    id: &str,
    value: &mut usize,
    range: std::ops::RangeInclusive<usize>,
) {
    let mut v = *value as i64;
    num_input_i64(
        ui,
        id,
        &mut v,
        *range.start() as i64..=*range.end() as i64,
    );
    *value = v.max(0) as usize;
}

/// The 9 script presets with their Chinese labels, in menu order.
const SCRIPT_PRESET_LABELS: [(ScriptPreset, &str); 9] = [
    (ScriptPreset::ToPinyin, "汉字转拼音"),
    (ScriptPreset::PinyinCapitalized, "拼音首字母大写"),
    (ScriptPreset::FirstPinyinInitial, "首汉字拼音声母"),
    (ScriptPreset::CharToCode, "字符转数值"),
    (ScriptPreset::CodeToChar, "数值转字符"),
    (ScriptPreset::Reverse, "字符顺序颠倒"),
    (ScriptPreset::SwapCase, "大小写翻转"),
    (ScriptPreset::AltUpperFirst, "字母一大一小"),
    (ScriptPreset::AltLowerFirst, "字母一小一大"),
];

/// Hand-drawn dropdown for the script presets. egui's stock ComboBox
/// centers the label's line box, not its ink, so CJK text floated to
/// the top of the box; this one shares the buttons' center line and
/// paints its own ▾.
fn script_preset_dropdown(ui: &mut egui::Ui, current: &mut Option<ScriptPreset>, width: f32) {
    let label = match current {
        Some(cur) => SCRIPT_PRESET_LABELS
            .iter()
            .find(|(p, _)| p == cur)
            .map(|(_, l)| *l)
            .unwrap_or(""),
        None => "请选择",
    };
    let label_color = if current.is_none() {
        ui.visuals().weak_text_color()
    } else {
        ui.visuals().text_color()
    };
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(width, BTN_H), egui::Sense::click());
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same(7),
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let galley = label_galley_colored(ui, label, label_color);
    painter.galley(
        egui::Pos2::new(
            rect.left() + 10.0,
            rect.center().y - galley.size().y / 2.0 + LABEL_INK_NUDGE,
        ),
        galley,
        label_color,
    );
    // Painted ▾ at the right end.
    let cx = rect.right() - 16.0;
    let cy = rect.center().y;
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::Pos2::new(cx - 5.0, cy - 3.0),
            egui::Pos2::new(cx + 5.0, cy - 3.0),
            egui::Pos2::new(cx, cy + 3.5),
        ],
        ui.visuals().text_color(),
        egui::Stroke::NONE,
    ));
    let popup_id = egui::Id::new("script_preset_popup");
    if response.clicked() {
        ui.memory_mut(|m| m.toggle_popup(popup_id));
    }
    egui::popup_below_widget(
        ui,
        popup_id,
        &response,
        egui::PopupCloseBehavior::CloseOnClickOutside,
        |ui| {
            ui.set_min_width(width - 16.0);
            // First entry is "请选择" (= None): choosing it disarms
            // the section, which is why there is no separate 启用 box.
            let mut options: Vec<(Option<ScriptPreset>, &str)> = vec![(None, "请选择")];
            options.extend(
                SCRIPT_PRESET_LABELS
                    .iter()
                    .map(|(p, l)| (Some(*p), *l)),
            );
            for (preset, name) in options {
                let (row, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), BTN_H),
                    egui::Sense::click(),
                );
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(
                        row,
                        egui::CornerRadius::same(6),
                        ui.visuals().widgets.hovered.bg_fill,
                    );
                }
                if preset == *current {
                    // Painted check mark, left of the label.
                    let s = egui::Stroke::new(1.8_f32, ui.visuals().text_color());
                    let c = egui::Pos2::new(row.left() + 13.0, row.center().y);
                    p.line_segment(
                        [
                            egui::Pos2::new(c.x - 4.5, c.y + 0.5),
                            egui::Pos2::new(c.x - 1.2, c.y + 3.8),
                        ],
                        s,
                    );
                    p.line_segment(
                        [
                            egui::Pos2::new(c.x - 1.2, c.y + 3.8),
                            egui::Pos2::new(c.x + 5.0, c.y - 3.8),
                        ],
                        s,
                    );
                }
                let g = label_galley_colored(ui, name, ui.visuals().text_color());
                p.galley(
                    egui::Pos2::new(
                        row.left() + 28.0,
                        row.center().y - g.size().y / 2.0 + LABEL_INK_NUDGE,
                    ),
                    g,
                    ui.visuals().text_color(),
                );
                if resp.clicked() {
                    *current = preset;
                    ui.memory_mut(|m| m.close_popup());
                }
            }
        },
    );
}

/// Miniature of the app icon, painted for the About window: blue
/// rounded square, two white documents, orange circular arrow.
fn about_icon(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(88.0, 88.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(
        rect,
        egui::CornerRadius::same(20),
        egui::Color32::from_rgb(27, 78, 196),
    );
    // Orange circular arrow (arc with a gap, arrowhead at the end).
    let orange = egui::Color32::from_rgb(245, 133, 36);
    let center = rect.center();
    let r = 34.0;
    let start = -140.0_f32.to_radians();
    let sweep = 300.0_f32.to_radians();
    let pts: Vec<egui::Pos2> = (0..=60)
        .map(|i| {
            let a = start + sweep * (i as f32 / 60.0);
            center + egui::vec2(r * a.cos(), r * a.sin())
        })
        .collect();
    p.add(egui::Shape::line(
        pts.clone(),
        egui::Stroke::new(5.5_f32, orange),
    ));
    let end = *pts.last().unwrap();
    let end_a = start + sweep;
    let tangent = egui::vec2(-end_a.sin(), end_a.cos());
    let radial = egui::vec2(end_a.cos(), end_a.sin());
    p.add(egui::Shape::convex_polygon(
        vec![
            end + tangent * 8.0,
            end - tangent * 5.0 + radial * 5.5,
            end - tangent * 5.0 - radial * 5.5,
        ],
        orange,
        egui::Stroke::NONE,
    ));
    // Documents: back one dimmer, front one white, folded corners.
    let doc = |p: &egui::Painter, origin: egui::Pos2, color: egui::Color32| {
        let d = egui::Rect::from_min_size(origin, egui::vec2(30.0, 40.0));
        p.rect_filled(d, egui::CornerRadius::same(3), color);
        let fold = egui::Rect::from_min_size(
            egui::Pos2::new(d.right() - 9.0, d.top()),
            egui::vec2(9.0, 9.0),
        );
        p.rect_filled(
            fold,
            egui::CornerRadius::same(2),
            egui::Color32::from_black_alpha(28),
        );
    };
    doc(p, rect.min + egui::vec2(34.0, 18.0), egui::Color32::from_white_alpha(170));
    doc(p, rect.min + egui::vec2(24.0, 30.0), egui::Color32::WHITE);
}

/// Small remove (×) button drawn into a given rect. The × is painted,
/// not a font glyph: "✕" is missing from the loaded font chain and
/// would render as "?".
fn remove_btn_at(ui: &mut egui::Ui, rect: egui::Rect, id: egui::Id) -> bool {
    let response = ui.interact(rect, id, egui::Sense::click());
    let visuals = ui.style().interact(&response);
    let painter = ui.painter();
    painter.rect(
        rect,
        egui::CornerRadius::same(6),
        visuals.bg_fill,
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    let c = rect.center();
    let s = egui::Stroke::new(1.5_f32, visuals.text_color());
    let r = 3.8;
    painter.line_segment(
        [
            egui::Pos2::new(c.x - r, c.y - r),
            egui::Pos2::new(c.x + r, c.y + r),
        ],
        s,
    );
    painter.line_segment(
        [
            egui::Pos2::new(c.x + r, c.y - r),
            egui::Pos2::new(c.x - r, c.y + r),
        ],
        s,
    );
    response.clicked()
}

/// A titled block that groups one setting. Flat, PureMap-style: a strong
/// heading, the controls, then a hairline — no filled cards.
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.label(egui::RichText::new(title).strong().size(15.0));
    ui.add_space(5.0);
    body(ui);
    ui.add_space(9.0);
    ui.separator();
    ui.add_space(9.0);
}

/// Toolbar icons, hand-drawn with the painter as 1.8px line art — the same
/// design language as PureMap (58x62 icon buttons, icon above,
/// 11px label below, groups split by hairline separators).
#[derive(Clone, Copy)]
enum Icon {
    AddFile,
    AddFolder,
    Undo,
    Redo,
    Trash,
    SelectAll,
    Invert,
    Layers,
    Folder,
    Clock,
    Hash,
}

#[derive(Clone, Copy, PartialEq)]
enum IconState {
    Normal,
    Active,
    Disabled,
}

/// One PureMap-style toolbar button. Returns true when clicked.
fn icon_button(ui: &mut egui::Ui, label: &str, icon: Icon, state: IconState) -> bool {
    use egui::{Align2, FontId, Pos2, Rect, Sense, Stroke, Vec2};
    let disabled = state == IconState::Disabled;
    let sense = if disabled { Sense::hover() } else { Sense::click() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(58.0, 62.0), sense);
    if state == IconState::Active {
        ui.painter().rect_filled(
            rect,
            8.0,
            ui.visuals().selection.bg_fill.gamma_multiply(0.35),
        );
    } else if !disabled && response.hovered() {
        ui.painter()
            .rect_filled(rect, 8.0, ui.visuals().widgets.hovered.bg_fill);
    }
    let painter = ui.painter();
    let color = if disabled {
        ui.visuals().weak_text_color()
    } else if response.hovered() || state == IconState::Active {
        ui.visuals().strong_text_color()
    } else {
        ui.visuals().text_color()
    };
    let center = Pos2::new(rect.center().x, rect.top() + 21.0);
    let stroke = Stroke::new(1.8_f32, color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
    let c = center;
    match icon {
        Icon::Undo | Icon::Redo => {
            let direction = if matches!(icon, Icon::Undo) { -1.0 } else { 1.0 };
            let points = (0..=16).map(|step| {
                let angle =
                    std::f32::consts::PI * 0.15 + std::f32::consts::PI * 1.45 * step as f32 / 16.0;
                Pos2::new(
                    c.x + direction * angle.cos() * 8.0,
                    c.y + angle.sin() * 8.0,
                )
            });
            painter.add(egui::Shape::line(points.collect(), stroke));
            let tip_x = c.x + direction * 8.0;
            line(Pos2::new(tip_x, c.y - 7.0), Pos2::new(tip_x - direction * 5.0, c.y - 2.0));
            line(Pos2::new(tip_x, c.y - 7.0), Pos2::new(tip_x + direction * 5.0, c.y - 2.0));
        }
        Icon::Trash => {
            painter.rect_stroke(
                Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(12.0, 14.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(Pos2::new(c.x - 8.0, c.y - 7.0), Pos2::new(c.x + 8.0, c.y - 7.0));
            line(Pos2::new(c.x - 3.0, c.y - 10.0), Pos2::new(c.x + 3.0, c.y - 10.0));
            line(Pos2::new(c.x - 3.0, c.y - 2.0), Pos2::new(c.x - 3.0, c.y + 7.0));
            line(Pos2::new(c.x + 3.0, c.y - 2.0), Pos2::new(c.x + 3.0, c.y + 7.0));
        }
        Icon::AddFile => {
            painter.rect_stroke(
                Rect::from_center_size(c + Vec2::new(0.0, 1.0), Vec2::new(13.0, 17.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            line(Pos2::new(c.x - 3.5, c.y - 4.0), Pos2::new(c.x + 3.5, c.y - 4.0));
            line(Pos2::new(c.x - 4.0, c.y + 3.0), Pos2::new(c.x + 4.0, c.y + 3.0));
            line(Pos2::new(c.x, c.y - 1.0), Pos2::new(c.x, c.y + 7.0));
        }
        Icon::AddFolder | Icon::Folder => {
            // Tab of the folder.
            painter.rect_stroke(
                Rect::from_min_max(
                    Pos2::new(c.x - 9.0, c.y - 8.0),
                    Pos2::new(c.x - 2.0, c.y - 3.5),
                ),
                1.5,
                stroke,
                egui::StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_center_size(c + Vec2::new(0.0, 2.5), Vec2::new(19.0, 13.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            if matches!(icon, Icon::AddFolder) {
                line(Pos2::new(c.x + 1.5, c.y + 2.5), Pos2::new(c.x + 7.5, c.y + 2.5));
                line(Pos2::new(c.x + 4.5, c.y - 0.5), Pos2::new(c.x + 4.5, c.y + 5.5));
            }
        }
        Icon::SelectAll | Icon::Invert => {
            painter.rect_stroke(
                Rect::from_center_size(c, Vec2::new(17.0, 17.0)),
                2.0,
                stroke,
                egui::StrokeKind::Inside,
            );
            if matches!(icon, Icon::SelectAll) {
                painter.add(egui::Shape::line(
                    vec![
                        Pos2::new(c.x - 5.0, c.y + 0.5),
                        Pos2::new(c.x - 1.5, c.y + 4.5),
                        Pos2::new(c.x + 5.5, c.y - 4.5),
                    ],
                    stroke,
                ));
            } else {
                // Invert: right half of the box filled.
                painter.rect_filled(
                    Rect::from_min_max(
                        Pos2::new(c.x, c.y - 6.5),
                        Pos2::new(c.x + 6.5, c.y + 6.5),
                    ),
                    0.0,
                    stroke.color,
                );
            }
        }
        Icon::Layers => {
            for off in [(-3.5, -3.5), (0.0, 0.0), (3.5, 3.5)] {
                painter.rect_stroke(
                    Rect::from_center_size(c + Vec2::new(off.0, off.1), Vec2::new(11.0, 11.0)),
                    2.0,
                    stroke,
                    egui::StrokeKind::Inside,
                );
            }
        }
        Icon::Clock => {
            painter.circle_stroke(c, 8.5, stroke);
            line(c, Pos2::new(c.x, c.y - 5.0));
            line(c, Pos2::new(c.x + 3.5, c.y + 2.0));
        }
        Icon::Hash => {
            // "#" for the auto-number toggle: two slanted verticals,
            // two horizontals.
            line(
                Pos2::new(c.x - 2.5, c.y - 8.0),
                Pos2::new(c.x - 4.5, c.y + 8.0),
            );
            line(
                Pos2::new(c.x + 4.5, c.y - 8.0),
                Pos2::new(c.x + 2.5, c.y + 8.0),
            );
            line(
                Pos2::new(c.x - 8.0, c.y - 2.8),
                Pos2::new(c.x + 8.0, c.y - 2.8),
            );
            line(
                Pos2::new(c.x - 8.0, c.y + 2.8),
                Pos2::new(c.x + 8.0, c.y + 2.8),
            );
        }
    }
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - 9.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(11.0),
        if disabled {
            ui.visuals().weak_text_color()
        } else {
            ui.visuals().text_color()
        },
    );
    !disabled && response.clicked()
}

/// macOS-style segmented tab bar: one rounded track, equal segments, the
/// selected segment shown as a raised grey pill (never an accent-filled
/// "button", which reads as a primary action and confuses people).
fn tab_bar(ui: &mut egui::Ui, current: &mut Tab) {
    use egui::{Align2, FontId, Pos2};
    let tabs = [
        (Tab::Overall, "整体"),
        (Tab::Replace, "替换"),
        (Tab::Numbering, "序号"),
        (Tab::Add, "添加"),
        (Tab::Delete, "删除"),
        (Tab::More, "更多"),
    ];
    let dark = ui.visuals().dark_mode;
    let (track, selected_fill, hover_fill) = if dark {
        (
            egui::Color32::from_rgb(0x26, 0x28, 0x2D),
            egui::Color32::from_rgb(0x4B, 0x4E, 0x55),
            egui::Color32::from_rgba_unmultiplied(0xFF, 0xFF, 0xFF, 0x14),
        )
    } else {
        (
            egui::Color32::from_rgb(0xE3, 0xE5, 0xEA),
            egui::Color32::WHITE,
            egui::Color32::from_rgba_unmultiplied(0x00, 0x00, 0x00, 0x0D),
        )
    };
    let height = 30.0;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, egui::CornerRadius::same(8), track);
    let seg_w = rect.width() / tabs.len() as f32;
    for (i, (tab, label)) in tabs.iter().enumerate() {
        let seg = egui::Rect::from_min_size(
            rect.min + egui::vec2(seg_w * i as f32, 0.0),
            egui::vec2(seg_w, height),
        );
        let resp = ui.interact(seg, egui::Id::new(("tab_seg", i)), egui::Sense::click());
        let is_current = *current == *tab;
        if is_current {
            painter.rect_filled(seg.shrink(2.0), egui::CornerRadius::same(6), selected_fill);
        } else if resp.hovered() {
            painter.rect_filled(seg.shrink(2.0), egui::CornerRadius::same(6), hover_fill);
        }
        let color = if is_current {
            ui.visuals().strong_text_color()
        } else {
            ui.visuals().text_color()
        };
        painter.text(
            // +2.9 ≈ the CJK ink correction at this 13px size
            // (see LABEL_INK_NUDGE).
            Pos2::new(seg.center().x, seg.center().y + 2.9),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(13.0),
            color,
        );
        if resp.clicked() {
            *current = *tab;
        }
    }
}

/// System file picker (macOS/Windows). Other platforms: no-op for now.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn pick_files() -> Vec<PathBuf> {
    // pick_files() already allows multiple selection in the macOS
    // panel (rfd sets AllowsMultipleSelection) — but plain clicks
    // replace the selection there, so spell the gestures out in the
    // title where the user can't miss them.
    rfd::FileDialog::new()
        .set_title("添加文件（可多选：⌘+点击逐个选，⇧+点击连选）")
        .pick_files()
        .unwrap_or_default()
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn pick_files() -> Vec<PathBuf> {
    Vec::new()
}

/// System folder picker (macOS/Windows). Other platforms: no-op for now.
#[cfg(any(target_os = "macos", target_os = "windows"))]
fn pick_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn pick_folder() -> Option<PathBuf> {
    None
}

/// Human-readable file size for the list.
/// File dialog for saving the operation pipeline (macOS/Windows);
/// other platforms fall back to the given path directly.
fn choose_save_path(current: &str) -> Option<PathBuf> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let name = std::path::Path::new(current)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "operations.json".to_string());
        return rfd::FileDialog::new()
            .set_title("保存操作记录")
            .set_file_name(name)
            .add_filter("JSON", &["json"])
            .save_file();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Some(PathBuf::from(current))
    }
}

/// File dialog for opening an operation pipeline file (macOS/Windows);
/// other platforms fall back to the given path directly.
fn choose_open_path(current: &str) -> Option<PathBuf> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let _ = current;
        return rfd::FileDialog::new()
            .set_title("载入操作记录")
            .add_filter("JSON", &["json"])
            .pick_file();
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        Some(PathBuf::from(current))
    }
}

fn human_size(bytes: u64) -> String {    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Display/sort helpers for list rows.
fn file_name_of(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn file_size_of(row: &Row) -> u64 {
    if row.is_dir {
        return 0;
    }
    std::fs::metadata(&row.path).map(|m| m.len()).unwrap_or(0)
}

fn file_mtime_of(row: &Row) -> std::time::SystemTime {
    std::fs::metadata(&row.path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Overall,
    Replace,
    Numbering,
    Add,
    Delete,
    More,
}

/// File-list sort keys (header clicks). Added = insertion order.
#[derive(Clone, Copy, PartialEq)]
enum SortKey {
    Added,
    Name,
    Size,
    Date,
}

struct Row {
    path: PathBuf,
    checked: bool,
    is_dir: bool,
    /// Why the last execute failed for this file, if it did. Shown in
    /// red in the status column until the next run.
    error: Option<String>,
}

struct SmartRenameApp {
    dark_mode: bool,
    follow_system_theme: bool,
    rows: Vec<Row>,
    pipeline: Vec<Operation>,
    /// Last frame's live draft (tab settings not yet committed to the
    /// pipeline); the preview refreshes whenever this changes.
    last_draft: Vec<Operation>,
    /// File-list name filter (display only; never affects which rows
    /// are checked or executed).
    filter: String,
    preview: Vec<PreviewItem>,
    undo_log: Option<UndoLog>,
    last_pipeline: Option<Vec<Operation>>,
    status: String,
    tab: Tab,
    preserve_timestamps: bool,
    auto_number: bool,
    recursive: bool,
    include_dirs: bool,
    ops_file_input: String,
    sort_key: SortKey,
    sort_asc: bool,

    // 整体 tab
    case_name: CaseKind,
    case_ext: CaseKind,
    capitalize: bool,
    cap_space_only: bool,
    change_ext_on: bool,
    change_ext_value: String,

    // 替换 tab
    rep_find: String,
    rep_replacement: String,
    rep_regex: bool,
    rep_case_sensitive: bool,
    rep_include_ext: bool,

    // 序号 tab
    num_on: bool,
    num_start: i64,
    num_step: i64,
    num_width: usize,
    num_style: NumberStyle,
    num_placement: NumberPlacement,
    num_template: String,

    // 添加 tab
    add_prefix: String,
    add_suffix: String,
    insert_pos: usize,
    insert_text: String,

    // 删除 tab
    del_start: usize,
    del_count: usize,
    del_text: String,
    del_case_sensitive: bool,

    // 更多 tab
    script_preset: Option<ScriptPreset>,
    date_on: bool,
    date_source: DateSource,
    date_format: DateFormat,
    date_mode: DateMode,
    show_about: bool,
    show_regex_help: bool,
    /// macOS native menu bar state (dummy handle on other platforms).
    // Read only on macOS (the native menu lives there).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    native_menu: Option<NativeMenuHandle>,
}

impl Default for SmartRenameApp {
    fn default() -> Self {
        Self {
            dark_mode: false,
            follow_system_theme: true,
            rows: Vec::new(),
            pipeline: Vec::new(),
            last_draft: Vec::new(),
            filter: String::new(),
            preview: Vec::new(),
            undo_log: None,
            last_pipeline: None,
            status: "把文件拖进窗口，或点工具栏的「添加文件 / 加文件夹」。".to_string(),
            tab: Tab::Overall,
            preserve_timestamps: false,
            auto_number: false,
            recursive: false,
            include_dirs: false,
            ops_file_input: "operations.json".to_string(),
            sort_key: SortKey::Added,
            sort_asc: true,
            case_name: CaseKind::Keep,
            case_ext: CaseKind::Keep,
            capitalize: false,
            cap_space_only: false,
            change_ext_on: false,
            change_ext_value: String::new(),
            rep_find: String::new(),
            rep_replacement: String::new(),
            rep_regex: false,
            rep_case_sensitive: true,
            rep_include_ext: false,
            num_on: false,
            num_start: 1,
            num_step: 1,
            num_width: 3,
            num_style: NumberStyle::Numeric,
            num_placement: NumberPlacement::Prefix,
            num_template: String::new(),
            add_prefix: String::new(),
            add_suffix: String::new(),
            insert_pos: 1,
            insert_text: String::new(),
            del_start: 1,
            del_count: 0,
            del_text: String::new(),
            del_case_sensitive: true,
            script_preset: None,
            date_on: false,
            date_source: DateSource::Exif,
            date_format: DateFormat::Compact,
            date_mode: DateMode::Replace,
            show_about: false,
            show_regex_help: false,
            native_menu: None,
        }
    }
}

/// One teaching example for the regex replace feature.
struct RegexExample {
    title: &'static str,
    explain: &'static str,
    find: &'static str,
    replacement: &'static str,
    sample: &'static str,
}

const REGEX_EXAMPLES: &[RegexExample] = &[
    RegexExample {
        title: "去掉开头的编号",
        explain: "行首的一串数字、连同后面的空格或 -._ 一起删掉。「001 - 歌名」→「歌名」",
        find: r"^\d+[\s._-]*",
        replacement: "",
        sample: "001 - My Song.mp3 → My Song.mp3",
    },
    RegexExample {
        title: "删掉括号及内容",
        explain: "删掉一对圆括号和里面的字。「歌名 (Live)」→「歌名」",
        find: r"[\(（].*?[\)）]",
        replacement: "",
        sample: "Song (Live).mp3 → Song.mp3",
    },
    RegexExample {
        title: "前后两段调换",
        explain: "以「 - 」分开的两段互换位置，$1、$2 是分到的两段。「歌手 - 歌名」→「歌名 - 歌手」",
        find: r"^(.+?) - (.+)$",
        replacement: "$2 - $1",
        sample: "Jay - Song.mp3 → Song - Jay.mp3",
    },
    RegexExample {
        title: "日期补零",
        explain: "把 2024-1-2 这样的日期补成 2024-01-02，方便排序。",
        find: r"(\d{4})-(\d)-(\d)",
        replacement: "$1-0$2-0$3",
        sample: "2024-1-2.txt → 2024-01-02.txt",
    },
    RegexExample {
        title: "空格换成下划线",
        explain: "把空格（多个连着也算一个）换成下划线。",
        find: r"\s+",
        replacement: "_",
        sample: "my file name.txt → my_file_name.txt",
    },
    RegexExample {
        title: "去掉来源标记【】",
        explain: "删掉文件名最前面的一对【】和里面的字。",
        find: r"^【.*?】",
        replacement: "",
        sample: "【XX资源站】电影.mkv → 电影.mkv",
    },
    RegexExample {
        title: "删方括号标签",
        explain: "字幕组、来源站常把标签放在方括号里，一并删掉（前面的空格也带走）。",
        find: r" ?\[.*?\]",
        replacement: "",
        sample: "[字幕组] 剧集.mkv → 剧集.mkv",
    },
    RegexExample {
        title: "日期去横线",
        explain: "把年月日抓出来直接拼回去，横线就没了。",
        find: r"(\d{4})-(\d{2})-(\d{2})",
        replacement: "$1$2$3",
        sample: "2024-08-15.txt → 20240815.txt",
    },
    RegexExample {
        title: "集数补零",
        explain: "$ 表示结尾：只处理名字最后一位是个位数的集数，补成两位。",
        find: r"E(\d)$",
        replacement: "E0$1",
        sample: "乡村爱情E3.mkv → 乡村爱情E03.mkv",
    },
    RegexExample {
        title: "年份提到最前",
        explain: "把结尾括号里的年份抓出来，挪到名字最前面，方便按年份排序。",
        find: r"^(.*?)\s*\((\d{4})\)$",
        replacement: "$2 - $1",
        sample: "一部电影 (2024).mkv → 2024 - 一部电影.mkv",
    },
    RegexExample {
        title: "横线下划线转空格",
        explain: "[_-] 表示横线或下划线，连续几个只换成一个空格。",
        find: r"[_-]+",
        replacement: " ",
        sample: "my-file__name.txt → my file name.txt",
    },
    RegexExample {
        title: "点号转空格",
        explain: "下载的影视文件爱用点号分隔；只作用于主名，不动扩展名。",
        find: r"\.",
        replacement: " ",
        sample: "My.Movie.2024.mkv → My Movie 2024.mkv",
    },
    RegexExample {
        title: "横线两侧统一空格",
        explain: "横线两边不管有没有空格、几个空格，都统一成「 - 」。",
        find: r"\s*-\s*",
        replacement: " - ",
        sample: "歌手-歌曲.mp3 → 歌手 - 歌曲.mp3",
    },
    RegexExample {
        title: "只保留数字",
        explain: "[^\\d] 是「不是数字的字符」，整串删掉就只剩数字。",
        find: r"[^\d]+",
        replacement: "",
        sample: "第12集.mkv → 12.mkv",
    },
    RegexExample {
        title: "删结尾的副本标记",
        explain: "系统另存副本时加的 (2)、- 副本、 copy，都在结尾，一次清掉。",
        find: r"(\s*\(\d+\)|\s*-\s*副本|\s+copy)$",
        replacement: "",
        sample: "报告 (2).docx → 报告.docx",
    },
];

impl SmartRenameApp {
    /// Indices of the rows matching the name filter (all rows when
    /// the filter is empty). Display only.
    fn visible_indices(&self) -> Vec<usize> {
        let needle = self.filter.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                needle.is_empty() || file_name_of(&r.path).to_lowercase().contains(&needle)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn visible_row_count(&self) -> usize {
        self.visible_indices().len()
    }

    fn checked_paths(&self) -> Vec<PathBuf> {
        self.rows
            .iter()
            .filter(|r| r.checked)
            .map(|r| r.path.clone())
            .collect()
    }

    fn refresh_preview(&mut self) {
        let paths = self.checked_paths();
        let strategy = if self.auto_number {
            ConflictStrategy::AutoNumber
        } else {
            ConflictStrategy::Block
        };
        // The preview always shows pipeline + live draft (the current
        // tab settings), so typing in any field updates it immediately.
        let mut ops = self.pipeline.clone();
        ops.extend(self.draft_ops());
        self.last_draft = self.draft_ops();
        self.preview = preview_with_strategy(&paths, &ops, strategy);
    }

    fn numbering_op(&self) -> Operation {
        Operation::Numbering {
            start: self.num_start,
            step: self.num_step,
            width: self.num_width,
            style: self.num_style,
            placement: self.num_placement,
            template: self.num_template.clone(),
        }
    }

    /// The live draft: operations implied by the current control state
    /// of every tab, in tab order. A section contributes when it is
    /// "armed": an enable checkbox ticked, a non-neutral choice made,
    /// or text present in its input. One-click sections （繁简、命名
    /// 风格、净化、父文件夹、改时间） carry no editable state and are
    /// not part of the draft.
    fn draft_ops(&self) -> Vec<Operation> {
        let mut ops = Vec::new();
        if self.case_name != CaseKind::Keep || self.case_ext != CaseKind::Keep {
            ops.push(Operation::Case {
                name: self.case_name,
                ext: self.case_ext,
            });
        }
        if self.capitalize {
            ops.push(Operation::CapitalizeWords {
                separator: if self.cap_space_only {
                    WordSeparator::SpaceOnly
                } else {
                    WordSeparator::NonAlphanumeric
                },
            });
        }
        if self.change_ext_on {
            let v = self.change_ext_value.trim().to_string();
            ops.push(Operation::ChangeExtension {
                new_ext: if v.is_empty() { None } else { Some(v) },
            });
        }
        if !self.rep_find.is_empty() {
            ops.push(Operation::Replace {
                find: self.rep_find.clone(),
                replacement: self.rep_replacement.clone(),
                use_regex: self.rep_regex,
                case_sensitive: self.rep_case_sensitive,
                include_ext: self.rep_include_ext,
            });
        }
        if self.num_on {
            ops.push(self.numbering_op());
        }
        if !self.add_prefix.is_empty() {
            ops.push(Operation::AddPrefix {
                text: self.add_prefix.clone(),
            });
        }
        if !self.add_suffix.is_empty() {
            ops.push(Operation::AddSuffix {
                text: self.add_suffix.clone(),
            });
        }
        if !self.insert_text.is_empty() {
            ops.push(Operation::InsertAt {
                position: self.insert_pos,
                text: self.insert_text.clone(),
            });
        }
        if self.del_count > 0 {
            ops.push(Operation::DeleteRange {
                start: self.del_start,
                count: self.del_count,
            });
        }
        if !self.del_text.is_empty() {
            ops.push(Operation::DeleteText {
                text: self.del_text.clone(),
                case_sensitive: self.del_case_sensitive,
            });
        }
        if let Some(preset) = self.script_preset {
            ops.push(Operation::Script {
                preset,
                target: ScriptTarget::Stem,
            });
        }
        if self.date_on {
            ops.push(Operation::DateName {
                source: self.date_source,
                format: self.date_format,
                mode: self.date_mode,
            });
        }
        ops
    }

    /// After an operation is committed to the pipeline, disarm the
    /// section it came from so the live draft does not apply it twice.
    fn disarm_for(&mut self, op: &Operation) {
        match op {
            Operation::Case { .. } => {
                self.case_name = CaseKind::Keep;
                self.case_ext = CaseKind::Keep;
            }
            Operation::CapitalizeWords { .. } => self.capitalize = false,
            Operation::ChangeExtension { .. } => self.change_ext_on = false,
            Operation::Replace { .. } => self.rep_find.clear(),
            Operation::Numbering { .. } => self.num_on = false,
            Operation::AddPrefix { .. } => self.add_prefix.clear(),
            Operation::AddSuffix { .. } => self.add_suffix.clear(),
            Operation::InsertAt { .. } => self.insert_text.clear(),
            Operation::DeleteRange { .. } => self.del_count = 0,
            Operation::DeleteText { .. } => self.del_text.clear(),
            Operation::Script { .. } => self.script_preset = None,
            Operation::DateName { .. } => self.date_on = false,
            _ => {}
        }
    }

    /// Sort the file list; numbering and other order-based operations
    /// follow the visible order. Clicking the active key flips direction.
    fn sort_rows(&mut self, key: SortKey) {
        if self.sort_key == key && key != SortKey::Added {
            self.sort_asc = !self.sort_asc;
        } else {
            self.sort_key = key;
            self.sort_asc = true;
        }
        let asc = self.sort_asc;
        match key {
            SortKey::Added => {}
            SortKey::Name => self.rows.sort_by(|a, b| {
                let ka = file_name_of(&a.path).to_lowercase();
                let kb = file_name_of(&b.path).to_lowercase();
                if asc { ka.cmp(&kb) } else { kb.cmp(&ka) }
            }),
            SortKey::Size => self.rows.sort_by(|a, b| {
                let sa = file_size_of(a);
                let sb = file_size_of(b);
                if asc { sa.cmp(&sb) } else { sb.cmp(&sa) }
            }),
            SortKey::Date => self.rows.sort_by(|a, b| {
                let da = file_mtime_of(a);
                let db = file_mtime_of(b);
                if asc { da.cmp(&db) } else { db.cmp(&da) }
            }),
        }
        self.refresh_preview();
    }

    fn add_paths(&mut self, paths: Vec<PathBuf>) {
        let mut added = 0;
        for p in paths {
            // Hidden files never enter the list (folder walks already
            // filter them in collect_paths; this covers direct picks
            // and drag-and-drop).
            if is_hidden_path(&p) {
                continue;
            }
            if self.rows.iter().any(|r| r.path == p) {
                continue;
            }
            let is_dir = p.is_dir();
            self.rows.push(Row {
                path: p,
                checked: true,
                is_dir,
                error: None,
            });
            added += 1;
        }
        if added > 0 {
            self.status = format!("已添加 {} 个文件。", added);
        }
        self.refresh_preview();
    }

    /// Toolbar / menu / shortcut entry points for adding content.
    fn add_files_dialog(&mut self) {
        let files = pick_files();
        if !files.is_empty() {
            self.add_paths(files);
        }
    }

    fn add_folder_dialog(&mut self) {
        if let Some(dir) = pick_folder() {
            let mut paths = collect_paths(&dir, self.recursive, self.include_dirs);
            if self.include_dirs {
                paths.push(dir);
            }
            self.add_paths(paths);
        }
    }

    /// Load the last executed pipeline back as the pending one （重演）.
    fn do_replay(&mut self) {
        if let Some(last) = self.last_pipeline.clone() {
            self.pipeline = last;
            self.refresh_preview();
            self.status = "已载入上一次执行的操作。".to_string();
        }
    }

    /// Remove the checked rows from the list (files on disk untouched).
    fn remove_checked_rows(&mut self) {
        let before = self.rows.len();
        self.rows.retain(|r| !r.checked);
        let removed = before - self.rows.len();
        if removed > 0 {
            self.status = format!("已从列表移除 {} 个文件。", removed);
            self.refresh_preview();
        }
    }

    fn push_op(&mut self, op: Operation) {
        self.status = format!("已应用：{}", op.describe());
        self.pipeline.push(op.clone());
        self.disarm_for(&op);
        self.refresh_preview();
    }

    fn do_execute(&mut self) {
        // Fold the live draft into the pipeline first, so what gets
        // executed is exactly what the preview showed — and the
        // operation record (and 重演） tells the truth about it.
        let draft = self.draft_ops();
        if !draft.is_empty() {
            for op in &draft {
                self.pipeline.push(op.clone());
                self.disarm_for(op);
            }
            self.refresh_preview();
        }
        let (results, log) = execute(
            &self.preview,
            &ExecuteOptions {
                preserve_timestamps: self.preserve_timestamps,
            },
        );
        let ok = results.iter().filter(|r| r.success).count();
        let total = results.len();
        let failed = total - ok;
        // Keep each failure's reason on its row (red, in the status
        // column) instead of leaving the user to guess from the count.
        for row in &mut self.rows {
            row.error = None;
        }
        for r in &results {
            if !r.success {
                if let Some(row) = self.rows.iter_mut().find(|row| row.path == r.path) {
                    row.error = Some(r.message.clone());
                }
            }
        }
        if !log.pairs.is_empty() {
            self.undo_log = Some(log);
        }
        self.last_pipeline = Some(self.pipeline.clone());
        // Rebuild the row list from post-rename reality.
        let renamed: std::collections::HashMap<PathBuf, PathBuf> = self
            .preview
            .iter()
            .filter(|i| i.status == PreviewStatus::Ready)
            .filter(|i| results.iter().any(|r| r.success && r.path == i.old_path))
            .map(|i| (i.old_path.clone(), i.new_path.clone()))
            .collect();
        for row in &mut self.rows {
            if let Some(newp) = renamed.get(&row.path) {
                row.path = newp.clone();
            }
        }
        self.rows.retain(|r| r.path.exists());
        // Files whose times were actually updated by this run: every
        // Unchanged item with a time spec, plus renamed items whose
        // rename succeeded.
        let times_ok = self
            .preview
            .iter()
            .filter(|i| {
                i.times.is_some()
                    && (i.status == PreviewStatus::Unchanged
                        || (i.status == PreviewStatus::Ready
                            && results.iter().any(|r| r.success && r.path == i.old_path)))
            })
            .count();
        // The executed operations are DONE. If the pipeline stayed
        // pending, the refreshed preview would apply it a second time
        // on top of the new names (double prefixes etc.). It lives on
        // in last_pipeline for 重演 and in the undo log; only clear
        // when something actually took effect.
        if ok > 0 || times_ok > 0 {
            self.pipeline.clear();
        }
        self.status = if total == 0 && times_ok > 0 {
            format!("已更新 {} 个文件的日期。", times_ok)
        } else if total == 0 {
            "没有可执行的文件（先勾选文件并设置好操作）。".to_string()
        } else if failed > 0 {
            format!(
                "执行完成：成功 {} / {}，失败 {} 个（原因见列表「状态」列）。",
                ok, total, failed
            )
        } else if times_ok > 0 {
            format!("执行完成：成功 {} / {}，并更新文件日期。", ok, total)
        } else {
            format!("执行完成：成功 {} / {}。", ok, total)
        };
        self.refresh_preview();
    }

    fn do_undo(&mut self) {
        if let Some(log) = self.undo_log.take() {
            for row in &mut self.rows {
                row.error = None;
            }
            let results = undo(&log);
            let ok = results.iter().filter(|r| r.success).count();
            let back: std::collections::HashMap<PathBuf, PathBuf> = log
                .pairs
                .iter()
                .map(|p| (p.current.clone(), p.original.clone()))
                .collect();
            for row in &mut self.rows {
                if let Some(orig) = back.get(&row.path) {
                    row.path = orig.clone();
                }
            }
            self.status = format!("撤消完成：{} 个文件已恢复原名。", ok);
            self.refresh_preview();
        }
    }
}

impl eframe::App for SmartRenameApp {
    /// Persist the small settings set (called by the framework on an
    /// interval and on exit, with the persistence feature enabled).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            SETTINGS_KEY,
            &Settings {
                follow_system_theme: self.follow_system_theme,
                dark_mode: self.dark_mode,
                recursive: self.recursive,
                include_dirs: self.include_dirs,
                preserve_timestamps: self.preserve_timestamps,
                auto_number: self.auto_number,
            },
        );
    }
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Live draft sync: if any tab's settings changed since the
        // last frame, the preview follows immediately.
        if self.draft_ops() != self.last_draft {
            self.refresh_preview();
        }
        // Follow the OS theme live, unless the user picked one manually.
        if self.follow_system_theme {
            if let Some(theme) = ctx.system_theme() {
                let dark = theme == egui::Theme::Dark;
                if dark != self.dark_mode {
                    self.dark_mode = dark;
                    apply_style(ctx, dark);
                }
            }
        }
        // Drag & drop files into the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            let mut expanded = Vec::new();
            for p in dropped {
                if p.is_dir() {
                    expanded.extend(collect_paths(&p, self.recursive, self.include_dirs));
                    if self.include_dirs {
                        expanded.push(p);
                    }
                } else {
                    expanded.push(p);
                }
            }
            self.add_paths(expanded);
        }

        // macOS: native system menu bar (init on first frame, then events).
        #[cfg(target_os = "macos")]
        self.native_menu_update(ctx);

        // Keyboard shortcuts — only when no text field has focus, so
        // ⌘Z inside an input still means "undo typing" and Delete
        // still edits text. (On macOS, ⌘O lives on the native menu
        // items instead; the app-level keys below would double-fire.)
        if ctx.memory(|m| m.focused().is_none()) {
            let (undo, redo, delete) = ctx.input(|i| {
                (
                    i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::Z),
                    i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                )
            });
            if undo {
                self.do_undo();
            }
            if redo {
                self.do_replay();
            }
            if delete {
                self.remove_checked_rows();
            }
            #[cfg(not(target_os = "macos"))]
            {
                let (open_files, open_folder) = ctx.input(|i| {
                    (
                        i.modifiers.command && !i.modifiers.shift && i.key_pressed(egui::Key::O),
                        i.modifiers.command && i.modifiers.shift && i.key_pressed(egui::Key::O),
                    )
                });
                if open_files {
                    self.add_files_dialog();
                }
                if open_folder {
                    self.add_folder_dialog();
                }
            }
        }

        // Other platforms: in-window menu bar with the same content.
        #[cfg(not(target_os = "macos"))]
        egui::TopBottomPanel::top("menubar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("文件", |ui| {
                    if ui.button("添加文件…").clicked() {
                        self.add_files_dialog();
                        ui.close_menu();
                    }
                    if ui.button("添加文件夹…").clicked() {
                        self.add_folder_dialog();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("清空列表").clicked() {
                        self.rows.clear();
                        self.refresh_preview();
                        ui.close_menu();
                    }
                    if ui.button("全选").clicked() {
                        for r in &mut self.rows {
                            r.checked = true;
                        }
                        self.refresh_preview();
                        ui.close_menu();
                    }
                    if ui.button("反选").clicked() {
                        for r in &mut self.rows {
                            r.checked = !r.checked;
                        }
                        self.refresh_preview();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("保存操作记录…").clicked() {
                        self.save_ops_dialog();
                        ui.close_menu();
                    }
                    if ui.button("载入操作记录…").clicked() {
                        self.load_ops_dialog();
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui.button("退出 SmartReName").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("编辑", |ui| {
                    if ui
                        .add_enabled(self.undo_log.is_some(), egui::Button::new("撤消更名"))
                        .clicked()
                    {
                        self.do_undo();
                        ui.close_menu();
                    }
                    if ui
                        .add_enabled(
                            self.last_pipeline.is_some(),
                            egui::Button::new("重演上一次操作"),
                        )
                        .clicked()
                    {
                        if let Some(last) = self.last_pipeline.clone() {
                            self.pipeline = last;
                            self.refresh_preview();
                            self.status = "已载入上一次执行的操作。".to_string();
                        }
                        ui.close_menu();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            !self.pipeline.is_empty(),
                            egui::Button::new("清空操作记录"),
                        )
                        .clicked()
                    {
                        self.pipeline.clear();
                        self.refresh_preview();
                        ui.close_menu();
                    }
                });
                ui.menu_button("显示", |ui| {
                    ui.label("主题");
                    for (label, follow, dark) in [
                        ("跟随系统", true, false),
                        ("浅色", false, false),
                        ("深色", false, true),
                    ] {
                        let selected = if follow {
                            self.follow_system_theme
                        } else {
                            !self.follow_system_theme && self.dark_mode == dark
                        };
                        let text = if selected {
                            format!("✓ {}", label)
                        } else {
                            format!("　 {}", label)
                        };
                        if ui.selectable_label(false, text).clicked() {
                            if follow {
                                self.follow_system_theme = true;
                            } else {
                                self.follow_system_theme = false;
                                self.dark_mode = dark;
                                apply_style(ctx, dark);
                            }
                            ui.close_menu();
                        }
                    }
                });
                ui.menu_button("帮助", |ui| {
                    if ui.button("正则表达式示例").clicked() {
                        self.show_regex_help = true;
                        ui.close_menu();
                    }
                    if ui.button("关于 SmartReName").clicked() {
                        self.show_about = true;
                        ui.close_menu();
                    }
                });
            });
        });

        egui::TopBottomPanel::top("toolbar")
            .exact_height(76.0)
            .show(ctx, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("toolbar_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.horizontal_centered(|ui| {
                            if icon_button(ui, "添加文件", Icon::AddFile, IconState::Normal) {
                                self.add_files_dialog();
                            }
                            if icon_button(ui, "加文件夹", Icon::AddFolder, IconState::Normal) {
                                self.add_folder_dialog();
                            }
                            ui.separator();
                            let undo_state = if self.undo_log.is_some() {
                                IconState::Normal
                            } else {
                                IconState::Disabled
                            };
                            if icon_button(ui, "撤消", Icon::Undo, undo_state) {
                                self.do_undo();
                            }
                            let replay_state = if self.last_pipeline.is_some() {
                                IconState::Normal
                            } else {
                                IconState::Disabled
                            };
                            if icon_button(ui, "重演", Icon::Redo, replay_state) {
                                self.do_replay();
                            }
                            ui.separator();
                            if icon_button(ui, "清空", Icon::Trash, IconState::Normal) {
                                self.rows.clear();
                                self.refresh_preview();
                            }
                            if icon_button(ui, "全选", Icon::SelectAll, IconState::Normal) {
                                for r in &mut self.rows {
                                    r.checked = true;
                                }
                                self.refresh_preview();
                            }
                            if icon_button(ui, "反选", Icon::Invert, IconState::Normal) {
                                for r in &mut self.rows {
                                    r.checked = !r.checked;
                                }
                                self.refresh_preview();
                            }
                            ui.separator();
                            let rec_state = if self.recursive {
                                IconState::Active
                            } else {
                                IconState::Normal
                            };
                            if icon_button(ui, "遍历", Icon::Layers, rec_state) {
                                self.recursive = !self.recursive;
                            }
                            let dir_state = if self.include_dirs {
                                IconState::Active
                            } else {
                                IconState::Normal
                            };
                            if icon_button(ui, "含文件夹", Icon::Folder, dir_state) {
                                self.include_dirs = !self.include_dirs;
                            }
                            let time_state = if self.preserve_timestamps {
                                IconState::Active
                            } else {
                                IconState::Normal
                            };
                            if icon_button(ui, "保留日期", Icon::Clock, time_state) {
                                self.preserve_timestamps = !self.preserve_timestamps;
                            }
                            let num_state = if self.auto_number {
                                IconState::Active
                            } else {
                                IconState::Normal
                            };
                            if icon_button(ui, "重名加序", Icon::Hash, num_state) {
                                self.auto_number = !self.auto_number;
                                self.refresh_preview();
                            }
                        });
                    });
            });

        egui::TopBottomPanel::bottom("statusbar")
            .exact_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    // Actionable = will rename, or name unchanged but a
                    // file-time change is queued (SetTimes-only runs).
                    let ready = self
                        .preview
                        .iter()
                        .filter(|i| {
                            i.status == PreviewStatus::Ready
                                || (i.status == PreviewStatus::Unchanged
                                    && i.times.is_some())
                        })
                        .count();
                    let label_w = (ui.available_width() - 170.0).max(80.0);
                    ui.add_sized(
                        [label_w, 34.0],
                        egui::Label::new(&self.status).wrap(),
                    );
                    if text_button_enabled(
                        ui,
                        egui::vec2(150.0, 34.0),
                        &format!("应用（{} 个）", ready),
                        true,
                        ready > 0,
                    ) {
                        self.do_execute();
                    }
                });
            });

        egui::SidePanel::right("ops_panel")
            .resizable(true)
            .default_width(360.0)
            .min_width(320.0)
            .max_width(480.0)
            .show(ctx, |ui| {
                // Segmented tab bar (system style) stays pinned;
                // everything below it scrolls.
                tab_bar(ui, &mut self.tab);
                ui.add_space(8.0);
                egui::ScrollArea::vertical()
                    .id_salt("ops_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        match self.tab {
                            Tab::Overall => self.tab_overall(ui),
                            Tab::Replace => self.tab_replace(ui),
                            Tab::Numbering => self.tab_numbering(ui),
                            Tab::Add => self.tab_add(ui),
                            Tab::Delete => self.tab_delete(ui),
                            Tab::More => self.tab_more(ui),
                        }
                        self.ops_record_section(ui);
                    });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(format!(
                    "文件列表（{} 个，已勾选 {}）",
                    self.rows.len(),
                    self.rows.iter().filter(|r| r.checked).count()
                ));
                // Name filter, right-aligned. Display only.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_sized(
                        [230.0, 26.0],
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("筛选文件名…")
                            .vertical_align(egui::Align::Center),
                    );
                    if !self.filter.trim().is_empty() {
                        let n = self.visible_row_count();
                        ui.label(
                            egui::RichText::new(format!("显示 {} 个", n))
                                .size(12.5)
                                .weak(),
                        );
                    }
                });
            });
            ui.add_space(4.0);
            if self.rows.is_empty() {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("把文件或文件夹拖到这里")
                            .size(17.0)
                            .color(ui.visuals().weak_text_color()),
                    );
                    ui.label(
                        egui::RichText::new("也可以点工具栏的「添加文件」或「加文件夹」")
                            .color(ui.visuals().weak_text_color()),
                    );
                });
                return;
            }
            // Fixed column layout: every row is allocated an exact 34px
            // rect and its cells are placed into sub-rects — deterministic
            // alignment and height, like a Finder list.
            let avail = ui.available_width();
            let check_w = 30.0;
            let size_w = 96.0;
            let status_w = 168.0;
            let remove_w = 48.0;
            let rest = (avail - check_w - size_w - status_w - remove_w - 16.0).max(120.0);
            let name_w = rest * 0.5;
            let preview_w = rest - name_w;
            let col_x = [
                8.0,
                8.0 + check_w,
                8.0 + check_w + name_w,
                8.0 + check_w + name_w + preview_w,
                8.0 + check_w + name_w + preview_w + size_w,
                8.0 + check_w + name_w + preview_w + size_w + status_w,
            ];
            {
                let (hrect, _) =
                    ui.allocate_exact_size(egui::vec2(avail, 22.0), egui::Sense::hover());
                let painter = ui.painter().clone();
                for (i, title) in ["", "文件名", "预览", "大小", "状态", ""].iter().enumerate() {
                    if title.is_empty() {
                        continue;
                    }
                    painter.text(
                        egui::Pos2::new(hrect.left() + col_x[i], hrect.center().y),
                        egui::Align2::LEFT_CENTER,
                        title,
                        egui::FontId::proportional(13.0),
                        ui.visuals().strong_text_color(),
                    );
                }
                // Clickable sort headers: 文件名 / 大小 / 日期 (right end).
                // The active key gets a small painted triangle (no font
                // glyph involved). Clicking again flips the direction.
                let font = egui::FontId::proportional(13.0);
                let mut sort_click: Option<SortKey> = None;
                let cells: [(SortKey, f32, f32, &str); 3] = [
                    (SortKey::Name, col_x[1], col_x[2], "文件名"),
                    (SortKey::Size, col_x[3], col_x[4], "大小"),
                    (SortKey::Date, avail - 52.0, avail - 8.0, "日期"),
                ];
                for (ci, (key, x0, x1, title)) in cells.iter().enumerate() {
                    let cell = egui::Rect::from_min_max(
                        egui::Pos2::new(hrect.left() + x0, hrect.top()),
                        egui::Pos2::new(hrect.left() + x1, hrect.bottom()),
                    );
                    if *key == SortKey::Date {
                        // "日期" is not part of the static title row.
                        painter.text(
                            egui::Pos2::new(cell.left(), cell.center().y),
                            egui::Align2::LEFT_CENTER,
                            title,
                            font.clone(),
                            ui.visuals().strong_text_color(),
                        );
                    }
                    let resp = ui.interact(cell, egui::Id::new(("sort_header", ci)), egui::Sense::click());
                    if resp.hovered() {
                        painter.rect_filled(
                            cell,
                            egui::CornerRadius::same(4),
                            ui.visuals().widgets.hovered.bg_fill.linear_multiply(0.35),
                        );
                        // Redraw the title over the hover wash.
                        painter.text(
                            egui::Pos2::new(cell.left(), cell.center().y),
                            egui::Align2::LEFT_CENTER,
                            title,
                            font.clone(),
                            ui.visuals().strong_text_color(),
                        );
                    }
                    if resp.clicked() {
                        sort_click = Some(*key);
                    }
                    if self.sort_key == *key {
                        let tw = ui.fonts(|f| {
                            f.layout_no_wrap(
                                title.to_string(),
                                font.clone(),
                                ui.visuals().strong_text_color(),
                            )
                            .size()
                            .x
                        });
                        let cx = cell.left() + tw + 7.0;
                        let cy = cell.center().y;
                        let color = ui.visuals().strong_text_color();
                        let pts = if self.sort_asc {
                            vec![
                                egui::Pos2::new(cx, cy - 3.5),
                                egui::Pos2::new(cx - 4.0, cy + 3.0),
                                egui::Pos2::new(cx + 4.0, cy + 3.0),
                            ]
                        } else {
                            vec![
                                egui::Pos2::new(cx, cy + 3.5),
                                egui::Pos2::new(cx - 4.0, cy - 3.0),
                                egui::Pos2::new(cx + 4.0, cy - 3.0),
                            ]
                        };
                        painter.add(egui::Shape::convex_polygon(
                            pts,
                            color,
                            egui::Stroke::NONE,
                        ));
                    }
                }
                if let Some(key) = sort_click {
                    self.sort_rows(key);
                }
            }
            ui.separator();
            // Owned display data (new name + status label) so the row
            // code does not hold a borrow of self.preview.
            let display: std::collections::HashMap<PathBuf, (String, String, bool)> = self
                .preview
                .iter()
                .map(|p| {
                    let (status, bad) = match &p.status {
                        PreviewStatus::Ready => ("可改名".to_string(), false),
                        PreviewStatus::Unchanged if p.times.is_some() => {
                            ("改日期".to_string(), false)
                        }
                        PreviewStatus::Unchanged => ("相同".to_string(), false),
                        PreviewStatus::Error(e) => (format!("错误：{}", e), true),
                        PreviewStatus::Conflict(e) => (format!("冲突：{}", e), true),
                    };
                    (p.old_path.clone(), (p.new_name.clone(), status, bad))
                })
                .collect();
            let mut needs_refresh = false;
            let mut remove_row: Option<usize> = None;
            egui::ScrollArea::vertical()
                .id_salt("file_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let visible = self.visible_indices();
                    if visible.is_empty() {
                        ui.add_space(24.0);
                        ui.vertical_centered(|ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "没有文件名包含「{}」的文件",
                                    self.filter.trim()
                                ))
                                .color(ui.visuals().weak_text_color()),
                            );
                        });
                    }
                    for (vi, idx) in visible.iter().enumerate() {
                        let idx = *idx;
                        let row = &mut self.rows[idx];
                        let (rect, _) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), 34.0),
                            egui::Sense::hover(),
                        );
                        if vi % 2 == 1 {
                            ui.painter().rect_filled(
                                rect,
                                egui::CornerRadius::same(6),
                                ui.visuals().faint_bg_color,
                            );
                        }
                        let cell = |i: usize, w: f32| {
                            egui::Rect::from_min_size(
                                egui::Pos2::new(
                                    rect.left() + col_x[i],
                                    rect.center().y - 11.0,
                                ),
                                egui::vec2(w, 22.0),
                            )
                        };
                        if ui
                            .put(egui::Rect::from_min_size(
                                egui::Pos2::new(rect.left() + col_x[0] + 4.0, rect.center().y - 11.0),
                                egui::vec2(check_w - 4.0, 22.0),
                            ), egui::Checkbox::new(&mut row.checked, ""))
                            .changed()
                        {
                            needs_refresh = true;
                        }
                        let name = row
                            .path
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        ui.put(cell(1, name_w), egui::Label::new(name).truncate());
                        let preview_text = display
                            .get(&row.path)
                            .map(|(n, _, _)| n.clone())
                            .unwrap_or_else(|| "—".to_string());
                        ui.put(cell(2, preview_w), egui::Label::new(preview_text).truncate());
                        let size = if row.is_dir {
                            "文件夹".to_string()
                        } else {
                            std::fs::metadata(&row.path)
                                .map(|m| human_size(m.len()))
                                .unwrap_or_default()
                        };
                        ui.put(cell(3, size_w), egui::Label::new(size).truncate());
                        if let Some(err) = &row.error {
                            // A failed execute leaves its reason here.
                            let rt = egui::RichText::new(format!("失败：{}", err))
                                .color(egui::Color32::from_rgb(0xD9, 0x3A, 0x2E));
                            ui.put(cell(4, status_w), egui::Label::new(rt).truncate());
                        } else if let Some((_, s, bad)) = display.get(&row.path) {
                            let rt = if *bad {
                                egui::RichText::new(s.clone())
                                    .color(egui::Color32::from_rgb(0xD9, 0x3A, 0x2E))
                            } else {
                                egui::RichText::new(s.clone())
                            };
                            ui.put(cell(4, status_w), egui::Label::new(rt).truncate());
                        }
                        if ui
                            .put(
                                cell(5, remove_w),
                                egui::Button::new(egui::RichText::new("移除").weak())
                                    .frame(false),
                            )
                            .clicked()
                        {
                            remove_row = Some(idx);
                        }
                        ui.add_space(2.0);
                    }
                });
            if let Some(i) = remove_row {
                self.rows.remove(i);
                needs_refresh = true;
            }
            if needs_refresh {
                self.refresh_preview();
            }
        });

        if self.show_about {
            // Modelled on PureMap's About: frameless window, custom
            // header with a hand-drawn ×, content optically centered.
            let mut open = self.show_about;
            let mut close_requested = false;
            egui::Window::new("关于 SmartReName")
                .id(egui::Id::new("about_window"))
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .title_bar(false)
                .fixed_size(egui::Vec2::new(480.0, 500.0))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::Vec2::new(ui.available_width(), 40.0),
                        egui::Layout::left_to_right(egui::Align::Center),
                        |ui| {
                            ui.label(egui::RichText::new("关于 SmartReName").size(14.0));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let (button_rect, response) = ui.allocate_exact_size(
                                        egui::Vec2::splat(30.0),
                                        egui::Sense::click(),
                                    );
                                    if response.hovered() {
                                        ui.painter().rect_filled(
                                            button_rect,
                                            4.0,
                                            ui.visuals().widgets.hovered.bg_fill,
                                        );
                                    }
                                    let center = button_rect.center();
                                    let half = 5.0;
                                    let stroke =
                                        egui::Stroke::new(1.5_f32, ui.visuals().text_color());
                                    ui.painter().line_segment(
                                        [
                                            center + egui::Vec2::splat(-half),
                                            center + egui::Vec2::splat(half),
                                        ],
                                        stroke,
                                    );
                                    ui.painter().line_segment(
                                        [
                                            center + egui::Vec2::new(-half, half),
                                            center + egui::Vec2::new(half, -half),
                                        ],
                                        stroke,
                                    );
                                    if response.clicked() {
                                        close_requested = true;
                                    }
                                },
                            );
                        },
                    );
                    ui.separator();
                    let body_size = ui.available_size();
                    ui.allocate_ui_with_layout(
                        body_size,
                        egui::Layout::top_down(egui::Align::Center)
                            .with_main_align(egui::Align::Center),
                        |ui| {
                            ui.add_space(10.0);
                            about_icon(ui);
                            ui.add_space(20.0);
                            ui.label(egui::RichText::new("SmartReName").size(22.0));
                            ui.add_space(6.0);
                            ui.label(
                                egui::RichText::new(format!(
                                    "版本 {}",
                                    env!("CARGO_PKG_VERSION")
                                ))
                                .size(16.0),
                            );
                            ui.add_space(12.0);
                            ui.label(
                                egui::RichText::new("纯粹、快速、可撤消的批量文件改名工具。")
                                    .size(13.5)
                                    .weak(),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                egui::RichText::new(
                                    "替换（含正则）· 序号 · 添加 · 删除 · 扩展命令\n繁简转换 · 日期命名 · 一键净化 · 操作记录",
                                )
                                .size(12.5)
                                .weak(),
                            );
                            ui.add_space(24.0);
                            ui.label(egui::RichText::new("By Mr.Ben").size(16.0));
                            ui.add_space(2.0);
                            ui.hyperlink_to(
                                egui::RichText::new("https://mrben.net").size(16.0),
                                "https://mrben.net",
                            );
                            ui.add_space(10.0);
                        },
                    );
                });
            if close_requested {
                open = false;
            }
            self.show_about = open;
        }

        if self.show_regex_help {
            egui::Window::new("正则表达式示例")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .default_width(660.0)
                .show(ctx, |ui| {
                    ui.label(
                        egui::RichText::new(
                            "都在「替换」页使用：勾选「使用正则表达式」，把查找和替换分别复制过去。正则只作用于主名，不动扩展名。",
                        )
                        .size(12.5)
                        .weak(),
                    );
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .max_height(520.0)
                        .show(ui, |ui| {
                            for ex in REGEX_EXAMPLES {
                                ui.horizontal(|ui| {
                                    ui.label(egui::RichText::new(ex.title).strong());
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if !ex.replacement.is_empty()
                                                && text_button(
                                                    ui,
                                                    egui::vec2(92.0, 28.0),
                                                    "复制替换",
                                                    false,
                                                )
                                            {
                                                ui.ctx().copy_text(ex.replacement.to_string());
                                                self.status =
                                                    format!("已复制替换内容：{}", ex.replacement);
                                            }
                                            if text_button(
                                                ui,
                                                egui::vec2(92.0, 28.0),
                                                "复制查找",
                                                false,
                                            ) {
                                                ui.ctx().copy_text(ex.find.to_string());
                                                self.status =
                                                    format!("已复制查找内容：{}", ex.find);
                                            }
                                        },
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(format!("查找：{}", ex.find))
                                        .monospace()
                                        .size(13.5),
                                );
                                if ex.replacement.is_empty() {
                                    ui.label(
                                        egui::RichText::new("替换：（留空，就是删掉）")
                                            .size(12.5)
                                            .weak(),
                                    );
                                } else {
                                    ui.label(
                                        egui::RichText::new(format!("替换：{}", ex.replacement))
                                            .monospace()
                                            .size(13.5),
                                    );
                                }
                                ui.label(egui::RichText::new(ex.explain).size(12.5).weak());
                                ui.label(egui::RichText::new(ex.sample).size(12.5).weak());
                                ui.add_space(4.0);
                                ui.separator();
                                ui.add_space(6.0);
                            }
                        });
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        if std_btn(ui, "关闭") {
                            self.show_regex_help = false;
                        }
                    });
                });
        }
    }
}

impl SmartRenameApp {
    /// Save the pipeline through the system save dialog (menu action —
    /// pipeline files no longer live inside every tab).
    fn save_ops_dialog(&mut self) {
        let Some(path) = choose_save_path(&self.ops_file_input) else {
            return;
        };
        match save_pipeline(&self.pipeline, &path) {
            Ok(()) => {
                self.ops_file_input = path.display().to_string();
                self.status = format!("操作记录已保存：{}", path.display());
            }
            Err(e) => self.status = e,
        }
    }

    /// Load a pipeline through the system open dialog (menu action).
    fn load_ops_dialog(&mut self) {
        let Some(path) = choose_open_path(&self.ops_file_input) else {
            return;
        };
        match load_pipeline(&path) {
            Ok(p) => {
                self.ops_file_input = path.display().to_string();
                self.pipeline = p;
                self.refresh_preview();
                self.status = format!("操作记录已载入：{}", path.display());
            }
            Err(e) => self.status = e,
        }
    }

    /// Pipeline list, shown under every tab in the scrolling ops panel.
    /// (Save/load moved to the 文件 menu as system file dialogs.)
    fn ops_record_section(&mut self, ui: &mut egui::Ui) {
        section(ui, "操作记录（按顺序执行）", |ui| {
            if self.pipeline.is_empty() {
                ui.label("还没有操作。在上方设置好后点「应用」。");
            }
            if !self.last_draft.is_empty() {
                ui.label(
                    egui::RichText::new(
                        "预览含本页正在编辑的设置（尚未加入记录），点右下角「应用」会一并执行。",
                    )
                    .weak(),
                );
            }
            let mut remove_at: Option<usize> = None;
            for (i, op) in self.pipeline.iter().enumerate() {
                // Custom row: the × button and the label ink are placed
                // on one shared center line (see LABEL_INK_NUDGE).
                let text = format!("{}. {}", i + 1, op.describe());
                let font = ui
                    .style()
                    .text_styles
                    .get(&egui::TextStyle::Body)
                    .cloned()
                    .unwrap_or_else(|| egui::FontId::proportional(14.5));
                let wrap_w = (ui.available_width() - 42.0).max(60.0);
                let galley = ui.fonts(|f| {
                    f.layout(text, font, ui.visuals().text_color(), wrap_w)
                });
                let height = galley.size().y.max(22.0);
                let (rect, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), height),
                    egui::Sense::hover(),
                );
                let btn_rect = egui::Rect::from_min_size(
                    egui::Pos2::new(rect.left(), rect.center().y - 11.0),
                    egui::vec2(26.0, 22.0),
                );
                if remove_btn_at(ui, btn_rect, egui::Id::new(("op_remove", i))) {
                    remove_at = Some(i);
                }
                ui.painter().galley(
                    egui::Pos2::new(
                        rect.left() + 34.0,
                        rect.center().y - galley.size().y / 2.0 + LABEL_INK_NUDGE,
                    ),
                    galley,
                    ui.visuals().text_color(),
                );
            }
            if let Some(i) = remove_at {
                self.pipeline.remove(i);
                self.refresh_preview();
            }
            if !self.pipeline.is_empty() && std_btn(ui, "清空操作") {
                self.pipeline.clear();
                self.refresh_preview();
            }
        });
    }
}

impl SmartRenameApp {
    fn wide_input(ui: &mut egui::Ui, text: &mut String) {
        ui.add_sized(
            [ui.available_width(), BTN_H],
            egui::TextEdit::singleline(text).vertical_align(egui::Align::Center),
        );
    }

    fn tab_overall(&mut self, ui: &mut egui::Ui) {
        section(ui, "大小写", |ui| {
            ui.label("文件名：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.case_name, CaseKind::Keep, "不变");
                radio_row(ui, &mut self.case_name, CaseKind::Upper, "大写");
                radio_row(ui, &mut self.case_name, CaseKind::Lower, "小写");
            });
            ui.label("扩展名：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.case_ext, CaseKind::Keep, "不变");
                radio_row(ui, &mut self.case_ext, CaseKind::Upper, "大写");
                radio_row(ui, &mut self.case_ext, CaseKind::Lower, "小写");
            });
            ui.add_space(4.0);
            if add_op_button(ui, self.case_name != CaseKind::Keep || self.case_ext != CaseKind::Keep) {
                let op = Operation::Case {
                    name: self.case_name,
                    ext: self.case_ext,
                };
                self.push_op(op);
            }
        });
        section(ui, "单词首字母大写", |ui| {
            check_row(ui, &mut self.capitalize, "启用");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.cap_space_only, true, "按空格分词");
                radio_row(ui, &mut self.cap_space_only, false, "按非字母数字分词");
            });
            ui.add_space(4.0);
            if add_op_button(ui, self.capitalize) {
                if self.capitalize {
                    let op = Operation::CapitalizeWords {
                        separator: if self.cap_space_only {
                            WordSeparator::SpaceOnly
                        } else {
                            WordSeparator::NonAlphanumeric
                        },
                    };
                    self.push_op(op);
                }
            }
        });
        section(ui, "更改扩展名", |ui| {
            check_row(ui, &mut self.change_ext_on, "启用（留空 = 删除扩展名）");
            Self::wide_input(ui, &mut self.change_ext_value);
            ui.add_space(4.0);
            if add_op_button(ui, self.change_ext_on) {
                if self.change_ext_on {
                    let v = self.change_ext_value.trim().to_string();
                    let op = Operation::ChangeExtension {
                        new_ext: if v.is_empty() { None } else { Some(v) },
                    };
                    self.push_op(op);
                }
            }
        });
        section(ui, "命名风格", |ui| {
            ui.horizontal_wrapped(|ui| {
                if std_btn(ui, "camelCase") {
                    self.push_op(Operation::NameStyle {
                        style: NameStyle::Camel,
                    });
                }
                if std_btn(ui, "PascalCase") {
                    self.push_op(Operation::NameStyle {
                        style: NameStyle::Pascal,
                    });
                }
                if std_btn(ui, "snake_case") {
                    self.push_op(Operation::NameStyle {
                        style: NameStyle::Snake,
                    });
                }
                if std_btn(ui, "kebab-case") {
                    self.push_op(Operation::NameStyle {
                        style: NameStyle::Kebab,
                    });
                }
            });
        });
        section(ui, "一键净化", |ui| {
            ui.label("去掉下载噪音：结尾的 (1)、- 副本、[网站.com]，全角转半角，多余空格合并。");
            ui.add_space(4.0);
            if text_button(ui, egui::vec2(150.0, 32.0), "一键净化", false) {
                self.push_op(Operation::CleanupName);
            }
        });
    }

    fn tab_replace(&mut self, ui: &mut egui::Ui) {
        section(ui, "查找与替换", |ui| {
            ui.label("查找：");
            Self::wide_input(ui, &mut self.rep_find);
            ui.label("替换为：");
            Self::wide_input(ui, &mut self.rep_replacement);
            check_row(ui, &mut self.rep_regex, "使用正则表达式");
            check_row(ui, &mut self.rep_case_sensitive, "区分大小写");
            check_row(ui, &mut self.rep_include_ext, "连扩展名一起替换");
            ui.add_space(4.0);
            if add_op_button(ui, !self.rep_find.is_empty()) {
                let op = Operation::Replace {
                    find: self.rep_find.clone(),
                    replacement: self.rep_replacement.clone(),
                    use_regex: self.rep_regex,
                    case_sensitive: self.rep_case_sensitive,
                    include_ext: self.rep_include_ext,
                };
                self.push_op(op);
            }
        });
        section(ui, "正则示例（点一下填入）", |ui| {
            // Only the few most-used examples live here (the first
            // entries of the library); the full, growing set is in
            // 帮助 → 正则表达式示例, where each has copy buttons.
            for ex in REGEX_EXAMPLES.iter().take(4) {
                if text_button(ui, egui::vec2(ui.available_width(), 30.0), ex.title, false) {
                    self.rep_find = ex.find.to_string();
                    self.rep_replacement = ex.replacement.to_string();
                    self.rep_regex = true;
                    self.rep_case_sensitive = true;
                    self.rep_include_ext = false;
                }
                ui.add(egui::Label::new(ex.explain).wrap());
                ui.small(ex.sample);
                ui.add_space(8.0);
            }
            ui.label(
                egui::RichText::new("更多示例在菜单「帮助 → 正则表达式示例」里，每条都能复制。")
                    .size(12.5)
                    .weak(),
            );
        });
    }

    fn tab_numbering(&mut self, ui: &mut egui::Ui) {
        section(ui, "序号设置", |ui| {
            check_row(ui, &mut self.num_on, "启用");
            egui::Grid::new("num_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("起始：");
                    num_input_i64(ui, "num_start", &mut self.num_start, i64::MIN..=i64::MAX);
                    ui.end_row();
                    ui.label("步长：");
                    num_input_i64(ui, "num_step", &mut self.num_step, i64::MIN..=i64::MAX);
                    ui.end_row();
                    ui.label("位数（不足补 0）：");
                    num_input_usize(ui, "num_width", &mut self.num_width, 0..=12);
                    ui.end_row();
                });
            ui.add_space(4.0);
            ui.label("样式：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.num_style, NumberStyle::Numeric, "数字");
                radio_row(ui, &mut self.num_style, NumberStyle::AlphaUpper, "大写字母");
                radio_row(ui, &mut self.num_style, NumberStyle::AlphaLower, "小写字母");
            });
            ui.label("位置：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.num_placement, NumberPlacement::Prefix, "放前面");
                radio_row(ui, &mut self.num_placement, NumberPlacement::Suffix, "放后面");
                radio_row(ui, &mut self.num_placement, NumberPlacement::Template, "用模板");
            });
            ui.label("模板（用 <#> 代表序号，如 ep-<#>）：");
            Self::wide_input(ui, &mut self.num_template);
            ui.add_space(4.0);
            if add_op_button(ui, self.num_on) {
                if self.num_on {
                    let op = self.numbering_op();
                    self.push_op(op);
                }
            }
        });
    }

    fn tab_add(&mut self, ui: &mut egui::Ui) {
        section(ui, "加前缀", |ui| {
            Self::wide_input(ui, &mut self.add_prefix);
            ui.add_space(4.0);
            if add_op_button(ui, !self.add_prefix.is_empty()) {
                let op = Operation::AddPrefix {
                    text: self.add_prefix.clone(),
                };
                self.push_op(op);
            }
        });
        section(ui, "加后缀（扩展名之前）", |ui| {
            Self::wide_input(ui, &mut self.add_suffix);
            ui.add_space(4.0);
            if add_op_button(ui, !self.add_suffix.is_empty()) {
                let op = Operation::AddSuffix {
                    text: self.add_suffix.clone(),
                };
                self.push_op(op);
            }
        });
        section(ui, "插入", |ui| {
            ui.horizontal(|ui| {
                ui.label("在第几个字符处插入：");
                num_input_usize(ui, "insert_pos", &mut self.insert_pos, 1..=999);
            });
            ui.label("插入内容：");
            Self::wide_input(ui, &mut self.insert_text);
            ui.add_space(4.0);
            if add_op_button(ui, !self.insert_text.is_empty()) {
                let op = Operation::InsertAt {
                    position: self.insert_pos,
                    text: self.insert_text.clone(),
                };
                self.push_op(op);
            }
        });
    }

    fn tab_delete(&mut self, ui: &mut egui::Ui) {
        section(ui, "按位置删除", |ui| {
            egui::Grid::new("del_grid")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label("从第几个字符起删：");
                    num_input_usize(ui, "del_start", &mut self.del_start, 1..=999);
                    ui.end_row();
                    ui.label("删几个字符：");
                    num_input_usize(ui, "del_count", &mut self.del_count, 0..=999);
                    ui.end_row();
                });
            ui.add_space(4.0);
            if add_op_button(ui, self.del_count > 0) {
                let op = Operation::DeleteRange {
                    start: self.del_start,
                    count: self.del_count,
                };
                self.push_op(op);
            }
        });
        section(ui, "删除字串", |ui| {
            ui.label("要删除的字串：");
            Self::wide_input(ui, &mut self.del_text);
            check_row(ui, &mut self.del_case_sensitive, "区分大小写");
            ui.add_space(4.0);
            if add_op_button(ui, !self.del_text.is_empty()) {
                let op = Operation::DeleteText {
                    text: self.del_text.clone(),
                    case_sensitive: self.del_case_sensitive,
                };
                self.push_op(op);
            }
        });
    }

    fn tab_more(&mut self, ui: &mut egui::Ui) {
        section(ui, "扩展命令（预设）", |ui| {
            // One row: dropdown flexes to the remaining width, the
            // apply button is a fixed size and just says 应用 — the
            // section title already says what it applies.
            ui.horizontal(|ui| {
                let btn_w = 96.0;
                let dd_w = (ui.available_width() - btn_w - 8.0).max(120.0);
                script_preset_dropdown(ui, &mut self.script_preset, dd_w);
                // No separate 启用 checkbox: "请选择" in the dropdown
                // is the off state, and the button stays grey until a
                // real preset is chosen.
                if text_button_enabled(
                    ui,
                    egui::vec2(btn_w, BTN_H),
                    "应用",
                    false,
                    self.script_preset.is_some(),
                ) {
                    if let Some(preset) = self.script_preset {
                        self.push_op(Operation::Script {
                            preset,
                            target: ScriptTarget::Stem,
                        });
                    }
                }
            });
        });
        section(ui, "文字转换", |ui| {
            ui.horizontal(|ui| {
                if std_btn(ui, "繁体转简体") {
                    self.push_op(Operation::Encoding {
                        kind: EncodingConvert::TraditionalToSimplified,
                    });
                }
                if std_btn(ui, "简体转繁体") {
                    self.push_op(Operation::Encoding {
                        kind: EncodingConvert::SimplifiedToTraditional,
                    });
                }
            });
        });
        section(ui, "文件日期", |ui| {
            ui.horizontal_wrapped(|ui| {
                if std_btn(ui, "修改时间设为现在") {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    self.push_op(Operation::SetTimes {
                        spec: TimeSpec {
                            modified_unix: Some(now),
                            accessed_unix: Some(now),
                            created_unix: None,
                        },
                    });
                }
                if std_btn(ui, "创建时间设为现在") {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    // macOS requires birthtime <= mtime, so the
                    // modification time moves along with it.
                    self.push_op(Operation::SetTimes {
                        spec: TimeSpec {
                            modified_unix: Some(now),
                            accessed_unix: None,
                            created_unix: Some(now),
                        },
                    });
                }
            });
        });
        section(ui, "日期命名", |ui| {
            check_row(ui, &mut self.date_on, "启用");
            ui.label("时间来源：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.date_source, DateSource::Exif, "拍摄时间");
                radio_row(ui, &mut self.date_source, DateSource::Modified, "修改时间");
                radio_row(ui, &mut self.date_source, DateSource::Created, "创建时间");
            });
            ui.label("格式：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.date_format, DateFormat::Compact, "20240815_143022");
                radio_row(ui, &mut self.date_format, DateFormat::Dashed, "2024-08-15_14-30-22");
            });
            ui.label("方式：");
            ui.horizontal(|ui| {
                radio_row(ui, &mut self.date_mode, DateMode::Replace, "整名替换");
                radio_row(ui, &mut self.date_mode, DateMode::Prefix, "作前缀");
                radio_row(ui, &mut self.date_mode, DateMode::Suffix, "作后缀");
            });
            ui.add_space(4.0);
            if add_op_button(ui, self.date_on) {
                if self.date_on {
                    self.push_op(Operation::DateName {
                        source: self.date_source,
                        format: self.date_format,
                        mode: self.date_mode,
                    });
                }
            }
            ui.label("拍摄时间读照片的 EXIF；没有 EXIF 时自动用修改时间。");
        });
        section(ui, "按文件夹命名", |ui| {
            ui.horizontal_wrapped(|ui| {
                if std_btn(ui, "以父文件夹命名") {
                    self.push_op(Operation::NameFromParent { numbered: false });
                }
                if std_btn(ui, "父文件夹名+序号") {
                    self.push_op(Operation::NameFromParent { numbered: true });
                }
            });
        });
    }
}

// ---------------------------------------------------------------------------
// macOS native menu bar (muda). The menus live in the system menu bar at
// the top of the screen, as a Mac app should. On other platforms the app
// falls back to an in-window menu bar with the same items.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
type NativeMenuHandle = NativeMenu;
#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
struct NativeMenuHandle;

/// Open a URL in the user's default browser (Help menu links).
#[cfg(target_os = "macos")]
fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

#[cfg(target_os = "macos")]
struct NativeMenu {
    _menu: muda::Menu,
    add_files: muda::MenuItem,
    add_folder: muda::MenuItem,
    clear_list: muda::MenuItem,
    select_all: muda::MenuItem,
    invert: muda::MenuItem,
    save_ops: muda::MenuItem,
    load_ops: muda::MenuItem,
    undo: muda::MenuItem,
    replay: muda::MenuItem,
    clear_ops: muda::MenuItem,
    theme_follow: muda::CheckMenuItem,
    theme_light: muda::CheckMenuItem,
    theme_dark: muda::CheckMenuItem,
    about: muda::MenuItem,
    regex_help: muda::MenuItem,
    homepage: muda::MenuItem,
    github: muda::MenuItem,
    last_undo: bool,
    last_replay: bool,
    last_clear: bool,
    last_follow: bool,
    last_dark_mode: bool,
}

#[cfg(target_os = "macos")]
fn build_native_menu() -> Option<NativeMenu> {
    use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};

    let menu = Menu::new();

    // App menu (first menu, named after the app by macOS convention).
    let app_menu = Submenu::new("SmartReName", true);
    // Custom item (not the predefined about panel) so it opens the
    // app's own beautified About window.
    let about = MenuItem::new("关于 SmartReName", true, None);
    app_menu.append(&PredefinedMenuItem::quit(None)).ok()?;
    menu.append(&app_menu).ok()?;

    // 文件
    let file_menu = Submenu::new("文件", true);
    let add_files = MenuItem::new(
        "添加文件…",
        true,
        Some(muda::accelerator::Accelerator::new(
            Some(muda::accelerator::Modifiers::META),
            muda::accelerator::Code::KeyO,
        )),
    );
    let add_folder = MenuItem::new(
        "添加文件夹…",
        true,
        Some(muda::accelerator::Accelerator::new(
            Some(muda::accelerator::Modifiers::META | muda::accelerator::Modifiers::SHIFT),
            muda::accelerator::Code::KeyO,
        )),
    );
    let clear_list = MenuItem::new("清空列表", true, None);
    let select_all = MenuItem::new("全选", true, None);
    let invert = MenuItem::new("反选", true, None);
    let save_ops = MenuItem::new("保存操作记录…", true, None);
    let load_ops = MenuItem::new("载入操作记录…", true, None);
    file_menu.append(&add_files).ok()?;
    file_menu.append(&add_folder).ok()?;
    file_menu.append(&PredefinedMenuItem::separator()).ok()?;
    file_menu.append(&clear_list).ok()?;
    file_menu.append(&select_all).ok()?;
    file_menu.append(&invert).ok()?;
    file_menu.append(&PredefinedMenuItem::separator()).ok()?;
    file_menu.append(&save_ops).ok()?;
    file_menu.append(&load_ops).ok()?;
    menu.append(&file_menu).ok()?;

    // 编辑
    let edit_menu = Submenu::new("编辑", true);
    let undo = MenuItem::new("撤消更名", false, None);
    let replay = MenuItem::new("重演上一次操作", false, None);
    let clear_ops = MenuItem::new("清空操作记录", false, None);
    edit_menu.append(&undo).ok()?;
    edit_menu.append(&replay).ok()?;
    edit_menu.append(&PredefinedMenuItem::separator()).ok()?;
    edit_menu.append(&clear_ops).ok()?;
    menu.append(&edit_menu).ok()?;

    // 显示 (theme lives here)
    let view_menu = Submenu::new("显示", true);
    let theme_label = MenuItem::new("主题", false, None);
    let theme_follow = CheckMenuItem::new("跟随系统", true, true, None);
    let theme_light = CheckMenuItem::new("浅色", true, false, None);
    let theme_dark = CheckMenuItem::new("深色", true, false, None);
    view_menu.append(&theme_label).ok()?;
    view_menu.append(&theme_follow).ok()?;
    view_menu.append(&theme_light).ok()?;
    view_menu.append(&theme_dark).ok()?;
    menu.append(&view_menu).ok()?;

    // 帮助 (About lives here, together with the homepage and
    // GitHub links).
    let help_menu = Submenu::new("帮助", true);
    let regex_help = MenuItem::new("正则表达式示例", true, None);
    let homepage = MenuItem::new("作者主页", true, None);
    let github = MenuItem::new("在GitHub上查看", true, None);
    help_menu.append(&regex_help).ok()?;
    help_menu.append(&PredefinedMenuItem::separator()).ok()?;
    help_menu.append(&about).ok()?;
    help_menu.append(&homepage).ok()?;
    help_menu.append(&github).ok()?;
    menu.append(&help_menu).ok()?;

    menu.init_for_nsapp();

    Some(NativeMenu {
        _menu: menu,
        add_files,
        add_folder,
        clear_list,
        select_all,
        invert,
        save_ops,
        load_ops,
        undo,
        replay,
        clear_ops,
        theme_follow,
        theme_light,
        theme_dark,
        about,
        regex_help,
        homepage,
        github,
        last_undo: false,
        last_replay: false,
        last_clear: false,
        last_follow: true,
        last_dark_mode: false,
    })
}

#[cfg(target_os = "macos")]
impl SmartRenameApp {
    /// Initialize the native menu on first frame; afterwards, each frame,
    /// drain pending menu events and keep item states in sync.
    fn native_menu_update(&mut self, ctx: &egui::Context) {
        if self.native_menu.is_none() {
            self.native_menu = build_native_menu();
        }
        // Handle menu events. Ids are compared against the stored items.
        while let Ok(event) = muda::MenuEvent::receiver().try_recv() {
            let Some(m) = &self.native_menu else { break };
            let id = event.id();
            if id == m.add_files.id() {
                self.add_files_dialog();
            } else if id == m.add_folder.id() {
                self.add_folder_dialog();
            } else if id == m.clear_list.id() {
                self.rows.clear();
                self.refresh_preview();
            } else if id == m.select_all.id() {
                for r in &mut self.rows {
                    r.checked = true;
                }
                self.refresh_preview();
            } else if id == m.invert.id() {
                for r in &mut self.rows {
                    r.checked = !r.checked;
                }
                self.refresh_preview();
            } else if id == m.save_ops.id() {
                self.save_ops_dialog();
            } else if id == m.load_ops.id() {
                self.load_ops_dialog();
            } else if id == m.undo.id() {
                self.do_undo();
            } else if id == m.replay.id() {
                self.do_replay();
            } else if id == m.clear_ops.id() {
                self.pipeline.clear();
                self.refresh_preview();
            } else if id == m.theme_follow.id() {
                self.follow_system_theme = true;
            } else if id == m.theme_light.id() {
                self.follow_system_theme = false;
                self.dark_mode = false;
                apply_style(ctx, false);
            } else if id == m.theme_dark.id() {
                self.follow_system_theme = false;
                self.dark_mode = true;
                apply_style(ctx, true);
            } else if id == m.about.id() {
                self.show_about = true;
            } else if id == m.regex_help.id() {
                self.show_regex_help = true;
            } else if id == m.homepage.id() {
                open_url("https://mrben.net");
            } else if id == m.github.id() {
                open_url("https://github.com/i-love-osaka/SmartRename");
            }
        }
        // Sync enabled / checked states when they change.
        if let Some(m) = &mut self.native_menu {
            let undo_on = self.undo_log.is_some();
            if undo_on != m.last_undo {
                m.undo.set_enabled(undo_on);
                m.last_undo = undo_on;
            }
            let replay_on = self.last_pipeline.is_some();
            if replay_on != m.last_replay {
                m.replay.set_enabled(replay_on);
                m.last_replay = replay_on;
            }
            let clear_on = !self.pipeline.is_empty();
            if clear_on != m.last_clear {
                m.clear_ops.set_enabled(clear_on);
                m.last_clear = clear_on;
            }
            if self.follow_system_theme != m.last_follow
                || self.dark_mode != m.last_dark_mode
            {
                m.theme_follow.set_checked(self.follow_system_theme);
                m.theme_light
                    .set_checked(!self.follow_system_theme && !self.dark_mode);
                m.theme_dark
                    .set_checked(!self.follow_system_theme && self.dark_mode);
                m.last_follow = self.follow_system_theme;
                m.last_dark_mode = self.dark_mode;
            }
        }
    }
}
