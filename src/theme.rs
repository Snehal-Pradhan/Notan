use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle,
    Visuals,
};

pub const BG: Color32 = Color32::from_rgb(0x16, 0x16, 0x18);
pub const SIDEBAR_BG: Color32 = Color32::from_rgb(0x10, 0x10, 0x12);
pub const TOP_BAR_BG: Color32 = Color32::from_rgb(0x0c, 0x11, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x1c, 0x1c, 0x1f);
pub const PANEL_RAISED: Color32 = Color32::from_rgb(0x21, 0x21, 0x24);
pub const CODE_BG: Color32 = Color32::from_rgb(0x13, 0x13, 0x15);
pub const QUOTE_BG: Color32 = Color32::from_rgb(0x1b, 0x1b, 0x1e);
pub const LINE: Color32 = Color32::from_rgb(0x30, 0x30, 0x35);
pub const LINE_SOFT: Color32 = Color32::from_rgb(0x26, 0x26, 0x2a);
pub const TEXT: Color32 = Color32::from_rgb(0xb8, 0xbb, 0xc0);
pub const TEXT_BRIGHT: Color32 = Color32::from_rgb(0xe4, 0xe6, 0xea);
pub const DIM: Color32 = Color32::from_rgb(0x8f, 0x95, 0x9c);
pub const FAINT: Color32 = Color32::from_rgb(0x7d, 0x83, 0x8b);
/// Single accent: cold off-white. Selection, active marks, checked tasks.
pub const ACCENT: Color32 = Color32::from_rgb(0xe4, 0xe6, 0xea);
/// Reserved for failures and conflicts only.
pub const DANGER: Color32 = Color32::from_rgb(0xc4, 0x7a, 0x58);
pub const HOVER: Color32 = Color32::from_rgb(0x1e, 0x1e, 0x21);
pub const ACTIVE: Color32 = Color32::from_rgb(0x28, 0x28, 0x2c);
pub const SELECTED_ROW: Color32 = Color32::from_rgb(0x23, 0x23, 0x27);
/// Low-opacity grey overlay: the text under a selection stays readable.
pub const TEXT_SELECTION: Color32 = Color32::from_rgba_premultiplied(42, 44, 48, 60);

// Muted syntax colours: enough hue to parse code, still low-saturation.
pub const SYN_KEYWORD: Color32 = Color32::from_rgb(0x8f, 0xa6, 0xbf);
pub const SYN_TYPE: Color32 = Color32::from_rgb(0xb3, 0xa3, 0xc4);
pub const SYN_FUNCTION: Color32 = Color32::from_rgb(0xd6, 0xdb, 0xe2);
pub const SYN_STRING: Color32 = Color32::from_rgb(0xa3, 0xb5, 0x8e);
pub const SYN_NUMBER: Color32 = Color32::from_rgb(0xc3, 0xa8, 0x7d);
pub const SYN_COMMENT: Color32 = Color32::from_rgb(0x7f, 0x85, 0x8e);

// Heading colours for `##[name]` markers and TYPE presets.
pub const HEAD_GRAY: Color32 = Color32::from_rgb(0x9a, 0xa0, 0xa8);
pub const HEAD_RED: Color32 = Color32::from_rgb(0xd0, 0x8a, 0x86);
pub const HEAD_ORANGE: Color32 = Color32::from_rgb(0xd3, 0xa0, 0x6a);
pub const HEAD_YELLOW: Color32 = Color32::from_rgb(0xc9, 0xbd, 0x7e);
pub const HEAD_GREEN: Color32 = Color32::from_rgb(0x97, 0xb9, 0x8a);
pub const HEAD_TEAL: Color32 = Color32::from_rgb(0x82, 0xb7, 0xb1);
pub const HEAD_BLUE: Color32 = Color32::from_rgb(0x8f, 0xb2, 0xd4);
pub const HEAD_PURPLE: Color32 = Color32::from_rgb(0xb6, 0xa0, 0xcf);
pub const HEAD_PINK: Color32 = Color32::from_rgb(0xcf, 0x9b, 0xb6);
pub const HEAD_WHITE: Color32 = Color32::from_rgb(0xe4, 0xe6, 0xea);

pub fn heading_color(name: &str) -> Option<Color32> {
    match name.trim().to_lowercase().as_str() {
        "gray" | "grey" => Some(HEAD_GRAY),
        "red" => Some(HEAD_RED),
        "orange" => Some(HEAD_ORANGE),
        "yellow" => Some(HEAD_YELLOW),
        "green" => Some(HEAD_GREEN),
        "teal" => Some(HEAD_TEAL),
        "blue" => Some(HEAD_BLUE),
        "purple" => Some(HEAD_PURPLE),
        "pink" => Some(HEAD_PINK),
        "white" => Some(HEAD_WHITE),
        _ => None,
    }
}

/// A document `TYPE:` maps to a default heading colour.
pub fn type_color(kind: &str) -> Option<Color32> {
    let kind = kind.trim().to_lowercase();
    let color = if kind.contains("setting") {
        HEAD_GRAY
    } else if kind.contains("future") || kind.contains("idea") {
        HEAD_PURPLE
    } else if kind.contains("objective") || kind.contains("current") {
        HEAD_ORANGE
    } else if kind.contains("architect") || kind.contains("design") {
        HEAD_BLUE
    } else if kind.contains("test") {
        HEAD_GREEN
    } else if kind.contains("doc") {
        HEAD_TEAL
    } else {
        return None;
    };
    Some(color)
}

pub const SIDEBAR_WIDTH: f32 = 250.0;
pub const TOP_BAR_HEIGHT: f32 = 38.0;
pub const STATUS_BAR_HEIGHT: f32 = 24.0;

pub const BODY_SIZE: f32 = 14.5;
pub const SMALL_SIZE: f32 = 11.5;
pub const MONO_SIZE: f32 = 14.0;

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);

    let mut style = (*ctx.style()).clone();
    style.visuals = visuals();
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(0.0, 0.0);
    style.spacing.scroll = egui::style::ScrollStyle {
        floating: true,
        bar_width: 6.0,
        floating_width: 3.0,
        floating_allocated_width: 3.0,
        handle_min_length: 28.0,
        bar_inner_margin: 2.0,
        bar_outer_margin: 0.0,
        foreground_color: false,
        dormant_background_opacity: 0.0,
        active_background_opacity: 0.0,
        interact_background_opacity: 0.6,
        dormant_handle_opacity: 0.0,
        active_handle_opacity: 0.35,
        interact_handle_opacity: 0.9,
    };
    style.text_styles = [
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(SMALL_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONO_SIZE, FontFamily::Monospace),
        ),
        (
            TextStyle::Heading,
            FontId::new(20.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(13.0, FontFamily::Proportional),
        ),
    ]
    .into();
    ctx.set_style(style);
}

const JETBRAINS_MONO: &[u8] = include_bytes!("../assets/JetBrainsMono-Regular.ttf");

const ICONS: &[(&str, &[u8])] = &[
    ("folder", include_bytes!("../assets/icons/folder.rgba")),
    ("file", include_bytes!("../assets/icons/file.rgba")),
    ("file-plus", include_bytes!("../assets/icons/file-plus.rgba")),
    ("folder-plus", include_bytes!("../assets/icons/folder-plus.rgba")),
    ("chevron-right", include_bytes!("../assets/icons/chevron-right.rgba")),
    ("chevron-down", include_bytes!("../assets/icons/chevron-down.rgba")),
    ("x", include_bytes!("../assets/icons/x.rgba")),
    ("copy", include_bytes!("../assets/icons/copy.rgba")),
    ("copy-check", include_bytes!("../assets/icons/copy-check.rgba")),
    ("settings", include_bytes!("../assets/icons/settings.rgba")),
];

/// Tabler-derived 64 px masks, tinted at draw time.
pub fn icon(ctx: &egui::Context, name: &str) -> egui::TextureId {
    let id = egui::Id::new(("notan-icon", name));
    if let Some(texture) = ctx.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture.id();
    }
    let Some((_, bytes)) = ICONS.iter().find(|(key, _)| *key == name) else {
        return egui::TextureId::default();
    };
    let texture = ctx.load_texture(
        format!("notan-icon-{name}"),
        egui::ColorImage::from_rgba_unmultiplied([64, 64], bytes),
        egui::TextureOptions::LINEAR,
    );
    let texture_id = texture.id();
    ctx.data_mut(|data| data.insert_temp(id, texture));
    texture_id
}

pub fn paint_icon(
    painter: &egui::Painter,
    texture: egui::TextureId,
    rect: egui::Rect,
    color: Color32,
) {
    painter.image(
        texture,
        rect,
        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        color,
    );
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "jetbrains_mono".to_owned(),
        Arc::new(FontData::from_static(JETBRAINS_MONO)),
    );
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .insert(0, "jetbrains_mono".to_owned());

    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "jetbrains_mono".to_owned());

    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNS.ttf") {
        fonts
            .font_data
            .insert("sf_text".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .push("sf_text".to_owned());
    }

    if let Ok(bytes) = std::fs::read("/System/Library/Fonts/SFNSMono.ttf") {
        fonts
            .font_data
            .insert("sf_mono".to_owned(), Arc::new(FontData::from_owned(bytes)));
        fonts
            .families
            .entry(FontFamily::Monospace)
            .or_default()
            .push("sf_mono".to_owned());
    }

    let mut preview = Vec::new();
    if fonts.font_data.contains_key("sf_text") {
        preview.push("sf_text".to_owned());
    }
    preview.push("jetbrains_mono".to_owned());
    fonts
        .families
        .insert(FontFamily::Name("preview".into()), preview);

    ctx.set_fonts(fonts);
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();

    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = CODE_BG;
    v.faint_bg_color = HOVER;
    v.code_bg_color = CODE_BG;
    v.override_text_color = Some(TEXT);
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.hyperlink_color = TEXT_BRIGHT;
    // egui paints selected glyphs with `selection.stroke.color`; a transparent
    // stroke would erase the text, so keep it at body brightness.
    v.selection = egui::style::Selection {
        bg_fill: TEXT_SELECTION,
        stroke: Stroke::new(1.0, TEXT_BRIGHT),
    };
    v.extreme_bg_color = BG;
    v.text_cursor.stroke = Stroke::new(1.5, TEXT_BRIGHT);

    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = Stroke::NONE;
        w.expansion = 0.0;
        w.fg_stroke = Stroke::new(1.0, TEXT);
        w.bg_fill = Color32::TRANSPARENT;
        w.weak_bg_fill = Color32::TRANSPARENT;
    }
    v.widgets.inactive.weak_bg_fill = PANEL;
    // Scroll handles read `bg_fill`; keep widgets transparent elsewhere.
    v.widgets.inactive.bg_fill = Color32::from_rgb(0x33, 0x33, 0x39);
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x45, 0x45, 0x4c);
    v.widgets.active.bg_fill = Color32::from_rgb(0x53, 0x53, 0x5b);
    v.widgets.hovered.weak_bg_fill = ACTIVE;
    v.widgets.hovered.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.widgets.active.weak_bg_fill = ACTIVE;
    v.widgets.active.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.widgets.open.fg_stroke = Stroke::new(1.0, TEXT_BRIGHT);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE_SOFT);

    v
}

/// Show a horizontal hairline at the bottom of the given rect.
pub fn hairline_bottom(ui: &egui::Ui, rect: egui::Rect) {
    ui.painter().line_segment(
        [
            egui::pos2(rect.left(), rect.bottom()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        Stroke::new(1.0, LINE_SOFT),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_and_style_build() {
        let ctx = egui::Context::default();
        install(&ctx);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("notan — ⌘K · café ☑");
            });
        });
    }
}
