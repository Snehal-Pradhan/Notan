use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, FontFamily, FontId, Label, Layout,
    Margin, Rect, RichText, Sense, Stroke, TextFormat, TextWrapMode, Ui,
};
use eframe::egui::text::LayoutJob;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

use crate::highlight::{self, Kind};
use crate::theme;

#[derive(Clone, Debug, Default)]
pub struct Span {
    pub text: String,
    pub strong: bool,
    pub em: bool,
    pub strike: bool,
    pub link: bool,
    pub href: String,
    pub code: bool,
}

#[derive(Clone, Debug)]
pub enum Marker {
    Bullet,
    Ordered(u64),
    Task(bool),
}

#[derive(Clone, Debug)]
pub enum Block {
    Heading {
        level: u8,
        spans: Vec<Span>,
        quote: usize,
        color: Option<Color32>,
    },
    Text {
        spans: Vec<Span>,
        indent: usize,
        marker: Option<Marker>,
        quote: usize,
    },
    Code {
        lang: String,
        text: String,
        tokens: Vec<highlight::Token>,
        quote: usize,
    },
    Image {
        src: String,
        alt: String,
        quote: usize,
    },
    Rule {
        quote: usize,
    },
    Table {
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
        quote: usize,
    },
}

impl Block {
    fn quote(&self) -> usize {
        match self {
            Self::Heading { quote, .. }
            | Self::Code { quote, .. }
            | Self::Image { quote, .. }
            | Self::Rule { quote }
            | Self::Table { quote, .. } => *quote,
            Self::Text { quote, .. } => *quote,
        }
    }
}

#[derive(Default)]
struct ListState {
    ordered: bool,
    next: u64,
}

#[derive(Default)]
struct TableBuild {
    head: Vec<Vec<Span>>,
    rows: Vec<Vec<Vec<Span>>>,
    in_head: bool,
    row: Vec<Vec<Span>>,
    cell: Vec<Span>,
    cell_open: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DocMeta {
    pub kind: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug)]
pub struct Parsed {
    pub blocks: Vec<Block>,
    pub meta: DocMeta,
}

/// Read leading `TYPE:` and `COLOR:` directives. They are not rendered.
fn split_meta(src: &str) -> (DocMeta, &str) {
    let mut meta = DocMeta::default();
    let mut offset = 0;

    // YAML-style front matter: --- ... ---
    let mut lines = src.split_inclusive('\n');
    if lines.next().is_some_and(|line| line.trim() == "---") {
        let mut block = String::new();
        let mut consumed = src.find('\n').map_or(src.len(), |i| i + 1);
        let mut closed = false;
        for line in lines {
            consumed += line.len();
            if line.trim() == "---" {
                closed = true;
                break;
            }
            block.push_str(line);
        }
        if closed {
            for line in block.lines() {
                let trimmed = line.trim();
                let lower = trimmed.to_lowercase();
                if let Some(value) = lower.strip_prefix("type:") {
                    let _ = value;
                    meta.kind = Some(trimmed[5..].trim().to_owned());
                } else if let Some(value) = lower.strip_prefix("color:") {
                    let _ = value;
                    meta.color = Some(trimmed[6..].trim().to_owned());
                }
            }
            let mut rest = &src[consumed..];
            while rest.starts_with('\n') {
                rest = &rest[1..];
            }
            return (meta, rest);
        }
    }

    let mut seen = false;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            if seen {
                offset += line.len();
                continue;
            }
            break;
        }
        let lower = trimmed.to_lowercase();
        if let Some(value) = lower.strip_prefix("type:") {
            let _ = value;
            meta.kind = Some(trimmed[5..].trim().to_owned());
            offset += line.len();
            seen = true;
        } else if let Some(value) = lower.strip_prefix("color:") {
            let _ = value;
            meta.color = Some(trimmed[6..].trim().to_owned());
            offset += line.len();
            seen = true;
        } else {
            break;
        }
    }

    let mut rest = &src[offset..];
    while rest.starts_with('\n') {
        rest = &rest[1..];
    }
    (meta, rest)
}

/// Allow `##[blue] Title` without the space CommonMark requires.
fn normalize_headings(src: &str) -> String {
    let mut out = String::with_capacity(src.len() + 16);
    for line in src.split_inclusive('\n') {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some('[') {
            out.push_str(&line[..hashes]);
            out.push(' ');
            out.push_str(&line[hashes..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

fn take_heading_color(spans: &mut Vec<Span>) -> Option<Color32> {
    let first = spans.first()?;
    let text = first.text.clone();
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('[')?;
    let end = rest.find(']')?;
    let color = theme::heading_color(&rest[..end])?;
    let keep = text.len() - trimmed.len() + 1 + end + 1;
    if let Some(first) = spans.first_mut() {
        first.text = text[keep..].trim_start().to_owned();
    }
    if spans.first().is_some_and(|span| span.text.is_empty()) {
        spans.remove(0);
    }
    Some(color)
}

pub fn parse(src: &str) -> Parsed {
    let (meta, body) = split_meta(src);
    let source = normalize_headings(body);
    let default_color = meta
        .color
        .as_deref()
        .and_then(theme::heading_color)
        .or_else(|| meta.kind.as_deref().and_then(theme::type_color));

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let mut blocks = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let mut strong = 0usize;
    let mut em = 0usize;
    let mut strike = 0usize;
    let mut link = 0usize;
    let mut hrefs: Vec<String> = Vec::new();
    let mut image = 0usize;
    let mut alt = String::new();
    let mut image_src = String::new();
    let mut pending_image: Option<(String, String)> = None;
    let mut lists: Vec<ListState> = Vec::new();
    let mut marker: Option<Marker> = None;
    let mut quote = 0usize;
    let mut heading: Option<(u8, usize)> = None;
    let mut code: Option<(String, String, usize)> = None;
    let mut table: Option<TableBuild> = None;

    for event in Parser::new_ext(&source, options) {
        match event {
            Event::Start(Tag::Paragraph) => spans.clear(),
            Event::End(TagEnd::Paragraph) => {
                if let Some((src, alt)) = pending_image.take() {
                    blocks.push(Block::Image { src, alt, quote });
                } else {
                    flush_text(
                        &mut blocks,
                        &mut spans,
                        &mut marker,
                        lists.len().saturating_sub(1) * 18,
                        quote,
                    );
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some((level as u8, quote));
                spans.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if !spans.is_empty() {
                    let (level, quote) = heading.take().unwrap_or((1, 0));
                    let mut heading_spans = std::mem::take(&mut spans);
                    let color = take_heading_color(&mut heading_spans).or(if level >= 2 {
                        default_color
                    } else {
                        None
                    });
                    blocks.push(Block::Heading {
                        level,
                        quote,
                        spans: heading_spans,
                        color,
                    });
                } else {
                    heading = None;
                }
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => quote = quote.saturating_sub(1),
            Event::Start(Tag::List(start)) => {
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
                lists.push(ListState {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                });
            }
            Event::End(TagEnd::List(_)) => {
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
                if let Some(list) = lists.last_mut() {
                    marker = Some(if list.ordered {
                        let n = list.next;
                        list.next += 1;
                        Marker::Ordered(n)
                    } else {
                        Marker::Bullet
                    });
                }
            }
            Event::End(TagEnd::Item) => {
                // Tight list items are emitted without a Paragraph wrapper.
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
                let lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                code = Some((lang, String::new(), quote));
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some((lang, text, quote)) = code.take() {
                    let tokens = highlight::tokens(&lang, text.trim_end_matches('\n'));
                    blocks.push(Block::Code {
                        lang,
                        text,
                        tokens,
                        quote,
                    });
                }
            }
            Event::Start(Tag::Emphasis) => em += 1,
            Event::End(TagEnd::Emphasis) => em = em.saturating_sub(1),
            Event::Start(Tag::Strong) => strong += 1,
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => strike += 1,
            Event::End(TagEnd::Strikethrough) => strike = strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => {
                link += 1;
                hrefs.push(dest_url.to_string());
            }
            Event::End(TagEnd::Link) => {
                link = link.saturating_sub(1);
                hrefs.pop();
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                image += 1;
                alt.clear();
                image_src = dest_url.to_string();
            }
            Event::End(TagEnd::Image) => {
                image = image.saturating_sub(1);
                let label = if alt.is_empty() {
                    "image".to_owned()
                } else {
                    alt.clone()
                };
                let src = std::mem::take(&mut image_src);
                if spans.is_empty() && marker.is_none() {
                    pending_image = Some((src, label));
                } else {
                    push_span(
                        &mut spans,
                        format!("[image: {label}]"),
                        strong > 0,
                        em > 0,
                        false,
                        false,
                        false,
                    );
                }
                alt.clear();
            }
            Event::Start(Tag::Table(_)) => {
                flush_text(
                    &mut blocks,
                    &mut spans,
                    &mut marker,
                    lists.len().saturating_sub(1) * 18,
                    quote,
                );
                table = Some(TableBuild::default());
            }
            Event::End(TagEnd::Table) => {
                if let Some(t) = table.take() {
                    blocks.push(Block::Table {
                        head: t.head,
                        rows: t.rows,
                        quote,
                    });
                }
            }
            Event::Start(Tag::TableHead) => {
                if let Some(t) = &mut table {
                    t.in_head = true;
                    t.row.clear();
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(t) = &mut table {
                    // GFM tables emit header cells directly, without a TableRow.
                    if !t.row.is_empty() {
                        t.head = std::mem::take(&mut t.row);
                    }
                    t.in_head = false;
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(t) = &mut table {
                    t.row.clear();
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(t) = &mut table {
                    let row = std::mem::take(&mut t.row);
                    if t.in_head {
                        t.head = row;
                    } else {
                        t.rows.push(row);
                    }
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(t) = &mut table {
                    t.cell.clear();
                    t.cell_open = true;
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(t) = &mut table {
                    let cell = std::mem::take(&mut t.cell);
                    t.row.push(cell);
                    t.cell_open = false;
                }
            }
            Event::Text(text) => {
                let href = hrefs.last().cloned();
                if let Some((_, code_text, _)) = &mut code {
                    code_text.push_str(&text);
                } else if image > 0 {
                    alt.push_str(&text);
                } else if let Some(t) = &mut table {
                    if t.cell_open {
                        push_span(
                            &mut t.cell,
                            text.to_string(),
                            strong > 0,
                            em > 0,
                            strike > 0,
                            link > 0,
                            false,
                        );
                        attach_href(&mut t.cell, href.as_ref());
                    }
                } else {
                    push_span(
                        &mut spans,
                        text.to_string(),
                        strong > 0,
                        em > 0,
                        strike > 0,
                        link > 0,
                        false,
                    );
                    attach_href(&mut spans, href.as_ref());
                }
            }
            Event::Code(text) => {
                let href = hrefs.last().cloned();
                if let Some((_, code_text, _)) = &mut code {
                    code_text.push_str(&text);
                } else {
                    push_span(
                        &mut spans,
                        format!("\u{200a}{text}\u{200a}"),
                        strong > 0,
                        em > 0,
                        false,
                        link > 0,
                        true,
                    );
                    attach_href(&mut spans, href.as_ref());
                }
            }
            Event::SoftBreak => {
                let href = hrefs.last().cloned();
                if let Some((_, code_text, _)) = &mut code {
                    code_text.push('\n');
                } else {
                    push_span(
                        &mut spans,
                        " ".to_owned(),
                        strong > 0,
                        em > 0,
                        strike > 0,
                        link > 0,
                        false,
                    );
                    attach_href(&mut spans, href.as_ref());
                }
            }
            Event::HardBreak => {
                let href = hrefs.last().cloned();
                push_span(
                    &mut spans,
                    "\n".to_owned(),
                    strong > 0,
                    em > 0,
                    strike > 0,
                    link > 0,
                    false,
                );
                attach_href(&mut spans, href.as_ref());
            }
            Event::Rule => blocks.push(Block::Rule { quote }),
            Event::TaskListMarker(checked) => marker = Some(Marker::Task(checked)),
            _ => {}
        }
    }

    Parsed { blocks, meta }
}

fn flush_text(
    blocks: &mut Vec<Block>,
    spans: &mut Vec<Span>,
    marker: &mut Option<Marker>,
    indent: usize,
    quote: usize,
) {
    if spans.is_empty() && marker.is_none() {
        return;
    }
    blocks.push(Block::Text {
        spans: std::mem::take(spans),
        indent,
        marker: marker.take(),
        quote,
    });
}

fn attach_href(spans: &mut [Span], href: Option<&String>) {
    if let Some(href) = href {
        if let Some(last) = spans.last_mut() {
            if last.link {
                last.href = href.clone();
            }
        }
    }
}

fn push_span(
    spans: &mut Vec<Span>,
    text: String,
    strong: bool,
    em: bool,
    strike: bool,
    link: bool,
    code: bool,
) {
    if text.is_empty() {
        return;
    }
    if !link {
        if let Some(last) = spans.last_mut() {
            if last.strong == strong
                && last.em == em
                && last.strike == strike
                && last.link == link
                && last.code == code
            {
                last.text.push_str(&text);
                return;
            }
        }
    }
    spans.push(Span {
        text,
        strong,
        em,
        strike,
        link,
        href: String::new(),
        code,
    });
}

/// A click in the preview that the editor should apply to the source.
#[derive(Clone, Debug)]
pub enum Action {
    Copy(String),
    ToggleTask(usize),
    OpenLink(String),
}

/// Draw the document. Returns the action requested by a click, if any.
pub fn show(ui: &mut Ui, blocks: &[Block], base: Option<&std::path::Path>) -> Option<Action> {
    let mut action = None;
    let mut tasks = 0;
    let mut index = 0;
    while index < blocks.len() {
        let quote = blocks[index].quote();
        if quote > 0 {
            let start = index;
            while index < blocks.len() && blocks[index].quote() == quote {
                index += 1;
            }
            if let Some(next) = quote_group(ui, &blocks[start..index], &mut tasks, base) {
                action = Some(next);
            }
        } else {
            if let Some(next) = block_ui(ui, &blocks[index], index, &mut tasks, base) {
                action = Some(next);
            }
            index += 1;
        }
    }
    action
}

fn block_ui(
    ui: &mut Ui,
    block: &Block,
    index: usize,
    tasks: &mut usize,
    base: Option<&std::path::Path>,
) -> Option<Action> {
    match block {
        Block::Heading {
            level,
            spans,
            color,
            ..
        } => {
            heading(ui, *level, spans, index == 0, *color);
            None
        }
        Block::Text {
            spans,
            indent,
            marker,
            ..
        } => {
            let task = match marker {
                Some(Marker::Task(_)) => {
                    let task = *tasks;
                    *tasks += 1;
                    Some(task)
                }
                _ => None,
            };
            text_block(ui, spans, *indent, marker.as_ref(), 0, task)
        }
        Block::Code {
            lang, text, tokens, ..
        } => code_block(ui, lang, text, tokens).map(Action::Copy),
        Block::Image { src, alt, .. } => {
            image_block(ui, src, alt, base);
            None
        }
        Block::Rule { .. } => {
            rule(ui);
            None
        }
        Block::Table { head, rows, .. } => {
            table(ui, head, rows);
            None
        }
    }
}

/// Render a run of consecutive blockquote blocks as one callout surface.
/// `> [!NOTE]` / `> [!WARNING]` on the first line becomes the callout title.
fn quote_group(
    ui: &mut Ui,
    group: &[Block],
    tasks: &mut usize,
    base: Option<&std::path::Path>,
) -> Option<Action> {
    let (label, body) = split_callout(group);
    let mut action = None;

    ui.add_space(6.0);
    let inner = egui::Frame::new()
        .fill(theme::QUOTE_BG)
        .corner_radius(CornerRadius::same(6))
        .inner_margin(Margin {
            left: 16,
            right: 14,
            top: 8,
            bottom: 2,
        })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let width = ui.available_width().max(120.0);
            ui.set_width(width);
            if let Some(kind) = &label {
                ui.label(RichText::new(kind).size(9.5).color(theme::ACCENT));
                ui.add_space(1.0);
            }
            for (index, block) in body.iter().enumerate() {
                if let Some(next) = block_ui(ui, block, index, tasks, base) {
                    action = Some(next);
                }
            }
        });

    let rect = inner.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_size(
            egui::pos2(rect.left(), rect.top() + 2.0),
            egui::vec2(2.0, (rect.height() - 4.0).max(0.0)),
        ),
        CornerRadius::same(1),
        theme::FAINT,
    );
    ui.add_space(8.0);
    action
}

fn split_callout(group: &[Block]) -> (Option<String>, Vec<Block>) {
    let mut body: Vec<Block> = group.to_vec();
    let Some(Block::Text { spans, .. }) = body.first_mut() else {
        return (None, body);
    };
    let Some(first) = spans.first_mut() else {
        return (None, body);
    };
    let text = first.text.trim_start().to_owned();
    let Some(rest) = text.strip_prefix("[!") else {
        return (None, body);
    };
    let Some(end) = rest.find(']') else {
        return (None, body);
    };
    let kind = rest[..end].trim().to_owned();
    if kind.is_empty() || kind.len() > 24 {
        return (None, body);
    }

    let remainder = rest[end + 1..].trim_start().to_owned();
    if remainder.is_empty() {
        spans.remove(0);
    } else {
        first.text = remainder;
    }
    if spans.is_empty() {
        body.remove(0);
    }
    (Some(kind), body)
}

fn syntax_color(kind: Kind) -> Color32 {
    match kind {
        Kind::Keyword => theme::SYN_KEYWORD,
        Kind::Type => theme::SYN_TYPE,
        Kind::Function => theme::SYN_FUNCTION,
        Kind::String => theme::SYN_STRING,
        Kind::Number => theme::SYN_NUMBER,
        Kind::Comment => theme::SYN_COMMENT,
        Kind::Plain => theme::TEXT,
    }
}

fn heading(
    ui: &mut Ui,
    level: u8,
    spans: &[Span],
    first: bool,
    color: Option<Color32>,
) {
    let (size, top, bottom) = match level {
        1 => (23.0, 22.0, 10.0),
        2 => (19.0, 18.0, 8.0),
        3 => (16.0, 15.0, 6.0),
        _ => (14.5, 13.0, 5.0),
    };
    ui.add_space(if first { 0.0 } else { top });
    let width = ui.available_width();
    let job = job(spans, size, color.unwrap_or(theme::TEXT_BRIGHT), width);
    ui.add(Label::new(job).selectable(true));
    ui.add_space(bottom);
}

fn text_block(
    ui: &mut Ui,
    spans: &[Span],
    indent: usize,
    marker: Option<&Marker>,
    quote: usize,
    task: Option<usize>,
) -> Option<Action> {
    if spans.is_empty() && marker.is_none() {
        return None;
    }

    let indent = indent as f32 + quote as f32 * 16.0;
    let marker_width = if marker.is_some() { 24.0 } else { 0.0 };
    let width = (ui.available_width() - indent - marker_width).max(80.0);
    let mut action = None;

    ui.horizontal_top(|ui| {
        if indent > 0.0 {
            ui.add_space(indent);
        }

        if let Some(marker) = marker {
            match marker {
                Marker::Task(checked) => {
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(marker_width, 21.0),
                        Sense::click(),
                    );
                    let box_rect =
                        Rect::from_center_size(rect.center(), egui::vec2(13.0, 13.0));
                    if *checked {
                        ui.painter().rect_filled(
                            box_rect,
                            CornerRadius::same(3),
                            theme::ACCENT,
                        );
                        let c = box_rect.center();
                        let stroke = Stroke::new(1.6, theme::BG);
                        ui.painter().line_segment(
                            [
                                egui::pos2(c.x - 3.0, c.y),
                                egui::pos2(c.x - 1.0, c.y + 2.8),
                            ],
                            stroke,
                        );
                        ui.painter().line_segment(
                            [
                                egui::pos2(c.x - 1.0, c.y + 2.8),
                                egui::pos2(c.x + 3.8, c.y - 3.0),
                            ],
                            stroke,
                        );
                    } else {
                        let color = if response.hovered() {
                            theme::TEXT
                        } else {
                            theme::DIM
                        };
                        ui.painter().rect_stroke(
                            box_rect,
                            CornerRadius::same(3),
                            Stroke::new(1.2, color),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    if response.clicked() {
                        action = task.map(Action::ToggleTask);
                    }
                }
                Marker::Bullet => {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(marker_width, 21.0),
                        Sense::hover(),
                    );
                    ui.painter().circle_filled(
                        egui::pos2(rect.right() - 7.0, rect.center().y),
                        2.4,
                        theme::DIM,
                    );
                }
                Marker::Ordered(n) => {
                    let (rect, _) = ui.allocate_exact_size(
                        egui::vec2(marker_width, 21.0),
                        Sense::hover(),
                    );
                    ui.painter().text(
                        egui::pos2(rect.right() - 2.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        format!("{n}."),
                        preview_font(14.0),
                        theme::DIM,
                    );
                }
            }
        }

        let ranges = link_ranges(spans);
        let job = job(spans, theme::BODY_SIZE, theme::TEXT, width);
        let galley = ui.painter().layout_job(job);
        let response = ui.add(
            Label::new(galley.clone())
                .wrap_mode(TextWrapMode::Wrap)
                .selectable(true),
        );
        if !ranges.is_empty() {
            let hit = |pos: egui::Pos2| -> Option<String> {
                let local = pos - response.rect.min;
                let cursor = galley.cursor_from_pos(local);
                let index = cursor.index;
                ranges
                    .iter()
                    .find(|(start, end, _)| index >= *start && index < *end)
                    .map(|(_, _, href)| href.clone())
            };
            if response.hovered() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    if hit(pos).is_some() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                }
            }
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    if let Some(href) = hit(pos) {
                        action = Some(Action::OpenLink(href));
                    }
                }
            }
        }

        if quote > 0 {
            let x = response.rect.left() - 9.0;
            ui.painter().line_segment(
                [
                    egui::pos2(x, response.rect.top()),
                    egui::pos2(x, response.rect.bottom()),
                ],
                Stroke::new(2.0, theme::LINE),
            );
        }
    });

    ui.add_space(if marker.is_some() { 3.0 } else { 7.0 });
    action
}

fn image_block(ui: &mut Ui, src: &str, alt: &str, base: Option<&std::path::Path>) {
    ui.add_space(8.0);
    if src.starts_with("http://") || src.starts_with("https://") {
        ui.label(
            RichText::new(format!("image offline: {alt}"))
                .size(11.0)
                .color(theme::FAINT),
        );
        ui.add_space(8.0);
        return;
    }

    let path = match base {
        Some(base) => base.join(src),
        None => std::path::PathBuf::from(src),
    };
    let Some((texture, width, height)) = load_image(ui.ctx(), &path) else {
        ui.label(
            RichText::new(format!("image missing: {alt}"))
                .size(11.0)
                .color(theme::FAINT),
        );
        ui.add_space(8.0);
        return;
    };

    let max_width = ui.available_width().clamp(80.0, 760.0);
    let scale = (max_width / width as f32)
        .min(420.0 / height.max(1) as f32)
        .min(1.0);
    let size = egui::vec2(width as f32 * scale, height as f32 * scale);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().image(
        texture,
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );
    ui.add_space(9.0);
}

fn load_image(
    ctx: &egui::Context,
    path: &std::path::Path,
) -> Option<(egui::TextureId, usize, usize)> {
    let modified = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let key = egui::Id::new(("notan-image", path, modified));
    if let Some((texture, width, height)) = ctx
        .data(|data| data.get_temp::<(egui::TextureHandle, usize, usize)>(key))
    {
        return Some((texture.id(), width, height));
    }

    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 12 * 1024 * 1024 {
        return None;
    }
    let image = image::open(path).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 || width * height > 40_000_000 {
        return None;
    }
    let texture = ctx.load_texture(
        format!("notan-img-{}", path.display()),
        egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, height as usize],
            image.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    );
    let id = texture.id();
    ctx.data_mut(|data| {
        data.insert_temp(
            key,
            (texture, width as usize, height as usize),
        )
    });
    Some((id, width as usize, height as usize))
}

fn code_block(
    ui: &mut Ui,
    lang: &str,
    text: &str,
    tokens: &[highlight::Token],
) -> Option<String> {
    let source = text.trim_end_matches('\n').to_owned();
    let mut copied = None;
    ui.add_space(8.0);
    egui::Frame::new()
        .fill(theme::CODE_BG)
        .corner_radius(CornerRadius::same(5))
        .inner_margin(Margin::symmetric(13, 11))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if !lang.is_empty() {
                    ui.label(
                        RichText::new(lang.to_uppercase())
                            .size(9.5)
                            .color(theme::FAINT),
                    );
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::click());
                    let now = std::time::Instant::now();
                    let active = ui
                        .ctx()
                        .data(|d| d.get_temp::<std::time::Instant>(response.id))
                        .is_some_and(|at| now.duration_since(at).as_millis() < 1400);
                    if response.clicked() {
                        copied = Some(source.clone());
                        ui.ctx().data_mut(|d| d.insert_temp(response.id, now));
                    }
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    let color = if active {
                        theme::ACCENT
                    } else if response.hovered() {
                        theme::TEXT
                    } else {
                        theme::FAINT
                    };
                    theme::paint_icon(
                        ui.painter(),
                        theme::icon(ui.ctx(), if active { "copy-check" } else { "copy" }),
                        Rect::from_center_size(rect.center(), egui::vec2(13.0, 13.0)),
                        color,
                    );
                    if active {
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(1400));
                    }
                });
            });
            ui.add_space(3.0);
            let width = ui.available_width();
            let mut job = LayoutJob::default();
            job.wrap.max_width = width;
            for token in tokens {
                let mut format = TextFormat::simple(
                    FontId::new(12.8, FontFamily::Monospace),
                    syntax_color(token.kind),
                );
                if token.kind == Kind::Comment {
                    format.italics = true;
                }
                job.append(&token.text, 0.0, format);
            }
            ui.add(Label::new(job).wrap_mode(TextWrapMode::Wrap).selectable(true));
        });
    ui.add_space(9.0);
    copied
}

fn rule(ui: &mut Ui) {
    ui.add_space(10.0);
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::ZERO, theme::LINE);
    ui.add_space(10.0);
}

fn table(ui: &mut Ui, head: &[Vec<Span>], rows: &[Vec<Vec<Span>>]) {
    let columns = head
        .first()
        .map(Vec::len)
        .or_else(|| rows.first().map(Vec::len))
        .unwrap_or(1)
        .max(1);
    let gap = 18.0;
    let cell_width =
        ((ui.available_width() - gap * (columns.saturating_sub(1)) as f32) / columns as f32)
            .max(70.0);

    ui.add_space(7.0);
    let mut header_bottom: Option<f32> = None;
    let mut body_top: Option<f32> = None;
    let grid = egui::Grid::new(ui.next_auto_id())
        .num_columns(columns)
        .spacing([gap, 7.0])
        .show(ui, |ui| {
            for cell in head {
                let job = job(cell, 13.0, theme::TEXT_BRIGHT, cell_width);
                let response = ui.add(Label::new(job).selectable(true));
                if header_bottom.is_none() {
                    header_bottom = Some(response.rect.bottom());
                }
            }
            if !head.is_empty() {
                ui.end_row();
            }
            for row in rows {
                for cell in row {
                    let job = job(cell, 13.0, theme::TEXT, cell_width);
                    let response = ui.add(Label::new(job).selectable(true));
                    if body_top.is_none() {
                        body_top = Some(response.rect.top());
                    }
                }
                ui.end_row();
            }
        });

    if let Some(bottom) = header_bottom {
        let top = body_top.unwrap_or(bottom + 8.0);
        let bounds = grid.response.rect;
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(bounds.left(), (bottom + top) * 0.5),
                egui::vec2(bounds.width(), 1.0),
            ),
            CornerRadius::ZERO,
            theme::LINE_SOFT,
        );
    }
    ui.add_space(9.0);
}

fn link_ranges(spans: &[Span]) -> Vec<(usize, usize, String)> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    for span in spans {
        let len = span.text.chars().count();
        if span.link && !span.href.is_empty() {
            ranges.push((offset, offset + len, span.href.clone()));
        }
        offset += len;
    }
    ranges
}

fn preview_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

fn job(spans: &[Span], size: f32, color: Color32, width: f32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = width;
    for span in spans {
        let mut format = TextFormat::simple(preview_font(size), color);
        if span.strong {
            format.color = theme::TEXT_BRIGHT;
        }
        if span.em {
            format.italics = true;
        }
        if span.strike {
            format.strikethrough = Stroke::new(1.0, theme::DIM);
        }
        if span.link {
            format.color = theme::TEXT_BRIGHT;
            format.underline = Stroke::new(1.0, theme::FAINT);
        }
        if span.code {
            format.font_id = FontId::new(size * 0.92, FontFamily::Monospace);
            format.color = theme::TEXT_BRIGHT;
            format.background = theme::CODE_BG;
        }
        job.append(&span.text, 0.0, format);
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "timing; run manually"]
    fn bench_parse() {
        let mut body = String::new();
        for i in 0..400 {
            body.push_str(&format!(
                "## Section {i}\n\nSome **bold** and `code` text with a [link](x).\n\n- item one\n- item two\n\n```rust\nfn x() -> i32 {{ 42 }}\n```\n\n"
            ));
        }
        let start = std::time::Instant::now();
        for _ in 0..40 {
            std::hint::black_box(parse(&body));
        }
        let per_call = start.elapsed() / 40;
        println!("parse: {per_call:?} per call, {} KiB", body.len() / 1024);
    }

    #[test]
    fn parses_front_matter_images_and_links() {
        let parsed = parse("---\ntype: tests\ncolor: red\n---\n\n## Plan\n");
        assert_eq!(parsed.meta.kind.as_deref(), Some("tests"));
        assert_eq!(parsed.meta.color.as_deref(), Some("red"));
        assert!(matches!(
            parsed.blocks.first(),
            Some(Block::Heading { level: 2, color: Some(_), .. })
        ));

        let parsed = parse("![Diagram](img/flow.png)\n");
        assert!(matches!(
            parsed.blocks.first(),
            Some(Block::Image { src, .. }) if src == "img/flow.png"
        ));

        let parsed = parse("See [auth](specs/auth.md).\n");
        let href = parsed.blocks.iter().find_map(|block| match block {
            Block::Text { spans, .. } => spans.iter().find(|span| !span.href.is_empty()),
            _ => None,
        });
        assert_eq!(href.map(|span| span.href.as_str()), Some("specs/auth.md"));
    }

    #[test]
    fn reads_type_and_heading_colors() {
        let src = "TYPE: architecture\nCOLOR: blue\n\n## Design\n\n##[red] Warning\n\n### Detail\n";
        let parsed = parse(src);
        assert_eq!(parsed.meta.kind.as_deref(), Some("architecture"));
        assert_eq!(parsed.meta.color.as_deref(), Some("blue"));

        let colors: Vec<_> = parsed
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Heading { color, level, .. } => Some((*level, *color)),
                _ => None,
            })
            .collect();
        assert_eq!(colors.len(), 3);
        assert_eq!(colors[0], (2, Some(theme::HEAD_BLUE)));
        assert_eq!(colors[1], (2, Some(theme::HEAD_RED)));
        assert_eq!(colors[2], (3, Some(theme::HEAD_BLUE)));

        let parsed = parse("##[green] No space\n");
        assert!(matches!(
            parsed.blocks.first(),
            Some(Block::Heading { level: 2, color: Some(_), .. })
        ));
    }

    #[test]
    fn parses_common_blocks() {
        let src = "# Title\n\nSome **bold** text.\n\n- one\n- two\n\n```rust\nfn x() {}\n```\n\n> quote\n";
        let blocks = parse(src).blocks;
        assert!(matches!(blocks[0], Block::Heading { level: 1, .. }));
        assert!(blocks.iter().any(|b| matches!(b, Block::Text { marker: Some(Marker::Bullet), .. })));
        assert!(blocks.iter().any(|b| matches!(b, Block::Code { .. })));
        assert!(blocks.iter().any(|b| matches!(b, Block::Text { quote: 1, .. })));
    }

    #[test]
    fn groups_multiline_quotes() {
        let src = "> [!note]\n>\n> first line\n> second line\n\nAfter.\n";
        let blocks = parse(src).blocks;
        assert_eq!(blocks[0].quote(), 1);
        assert_eq!(blocks[1].quote(), 1);
        let (label, body) = split_callout(&blocks[..2]);
        assert_eq!(label.as_deref(), Some("note"));
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0], Block::Text { spans, .. } if spans[0].text.starts_with("first")));
    }

    #[test]
    fn parses_tables_and_tasks() {
        let src = "| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n";
        let blocks = parse(src).blocks;
        let table = blocks
            .iter()
            .find_map(|b| match b {
                Block::Table { head, rows, .. } => Some((head, rows)),
                _ => None,
            })
            .expect("table");
        assert_eq!(table.0.len(), 2);
        assert_eq!(table.1.len(), 1);
        assert!(blocks.iter().any(|b| matches!(
            b,
            Block::Text {
                marker: Some(Marker::Task(true)),
                ..
            }
        )));
    }

    #[test]
    fn parses_nested_lists() {
        let blocks = parse("- parent\n  - child\n").blocks;
        let items: Vec<&Block> = blocks
            .iter()
            .filter(|b| matches!(b, Block::Text { marker: Some(_), .. }))
            .collect();
        assert_eq!(items.len(), 2);
        match items[0] {
            Block::Text { indent, spans, .. } => {
                assert_eq!(*indent, 0);
                assert_eq!(spans[0].text, "parent");
            }
            _ => unreachable!(),
        }
        match items[1] {
            Block::Text { indent, spans, .. } => {
                assert_eq!(*indent, 18);
                assert_eq!(spans[0].text, "child");
            }
            _ => unreachable!(),
        }
    }
}
