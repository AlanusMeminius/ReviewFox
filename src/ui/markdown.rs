use std::ops::Range;

use gpui::{
    AbsoluteLength, AnyElement, App, DefiniteLength, Div, FontStyle, FontWeight, HighlightStyle,
    SharedString, Stateful, StrikethroughStyle, StyledText, TextLayout, UnderlineStyle, div,
    prelude::*, px, relative,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag};

use super::appearance::{self, Appearance};
use super::theme;

/// Heading / small ratios relative to the UI Font body size (BeadsViewer typography).
const MARKDOWN_H1: f32 = 1.5;
const MARKDOWN_H2: f32 = 1.25;
const MARKDOWN_H3: f32 = 1.125;
const MARKDOWN_SMALL: f32 = 0.875;
/// Comfortable line height (BeadsViewer `LineHeight::Comfortable`).
const MARKDOWN_LINE_HEIGHT: f32 = 1.618;

/// Families Markdown renders with. `None` means make no `font_family` call.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MarkdownFonts {
    pub(crate) body: Option<String>,
    pub(crate) code: Option<String>,
    /// UI Font body size in px — headings scale from this.
    pub(crate) base_size: f32,
}

impl Default for MarkdownFonts {
    fn default() -> Self {
        Self {
            body: None,
            code: None,
            base_size: appearance::UI_FONT_SIZE_DEFAULT as f32,
        }
    }
}

/// Resolve Markdown families and base size from ReviewFox appearance.
pub(crate) fn fonts_from_app(cx: &App) -> MarkdownFonts {
    let appearance = cx.global::<Appearance>();
    MarkdownFonts {
        body: Some(appearance.ui_font.name.to_string()),
        code: Some(appearance.code_font.name.to_string()),
        base_size: appearance.ui_font_size as f32,
    }
}

fn markdown_text_size(fonts: &MarkdownFonts, ratio: f32) -> AbsoluteLength {
    px((fonts.base_size * ratio).round()).into()
}

fn markdown_line_height() -> DefiniteLength {
    relative(MARKDOWN_LINE_HEIGHT)
}

/// One shaped text run produced while rendering, keyed into the field's plain buffer.
#[derive(Clone)]
pub(crate) struct MarkdownSegment {
    pub layout: TextLayout,
    pub plain_start: usize,
}

/// An http(s) link span inside the rendered plain buffer.
#[derive(Clone, Debug)]
pub(crate) struct MarkdownLink {
    pub range: Range<usize>,
    pub url: SharedString,
}

/// Collects plain text, layouts, and link ranges while building the element tree.
#[derive(Default)]
pub(crate) struct RenderSink {
    pub plain: String,
    pub links: Vec<MarkdownLink>,
    pub segments: Vec<MarkdownSegment>,
}

impl RenderSink {
    fn block_break(&mut self) {
        if !self.plain.is_empty() && !self.plain.ends_with('\n') {
            self.plain.push('\n');
        }
    }

    fn push_segment(&mut self, layout: TextLayout, text: &str) {
        let plain_start = self.plain.len();
        self.plain.push_str(text);
        self.segments.push(MarkdownSegment {
            layout,
            plain_start,
        });
    }
}

#[derive(Debug, PartialEq)]
struct Document {
    blocks: Vec<Block>,
}

#[derive(Debug, PartialEq)]
enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        content: Vec<Inline>,
    },
    Quote(Vec<Block>),
    Code {
        language: Option<String>,
        code: String,
    },
    List {
        start: Option<u64>,
        items: Vec<Vec<Block>>,
    },
    Table {
        head: Vec<Vec<Inline>>,
        rows: Vec<Vec<Vec<Inline>>>,
    },
    Rule,
}

#[derive(Debug, PartialEq)]
enum Inline {
    Text(String),
    Emphasis(Vec<Inline>),
    Strong(Vec<Inline>),
    Strikethrough(Vec<Inline>),
    Code(String),
    Link {
        destination: String,
        content: Vec<Inline>,
    },
    Image {
        destination: String,
        alt: String,
    },
    TaskMarker(bool),
    SoftBreak,
    HardBreak,
}

impl Document {
    #[cfg(test)]
    fn inlines(&self) -> impl Iterator<Item = &Inline> {
        let mut inlines = Vec::new();
        for block in &self.blocks {
            collect_block_inlines(block, &mut inlines);
        }
        inlines.into_iter()
    }

    #[cfg(test)]
    fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(block_plain_text)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
fn collect_block_inlines<'a>(block: &'a Block, output: &mut Vec<&'a Inline>) {
    match block {
        Block::Paragraph(content) | Block::Heading { content, .. } => {
            collect_inlines(content, output)
        }
        Block::Quote(blocks) => {
            for block in blocks {
                collect_block_inlines(block, output);
            }
        }
        Block::List { items, .. } => {
            for item in items {
                for block in item {
                    collect_block_inlines(block, output);
                }
            }
        }
        Block::Table { head, rows } => {
            for cell in head.iter().chain(rows.iter().flatten()) {
                collect_inlines(cell, output);
            }
        }
        Block::Code { .. } | Block::Rule => {}
    }
}

#[cfg(test)]
fn collect_inlines<'a>(inlines: &'a [Inline], output: &mut Vec<&'a Inline>) {
    for inline in inlines {
        output.push(inline);
        match inline {
            Inline::Emphasis(content)
            | Inline::Strong(content)
            | Inline::Strikethrough(content)
            | Inline::Link { content, .. } => collect_inlines(content, output),
            _ => {}
        }
    }
}

fn block_plain_text(block: &Block) -> String {
    match block {
        Block::Paragraph(content) | Block::Heading { content, .. } => inline_plain_text(content),
        Block::Quote(blocks) => blocks
            .iter()
            .map(block_plain_text)
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Code { code, .. } => code.clone(),
        Block::List { items, .. } => items
            .iter()
            .flat_map(|item| item.iter().map(block_plain_text))
            .collect::<Vec<_>>()
            .join("\n"),
        Block::Table { head, rows } => head
            .iter()
            .chain(rows.iter().flatten())
            .map(|cell| inline_plain_text(cell))
            .collect::<Vec<_>>()
            .join("\t"),
        Block::Rule => String::new(),
    }
}

fn inline_plain_text(inlines: &[Inline]) -> String {
    let mut text = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(value) | Inline::Code(value) => text.push_str(value),
            Inline::Emphasis(content)
            | Inline::Strong(content)
            | Inline::Strikethrough(content)
            | Inline::Link { content, .. } => text.push_str(&inline_plain_text(content)),
            Inline::Image { destination, alt } => {
                text.push_str(alt);
                text.push_str(" (");
                text.push_str(destination);
                text.push(')');
            }
            Inline::TaskMarker(checked) => {
                text.push_str(if *checked { "☑ " } else { "☐ " });
            }
            Inline::SoftBreak => text.push(' '),
            Inline::HardBreak => text.push('\n'),
        }
    }
    text
}

#[derive(Debug)]
enum Node {
    Container(Container, Vec<Node>),
    Text(String),
    Code(String),
    TaskMarker(bool),
    SoftBreak,
    HardBreak,
    Rule,
}

#[derive(Debug)]
enum Container {
    Paragraph,
    Heading(u8),
    BlockQuote,
    CodeBlock(Option<String>),
    List(Option<u64>),
    Item,
    Table,
    TableHead,
    TableRow,
    TableCell,
    Emphasis,
    Strong,
    Strikethrough,
    Link(String),
    Image(String),
    Transparent,
}

struct Frame {
    container: Container,
    children: Vec<Node>,
}

fn parse_document(source: &str) -> Document {
    let options = Options::ENABLE_GFM
        | Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let mut roots = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();

    for event in Parser::new_ext(source, options) {
        match event {
            Event::Start(tag) => stack.push(Frame {
                container: container_from_tag(tag),
                children: Vec::new(),
            }),
            Event::End(_) => {
                if let Some(frame) = stack.pop() {
                    push_node(
                        &mut roots,
                        &mut stack,
                        Node::Container(frame.container, frame.children),
                    );
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                push_node(&mut roots, &mut stack, Node::Text(text.into_string()));
            }
            Event::Code(code) => {
                push_node(&mut roots, &mut stack, Node::Code(code.into_string()));
            }
            Event::SoftBreak => push_node(&mut roots, &mut stack, Node::SoftBreak),
            Event::HardBreak => push_node(&mut roots, &mut stack, Node::HardBreak),
            Event::Rule => push_node(&mut roots, &mut stack, Node::Rule),
            Event::TaskListMarker(checked) => {
                push_node(&mut roots, &mut stack, Node::TaskMarker(checked));
            }
            Event::FootnoteReference(name) => {
                push_node(&mut roots, &mut stack, Node::Text(name.into_string()));
            }
            Event::InlineMath(value) | Event::DisplayMath(value) => {
                push_node(&mut roots, &mut stack, Node::Text(value.into_string()));
            }
        }
    }

    Document {
        blocks: roots.into_iter().filter_map(node_to_block).collect(),
    }
}

fn push_node(roots: &mut Vec<Node>, stack: &mut [Frame], node: Node) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn container_from_tag(tag: Tag<'_>) -> Container {
    match tag {
        Tag::Paragraph => Container::Paragraph,
        Tag::Heading { level, .. } => Container::Heading(heading_level(level)),
        Tag::BlockQuote(_) => Container::BlockQuote,
        Tag::CodeBlock(kind) => Container::CodeBlock(match kind {
            CodeBlockKind::Indented => None,
            CodeBlockKind::Fenced(language) if language.is_empty() => None,
            CodeBlockKind::Fenced(language) => Some(language.into_string()),
        }),
        Tag::List(start) => Container::List(start),
        Tag::Item => Container::Item,
        Tag::Table(_) => Container::Table,
        Tag::TableHead => Container::TableHead,
        Tag::TableRow => Container::TableRow,
        Tag::TableCell => Container::TableCell,
        Tag::Emphasis => Container::Emphasis,
        Tag::Strong => Container::Strong,
        Tag::Strikethrough => Container::Strikethrough,
        Tag::Link { dest_url, .. } => Container::Link(dest_url.into_string()),
        Tag::Image { dest_url, .. } => Container::Image(dest_url.into_string()),
        Tag::HtmlBlock
        | Tag::FootnoteDefinition(_)
        | Tag::MetadataBlock(_)
        | Tag::DefinitionList
        | Tag::DefinitionListTitle
        | Tag::DefinitionListDefinition => Container::Transparent,
    }
}

fn node_to_block(node: Node) -> Option<Block> {
    match node {
        Node::Container(Container::Paragraph, children) => {
            Some(Block::Paragraph(nodes_to_inlines(children)))
        }
        Node::Container(Container::Heading(level), children) => Some(Block::Heading {
            level,
            content: nodes_to_inlines(children),
        }),
        Node::Container(Container::BlockQuote, children) => {
            Some(Block::Quote(nodes_to_blocks(children)))
        }
        Node::Container(Container::CodeBlock(language), children) => Some(Block::Code {
            language,
            code: nodes_plain_text(&children),
        }),
        Node::Container(Container::List(start), children) => Some(Block::List {
            start,
            items: children
                .into_iter()
                .filter_map(|node| match node {
                    Node::Container(Container::Item, children) => Some(nodes_to_blocks(children)),
                    _ => None,
                })
                .collect(),
        }),
        Node::Container(Container::Table, children) => Some(table_block(children)),
        Node::Rule => Some(Block::Rule),
        Node::Container(Container::Transparent, children) => {
            Some(Block::Paragraph(nodes_to_inlines(children)))
        }
        Node::Text(text) => Some(Block::Paragraph(vec![Inline::Text(text)])),
        _ => None,
    }
}

fn nodes_to_blocks(nodes: Vec<Node>) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut loose_inlines = Vec::new();
    for node in nodes {
        let is_block = matches!(
            node,
            Node::Container(
                Container::Paragraph
                    | Container::Heading(_)
                    | Container::BlockQuote
                    | Container::CodeBlock(_)
                    | Container::List(_)
                    | Container::Table
                    | Container::Transparent,
                _
            ) | Node::Rule
        );
        if is_block {
            if !loose_inlines.is_empty() {
                blocks.push(Block::Paragraph(std::mem::take(&mut loose_inlines)));
            }
            if let Some(block) = node_to_block(node) {
                blocks.push(block);
            }
        } else if let Some(inline) = node_to_inline(node) {
            loose_inlines.push(inline);
        }
    }
    if !loose_inlines.is_empty() {
        blocks.push(Block::Paragraph(loose_inlines));
    }
    blocks
}

fn table_block(children: Vec<Node>) -> Block {
    let mut head = Vec::new();
    let mut rows = Vec::new();
    for child in children {
        match child {
            Node::Container(Container::TableHead, children) => head = table_cells(children),
            Node::Container(Container::TableRow, children) => rows.push(table_cells(children)),
            _ => {}
        }
    }
    Block::Table { head, rows }
}

fn table_cells(children: Vec<Node>) -> Vec<Vec<Inline>> {
    children
        .into_iter()
        .filter_map(|node| match node {
            Node::Container(Container::TableCell, children) => Some(nodes_to_inlines(children)),
            _ => None,
        })
        .collect()
}

fn nodes_to_inlines(nodes: Vec<Node>) -> Vec<Inline> {
    nodes.into_iter().filter_map(node_to_inline).collect()
}

fn node_to_inline(node: Node) -> Option<Inline> {
    match node {
        Node::Text(text) => Some(Inline::Text(text)),
        Node::Code(code) => Some(Inline::Code(code)),
        Node::TaskMarker(checked) => Some(Inline::TaskMarker(checked)),
        Node::SoftBreak => Some(Inline::SoftBreak),
        Node::HardBreak => Some(Inline::HardBreak),
        Node::Container(Container::Emphasis, children) => {
            Some(Inline::Emphasis(nodes_to_inlines(children)))
        }
        Node::Container(Container::Strong, children) => {
            Some(Inline::Strong(nodes_to_inlines(children)))
        }
        Node::Container(Container::Strikethrough, children) => {
            Some(Inline::Strikethrough(nodes_to_inlines(children)))
        }
        Node::Container(Container::Link(destination), children) => Some(Inline::Link {
            destination,
            content: nodes_to_inlines(children),
        }),
        Node::Container(Container::Image(destination), children) => Some(Inline::Image {
            destination,
            alt: nodes_plain_text(&children),
        }),
        Node::Container(_, children) => Some(Inline::Text(nodes_plain_text(&children))),
        Node::Rule => None,
    }
}

fn nodes_plain_text(nodes: &[Node]) -> String {
    let mut text = String::new();
    for node in nodes {
        match node {
            Node::Text(value) | Node::Code(value) => text.push_str(value),
            Node::TaskMarker(checked) => text.push_str(if *checked { "[x] " } else { "[ ] " }),
            Node::SoftBreak | Node::HardBreak => text.push('\n'),
            Node::Container(_, children) => text.push_str(&nodes_plain_text(children)),
            Node::Rule => {}
        }
    }
    text
}

fn is_allowed_link(destination: &str) -> bool {
    let Some((scheme, _)) = destination.split_once(':') else {
        return false;
    };
    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
}

/// Renders `source` with the given families / base size.
pub(crate) fn render(source: &str, fonts: &MarkdownFonts) -> Div {
    render_with_fonts(source, fonts, None)
}

/// Same as [`render`], while recording plain text, link ranges, and `TextLayout`s
/// for mouse selection.
pub(crate) fn render_with_sink(source: &str, fonts: &MarkdownFonts, sink: &mut RenderSink) -> Div {
    render_with_fonts(source, fonts, Some(sink))
}

/// The body of [`render`]; kept separate so tests can supply fonts without an App.
fn render_with_fonts(source: &str, fonts: &MarkdownFonts, sink: Option<&mut RenderSink>) -> Div {
    let document = parse_document(source);
    let mut link_index = 0;
    // The body family is set once, here, and inherited: gpui pushes a parent's
    // text style onto a stack its children refine, so the code family set on
    // code blocks overrides this for exactly those.
    with_font_family(
        render_blocks(&document.blocks, fonts, &mut link_index, sink)
            .text_size(markdown_text_size(fonts, 1.0))
            .line_height(markdown_line_height()),
        fonts.body.as_deref(),
    )
}

/// Applies a resolved family, or leaves the element untouched when there is
/// none.
///
/// `None` means the platform default is the right answer, and that is said by
/// making no `font_family` call at all: `font_family("")` would instead name a
/// family nobody has.
fn with_font_family<E: Styled>(element: E, family: Option<&str>) -> E {
    match family {
        Some(family) => element.font_family(family.to_owned()),
        None => element,
    }
}

/// The chrome a fenced code block is drawn in, in the code family.
fn code_block_chrome(code_block_id: usize, fonts: &MarkdownFonts) -> Stateful<Div> {
    with_font_family(
        div()
            .id(("markdown-code", code_block_id))
            .w_full()
            .overflow_x_scroll()
            .p_3()
            .rounded_md()
            .bg(theme::hover())
            .border_1()
            .border_color(theme::line()),
        fonts.code.as_deref(),
    )
    .text_size(markdown_text_size(fonts, 1.0))
    .line_height(markdown_line_height())
}

/// The language named above a fenced code block.
///
/// The label names the code rather than being code, so it steps back out of the
/// monospace family into the body one.
fn language_label(language: &str, fonts: &MarkdownFonts) -> Div {
    with_font_family(
        div()
            .mb_2()
            .text_size(markdown_text_size(fonts, MARKDOWN_SMALL))
            .line_height(markdown_line_height()),
        fonts.body.as_deref(),
    )
    .text_color(theme::muted())
    .child(language.to_owned())
}

/// An inline code span, in the code family (font-family tests only).
fn inline_code_span(code: &str, fonts: &MarkdownFonts) -> Div {
    with_font_family(
        div().mx_1().px_1().rounded_sm().bg(theme::hover()),
        fonts.code.as_deref(),
    )
    .child(code.to_owned())
}

fn render_blocks(
    blocks: &[Block],
    fonts: &MarkdownFonts,
    link_index: &mut usize,
    mut sink: Option<&mut RenderSink>,
) -> Div {
    let mut container = div().w_full().flex().flex_col().gap_3();
    for block in blocks {
        let child = match sink.as_mut() {
            Some(sink) => render_block(block, fonts, link_index, Some(sink)),
            None => render_block(block, fonts, link_index, None),
        };
        container = container.child(child);
    }
    container
}

fn render_block(
    block: &Block,
    fonts: &MarkdownFonts,
    link_index: &mut usize,
    mut sink: Option<&mut RenderSink>,
) -> AnyElement {
    match block {
        Block::Paragraph(content) => {
            if let Some(sink) = sink.as_deref_mut() {
                sink.block_break();
            }
            div()
                .w_full()
                .min_w(px(0.))
                .child(render_inlines(content, fonts, sink))
                .into_any_element()
        }
        Block::Heading { level, content } => {
            if let Some(sink) = sink.as_deref_mut() {
                sink.block_break();
            }
            let heading = div()
                .w_full()
                .min_w(px(0.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .line_height(markdown_line_height())
                .child(render_inlines(content, fonts, sink));
            let ratio = match level {
                1 => MARKDOWN_H1,
                2 => MARKDOWN_H2,
                _ => MARKDOWN_H3,
            };
            heading
                .text_size(markdown_text_size(fonts, ratio))
                .into_any_element()
        }
        Block::Quote(blocks) => {
            if let Some(sink) = sink.as_deref_mut() {
                sink.block_break();
            }
            div()
                .w_full()
                .pl_3()
                .border_l_2()
                .border_color(theme::faint())
                .text_color(theme::muted())
                .child(render_blocks(blocks, fonts, link_index, sink))
                .into_any_element()
        }
        Block::Code { language, code } => {
            let code_block_id = *link_index;
            *link_index += 1;
            if let Some(sink) = sink.as_deref_mut() {
                sink.block_break();
            }
            let mut code_block = code_block_chrome(code_block_id, fonts);
            if let Some(language) = language {
                code_block = code_block.child(language_label(language, fonts));
            }
            // Each line is laid out on its own and forbidden to reflow, so a
            // long command keeps saying what it says and the block's horizontal
            // scroll range is the one that carries the Operator to the rest of
            // it. A soft-wrapped command reads as a different command.
            let lines: Vec<String> = code.split('\n').map(str::to_owned).collect();
            let line_count = lines.len();
            let mut line_elements = Vec::with_capacity(line_count);
            for (i, line) in lines.into_iter().enumerate() {
                let styled = push_plain_styled(&line, Vec::new(), sink.as_deref_mut());
                if let Some(sink) = sink.as_deref_mut()
                    && i + 1 < line_count
                {
                    sink.plain.push('\n');
                }
                line_elements.push(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .child(styled)
                        .into_any_element(),
                );
            }
            code_block
                .child(div().flex_none().flex().flex_col().children(line_elements))
                .into_any_element()
        }
        Block::List { start, items } => {
            if let Some(sink) = sink.as_deref_mut() {
                sink.block_break();
            }
            let mut list = div().w_full().flex().flex_col().gap_2();
            for (index, item) in items.iter().enumerate() {
                let marker = start
                    .map(|start| format!("{}.", start + index as u64))
                    .unwrap_or_else(|| "•".to_owned());
                list = list.child(
                    div()
                        .w_full()
                        .flex()
                        .items_start()
                        .gap_2()
                        .child(
                            div()
                                .w(px(22.))
                                .flex_none()
                                .text_color(theme::muted())
                                .child(marker),
                        )
                        .child(
                            render_blocks(item, fonts, link_index, sink.as_deref_mut())
                                .flex_1()
                                .min_w(px(0.)),
                        ),
                );
            }
            list.into_any_element()
        }
        Block::Table { head, rows } => {
            let table_id = *link_index;
            *link_index += 1;
            if let Some(sink) = sink.as_deref_mut() {
                sink.block_break();
            }
            let mut table = div()
                .id(("markdown-table", table_id))
                .w_full()
                .overflow_x_scroll()
                .border_1()
                .border_color(theme::line())
                .rounded_md();
            if !head.is_empty() {
                table = table.child(render_table_row(head, true, fonts, sink.as_deref_mut()));
            }
            for row in rows {
                table = table.child(render_table_row(row, false, fonts, sink.as_deref_mut()));
            }
            table.into_any_element()
        }
        Block::Rule => div()
            .w_full()
            .h(px(1.))
            .bg(theme::line())
            .into_any_element(),
    }
}

fn render_table_row(
    cells: &[Vec<Inline>],
    header: bool,
    fonts: &MarkdownFonts,
    mut sink: Option<&mut RenderSink>,
) -> Div {
    let mut row = div()
        .flex()
        .min_w(px(320.))
        .border_b_1()
        .border_color(theme::border_variant());
    if header {
        row = row.bg(theme::hover());
    }
    for (i, cell) in cells.iter().enumerate() {
        if let Some(sink) = sink.as_mut() {
            if i > 0 {
                sink.plain.push('\t');
            } else {
                sink.block_break();
            }
        }
        let mut cell_element = div()
            .min_w(px(120.))
            .flex_1()
            .px_3()
            .py_2()
            .border_r_1()
            .border_color(theme::border_variant());
        if header {
            cell_element = cell_element.font_weight(FontWeight::SEMIBOLD);
        }
        row = row.child(cell_element.child(render_inlines(cell, fonts, sink.as_deref_mut())));
    }
    row
}

fn render_inlines(
    inlines: &[Inline],
    fonts: &MarkdownFonts,
    sink: Option<&mut RenderSink>,
) -> AnyElement {
    let mut paint = InlinePaint::default();
    paint_inlines(inlines, InlineStyle::default(), fonts, &mut paint);
    if paint.text.is_empty() {
        return div().into_any_element();
    }
    let links = paint.links;
    let text = paint.text;
    let highlights = paint.highlights;
    let styled = match sink {
        Some(sink) => {
            let absolute_start = sink.plain.len();
            for (range, url) in links {
                sink.links.push(MarkdownLink {
                    range: absolute_start + range.start..absolute_start + range.end,
                    url,
                });
            }
            push_plain_styled(&text, highlights, Some(sink))
        }
        None => push_plain_styled(&text, highlights, None),
    };
    styled.into_any_element()
}

fn push_plain_styled(
    text: &str,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    sink: Option<&mut RenderSink>,
) -> StyledText {
    let styled = if highlights.is_empty() {
        StyledText::new(text.to_owned())
    } else {
        StyledText::new(text.to_owned()).with_highlights(highlights)
    };
    if let Some(sink) = sink {
        sink.push_segment(styled.layout().clone(), text);
    }
    styled
}

#[derive(Default)]
struct InlinePaint {
    text: String,
    highlights: Vec<(Range<usize>, HighlightStyle)>,
    links: Vec<(Range<usize>, SharedString)>,
}

#[derive(Clone, Copy, Default)]
struct InlineStyle {
    strong: bool,
    emphasis: bool,
    strike: bool,
    code: bool,
    link: bool,
}

impl InlineStyle {
    fn to_highlight(self) -> Option<HighlightStyle> {
        if !(self.strong || self.emphasis || self.strike || self.code || self.link) {
            return None;
        }
        let mut style = HighlightStyle::default();
        if self.strong {
            style.font_weight = Some(FontWeight::SEMIBOLD);
        }
        if self.emphasis {
            style.font_style = Some(FontStyle::Italic);
        }
        if self.strike {
            style.strikethrough = Some(StrikethroughStyle {
                thickness: px(1.),
                color: None,
            });
        }
        if self.code {
            style.background_color = Some(theme::hover().into());
        }
        if self.link {
            style.color = Some(theme::accent().into());
            style.underline = Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(theme::accent().into()),
                wavy: false,
            });
        }
        Some(style)
    }
}

fn paint_inlines(
    inlines: &[Inline],
    style: InlineStyle,
    fonts: &MarkdownFonts,
    paint: &mut InlinePaint,
) {
    let _ = fonts;
    for inline in inlines {
        match inline {
            Inline::Text(value) => append_styled(paint, value, style),
            Inline::Code(value) => {
                let mut code_style = style;
                code_style.code = true;
                append_styled(paint, value, code_style);
            }
            Inline::Emphasis(content) => {
                let mut next = style;
                next.emphasis = true;
                paint_inlines(content, next, fonts, paint);
            }
            Inline::Strong(content) => {
                let mut next = style;
                next.strong = true;
                paint_inlines(content, next, fonts, paint);
            }
            Inline::Strikethrough(content) => {
                let mut next = style;
                next.strike = true;
                paint_inlines(content, next, fonts, paint);
            }
            Inline::Link {
                destination,
                content,
            } if is_allowed_link(destination) => {
                let start = paint.text.len();
                let mut next = style;
                next.link = true;
                paint_inlines(content, next, fonts, paint);
                let end = paint.text.len();
                if start < end {
                    paint
                        .links
                        .push((start..end, SharedString::from(destination.clone())));
                }
            }
            Inline::Link { content, .. } => paint_inlines(content, style, fonts, paint),
            Inline::Image { destination, alt } => {
                append_styled(paint, &format!("{alt} ({destination})"), style);
            }
            Inline::TaskMarker(checked) => {
                append_styled(
                    paint,
                    if *checked { "☑ " } else { "☐ " },
                    style,
                );
            }
            Inline::SoftBreak => append_styled(paint, " ", style),
            Inline::HardBreak => append_styled(paint, "\n", style),
        }
    }
}

fn append_styled(paint: &mut InlinePaint, text: &str, style: InlineStyle) {
    if text.is_empty() {
        return;
    }
    let start = paint.text.len();
    paint.text.push_str(text);
    let end = paint.text.len();
    if let Some(highlight) = style.to_highlight() {
        paint.highlights.push((start..end, highlight));
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        Context, FontWeight, IntoElement, Pixels, Render, ScrollHandle, TestAppContext, Window, div,
        point, prelude::*, px, size,
    };

    use super::{
        Block, Inline, MarkdownFonts, appearance, code_block_chrome, inline_code_span,
        inline_plain_text, is_allowed_link, language_label, parse_document, render, render_with_fonts,
    };

    #[test]
    fn renderer_is_a_stateless_detail_component() {
        let _: gpui::Div = render("# Heading\n\nBody", &fonts(None, None));
    }

    #[test]
    fn parses_the_supported_gfm_structures() {
        let document = parse_document(
            "# Heading\n\n**bold** and ~~removed~~\n\n- [x] done\n\n| A | B |\n| - | - |\n| 1 | 2 |\n\n```rs\nfn main() {}\n```",
        );

        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, Block::Heading { .. }))
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, Block::List { .. }))
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, Block::Table { .. }))
        );
        assert!(
            document
                .blocks
                .iter()
                .any(|block| matches!(block, Block::Code { .. }))
        );
        assert!(
            document
                .inlines()
                .any(|inline| matches!(inline, Inline::Strong(_)))
        );
        assert!(
            document
                .inlines()
                .any(|inline| matches!(inline, Inline::Strikethrough(_)))
        );
        assert!(
            document
                .inlines()
                .any(|inline| matches!(inline, Inline::TaskMarker(true)))
        );
    }

    #[test]
    fn collects_plain_text_and_highlights_for_mixed_inlines() {
        let document = parse_document("hello **bold** and `code` plus [link](https://example.com)");
        let Block::Paragraph(inlines) = &document.blocks[0] else {
            panic!("expected paragraph");
        };
        let mut paint = super::InlinePaint::default();
        super::paint_inlines(
            inlines,
            super::InlineStyle::default(),
            &fonts(None, None),
            &mut paint,
        );
        assert_eq!(paint.text, "hello bold and code plus link");
        assert!(
            paint
                .highlights
                .iter()
                .any(|(range, style)| &paint.text[range.clone()] == "bold"
                    && style.font_weight == Some(FontWeight::SEMIBOLD))
        );
        assert!(
            paint
                .highlights
                .iter()
                .any(|(range, style)| &paint.text[range.clone()] == "code"
                    && style.background_color.is_some())
        );
        assert_eq!(paint.links.len(), 1);
        assert_eq!(&paint.text[paint.links[0].0.clone()], "link");
        assert_eq!(paint.links[0].1.as_ref(), "https://example.com");
    }

    #[test]
    fn render_sink_records_plain_offsets_for_segments() {
        let mut sink = super::RenderSink::default();
        let _ = super::render_with_sink("one\n\ntwo", &fonts(None, None), &mut sink);
        assert!(sink.plain.contains("one"));
        assert!(sink.plain.contains("two"));
        assert!(
            sink.segments.len() >= 2,
            "each paragraph should contribute a shaped segment"
        );
        assert_eq!(sink.segments[0].plain_start, 0);
        assert!(sink.segments[1].plain_start > 0);
    }

    #[test]
    fn keeps_raw_html_as_inert_literal_text() {
        let document = parse_document("<script>alert('no')</script>");

        assert_eq!(document.plain_text(), "<script>alert('no')</script>");
    }

    #[test]
    fn keeps_images_inert_and_preserves_their_alt_text_and_target() {
        let document = parse_document("![diagram](file:///C:/private/diagram.png)");

        assert_eq!(
            document.plain_text(),
            "diagram (file:///C:/private/diagram.png)"
        );
    }

    #[test]
    fn preserves_table_cells_for_native_layout() {
        let document = parse_document("| Name | State |\n| --- | --- |\n| parser | ready |");

        let Some(Block::Table { head, rows }) = document.blocks.first() else {
            panic!("expected a parsed table");
        };
        assert_eq!(head.len(), 2);
        assert_eq!(rows.len(), 1);
        assert_eq!(inline_plain_text(&head[0]), "Name");
        assert_eq!(inline_plain_text(&rows[0][1]), "ready");
    }

    #[test]
    fn only_allows_http_and_https_links() {
        assert!(is_allowed_link("https://example.com"));
        assert!(is_allowed_link("http://example.com"));
        assert!(!is_allowed_link("file:///C:/secret.txt"));
        assert!(!is_allowed_link("javascript:alert(1)"));
        assert!(!is_allowed_link("mailto:operator@example.com"));
    }

    /// Lays the rendered markdown out in a pane `width` wide and reports how
    /// tall its content became.
    ///
    /// Height is the honest signal for wrapping: text that wraps grows taller
    /// as its pane narrows, and text that cannot wrap stays exactly as tall
    /// however narrow the pane gets, running off the edge instead.
    fn content_height(cx: &mut TestAppContext, source: &str, width: f32) -> Pixels {
        struct Pane {
            source: String,
            scroll_handle: ScrollHandle,
        }

        impl Render for Pane {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div().size_full().flex().flex_col().child(
                    div()
                        .id("markdown-pane")
                        .track_scroll(&self.scroll_handle)
                        .flex_1()
                        .overflow_y_scroll()
                        .child(render(&self.source, &fonts(None, None))),
                )
            }
        }

        let cx = cx.add_empty_window();
        let scroll_handle = ScrollHandle::new();
        let source = source.to_owned();
        cx.draw(point(px(0.), px(0.)), size(px(width), px(40.)), |_, cx| {
            cx.new(|_| Pane {
                source,
                scroll_handle: scroll_handle.clone(),
            })
        });
        scroll_handle.max_offset().height
    }

    const NARROW: f32 = 320.;
    const WIDE: f32 = 2_400.;

    #[gpui::test]
    fn a_plain_paragraph_wraps_instead_of_running_off_the_pane(cx: &mut TestAppContext) {
        let paragraph = "This is a single plain paragraph with no inline markup at all, long              enough that it cannot possibly fit on one line of a narrow inspector pane. "
            .repeat(4);

        let narrow = content_height(cx, &paragraph, NARROW);
        let wide = content_height(cx, &paragraph, WIDE);

        assert!(
            narrow > wide,
            "a paragraph must grow taller as its pane narrows; it stayed {narrow:?} against              {wide:?}, which means it ran off the edge instead of wrapping"
        );
    }

    #[gpui::test]
    fn emphasis_and_inline_code_wrap_with_the_text_around_them(cx: &mut TestAppContext) {
        let source = "Some leading prose, then **a long stretch of bold text that must wrap              rather than run off the edge**, then `a-long-inline-code-span --with --several              --flags --that --keep --going`, and a closing clause to finish the line.";

        let narrow = content_height(cx, source, NARROW);
        let wide = content_height(cx, source, WIDE);

        assert!(
            narrow > wide,
            "inline runs must wrap between and within their segments; got {narrow:?}              against {wide:?}"
        );
    }

    #[gpui::test]
    fn a_list_item_wraps_inside_its_own_column(cx: &mut TestAppContext) {
        let source = "- A list item whose text is long enough to need wrapping, repeated until              it certainly exceeds the pane width, repeated until it certainly exceeds it.";

        let narrow = content_height(cx, source, NARROW);
        let wide = content_height(cx, source, WIDE);

        assert!(
            narrow > wide,
            "a list item must wrap beside its marker; got {narrow:?} against {wide:?}"
        );
    }

    #[gpui::test]
    fn a_fenced_code_block_keeps_its_lines_intact_however_narrow_the_pane(cx: &mut TestAppContext) {
        let source = "```
bd memories --json | jq 'to_entries | map(select(.key !=              \"schema_version\")) | from_entries' # one deliberately long line of shell
```";

        let narrow = content_height(cx, source, NARROW);
        let wide = content_height(cx, source, WIDE);

        assert_eq!(
            narrow, wide,
            "code must not reflow: wrapping a command changes what it appears to say, so a              long line scrolls horizontally instead"
        );
    }

    fn fonts(body: Option<&str>, code: Option<&str>) -> MarkdownFonts {
        MarkdownFonts {
            body: body.map(str::to_owned),
            code: code.map(str::to_owned),
            base_size: appearance::UI_FONT_SIZE_DEFAULT as f32,
        }
    }

    /// The family an element asked for, or `None` when it made no
    /// `font_family` call at all.
    ///
    /// The difference between the two is the whole point: a family nobody
    /// configured has to leave the style untouched, because that is what lets
    /// the platform default stand.
    ///
    /// The font decisions are asserted on the built element rather than on a
    /// drawn one because gpui's test platform installs `NoopTextSystem`: it
    /// reports no installed families and gives every font the same metrics, so
    /// a laid-out pane cannot tell one family from another, or from none.
    fn requested_family(mut element: impl Styled) -> Option<String> {
        element
            .text_style()
            .as_ref()
            .and_then(|text| text.font_family.as_ref())
            .map(|family| family.to_string())
    }

    #[test]
    fn the_configured_code_family_reaches_code_blocks_and_inline_spans() {
        let fonts = fonts(None, Some("Cascadia Mono"));

        assert_eq!(
            requested_family(code_block_chrome(0, &fonts)).as_deref(),
            Some("Cascadia Mono")
        );
        assert_eq!(
            requested_family(inline_code_span("bd ready", &fonts)).as_deref(),
            Some("Cascadia Mono")
        );
    }

    #[test]
    fn the_configured_body_family_reaches_body_text_and_the_language_label() {
        let fonts = fonts(Some("Iosevka Etoile"), None);

        assert_eq!(
            requested_family(render_with_fonts("Body", &fonts, None)).as_deref(),
            Some("Iosevka Etoile"),
            "the body family belongs on the root, where every block inherits it"
        );
        assert_eq!(
            requested_family(language_label("rs", &fonts)).as_deref(),
            Some("Iosevka Etoile"),
            "the language label steps out of monospace into the Operator's body font, not into a hardcoded UI font"
        );
    }

    #[test]
    fn an_unconfigured_family_makes_no_font_family_call() {
        let fonts = fonts(None, None);

        for (surface, family) in [
            (
                "the document root",
                requested_family(render_with_fonts("Body", &fonts, None)),
            ),
            (
                "a code block",
                requested_family(code_block_chrome(0, &fonts)),
            ),
            (
                "an inline code span",
                requested_family(inline_code_span("bd ready", &fonts)),
            ),
            (
                "a language label",
                requested_family(language_label("rs", &fonts)),
            ),
        ] {
            assert_eq!(
                family, None,
                "{surface} must name no family at all when none is configured; naming one \
                 overrides the platform default that `None` asks for"
            );
        }
    }
}
