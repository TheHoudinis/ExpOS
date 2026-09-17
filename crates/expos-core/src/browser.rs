//! A deliberately small, allocation-free document, style, and script core.
//!
//! This module does not fetch resources, import code, evaluate arbitrary
//! JavaScript, expose storage, or retain references to the source document.
//! HTML, CSS, scripts, and event handlers are all subject to fixed limits.

pub const MAX_BROWSER_NODES: usize = 48;
pub const MAX_STYLE_RULES: usize = 32;
pub const MAX_BROWSER_SCRIPTS: usize = 16;
pub const MAX_SCRIPT_BYTES: usize = 512;
pub const MAX_SCRIPT_STATEMENTS: usize = 24;
pub const MAX_CLICK_HANDLERS: usize = 12;
pub const MAX_SEARCH_RESULTS: usize = 8;
pub const BROWSER_TEXT_CAPACITY: usize = 512;
pub const MAX_BROWSER_DOCUMENT_BYTES: usize = 16 * 1024;

const MAX_HANDLER_BYTES: usize = 384;
const STYLE_PROPERTY_COUNT: usize = 10;
const INLINE_SPECIFICITY: u16 = 1_000;
const SCRIPT_SPECIFICITY: u16 = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Title,
    Heading,
    Paragraph,
    Link,
    ListItem,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct BrowserText {
    bytes: [u8; BROWSER_TEXT_CAPACITY],
    length: u16,
}

impl BrowserText {
    pub const fn empty() -> Self {
        Self {
            bytes: [0; BROWSER_TEXT_CAPACITY],
            length: 0,
        }
    }

    pub fn new(value: &str) -> Result<Self, BrowserError> {
        let value = value.trim();
        if value.is_empty() || !value.is_ascii() {
            return Err(BrowserError::InvalidDocument);
        }
        Ok(Self::truncated(value))
    }

    pub fn as_str(&self) -> &str {
        // Every constructor accepts ASCII only.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.length as usize]) }
    }

    fn truncated(value: &str) -> Self {
        let mut text = Self::empty();
        let length = value.len().min(BROWSER_TEXT_CAPACITY);
        text.bytes[..length].copy_from_slice(&value.as_bytes()[..length]);
        text.length = length as u16;
        text
    }

    fn script_value(value: &str) -> Result<Self, ScriptRejection> {
        if !value.is_ascii() {
            return Err(ScriptRejection::NonAscii);
        }
        if value.len() > BROWSER_TEXT_CAPACITY {
            return Err(ScriptRejection::ValueTooLong);
        }
        Ok(Self::truncated(value))
    }
}

impl core::fmt::Debug for BrowserText {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_tuple("BrowserText")
            .field(&self.as_str())
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentNode {
    pub kind: NodeKind,
    pub text: BrowserText,
    pub target: BrowserText,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CssColor {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: u8,
}

impl CssColor {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);
    pub const BLACK: Self = Self::rgb(0, 0, 0);
    pub const WHITE: Self = Self::rgb(255, 255, 255);
    pub const RED: Self = Self::rgb(255, 0, 0);
    pub const GREEN: Self = Self::rgb(0, 128, 0);
    pub const BLUE: Self = Self::rgb(0, 102, 204);

    pub const fn rgb(red: u8, green: u8, blue: u8) -> Self {
        Self::rgba(red, green, blue, 255)
    }

    pub const fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxEdges {
    pub top: i16,
    pub right: i16,
    pub bottom: i16,
    pub left: i16,
}

impl BoxEdges {
    pub const ZERO: Self = Self::all(0);

    pub const fn all(value: i16) -> Self {
        Self {
            top: value,
            right: value,
            bottom: value,
            left: value,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderStyle {
    pub width: u16,
    pub color: CssColor,
}

impl BorderStyle {
    pub const NONE: Self = Self {
        width: 0,
        color: CssColor::BLACK,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayMode {
    None,
    Block,
    Inline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssVisibility {
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComputedStyle {
    pub color: CssColor,
    pub background: CssColor,
    pub border: BorderStyle,
    pub font_size: u16,
    pub font_weight: u16,
    pub display: DisplayMode,
    pub visibility: CssVisibility,
    pub margin: BoxEdges,
    pub padding: BoxEdges,
    pub text_align: TextAlign,
}

impl ComputedStyle {
    pub const DEFAULT: Self = Self {
        // ExpOS application surfaces are dark by default. Explicit CSS
        // `color: black` still selects true black.
        color: CssColor::rgb(217, 221, 217),
        background: CssColor::TRANSPARENT,
        border: BorderStyle::NONE,
        font_size: 16,
        font_weight: 400,
        display: DisplayMode::Block,
        visibility: CssVisibility::Visible,
        margin: BoxEdges::ZERO,
        padding: BoxEdges::ZERO,
        text_align: TextAlign::Left,
    };

    pub const fn is_rendered(&self) -> bool {
        !matches!(self.display, DisplayMode::None)
            && matches!(self.visibility, CssVisibility::Visible)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DomEvent {
    Click,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptRejection {
    EmptyScript,
    NonAscii,
    ScriptLimit,
    ScriptTooLong,
    TooManyStatements,
    TooManyHandlers,
    NestingLimit,
    ForbiddenApi,
    UnsupportedSyntax,
    TargetNotFound,
    ValueTooLong,
    InvalidStyle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptReport {
    pub scripts_seen: u16,
    pub scripts_executed: u16,
    pub scripts_rejected: u16,
    pub statements_executed: u16,
    pub handlers_registered: u16,
    pub clicks_dispatched: u16,
    pub limit_hits: u16,
    pub last_rejection: Option<ScriptRejection>,
}

impl ScriptReport {
    pub const EMPTY: Self = Self {
        scripts_seen: 0,
        scripts_executed: 0,
        scripts_rejected: 0,
        statements_executed: 0,
        handlers_registered: 0,
        clicks_dispatched: 0,
        limit_hits: 0,
        last_rejection: None,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrowserError {
    InvalidUrl,
    InvalidDocument,
    DocumentTooLarge,
    TooManyNodes,
    TooManyStyleRules,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StyledNode<'a> {
    pub index: usize,
    pub node: &'a DocumentNode,
    pub tag: &'a str,
    pub id: &'a str,
    pub classes: &'a str,
    pub style: &'a ComputedStyle,
    pub clickable: bool,
}

/// A script-free local projection of DuckDuckGo's HTML results page.
///
/// Only result titles and their link targets cross this boundary. Remote
/// styles, scripts, forms, images, and other resources are never copied into
/// the returned document.
pub struct SearchResultsDocument {
    pub document: Document,
    pub result_count: u8,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FixedText<const N: usize> {
    bytes: [u8; N],
    length: u8,
}

impl<const N: usize> FixedText<N> {
    const fn empty() -> Self {
        Self {
            bytes: [0; N],
            length: 0,
        }
    }

    fn set_truncated(&mut self, value: &str) {
        let length = value.len().min(N).min(u8::MAX as usize);
        self.bytes[..length].copy_from_slice(&value.as_bytes()[..length]);
        self.length = length as u8;
    }

    fn as_str(&self) -> &str {
        // FixedText is populated exclusively from an ASCII document.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.length as usize]) }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct ElementMeta {
    tag: FixedText<16>,
    id: FixedText<48>,
    classes: FixedText<80>,
}

impl ElementMeta {
    const fn empty() -> Self {
        Self {
            tag: FixedText::empty(),
            id: FixedText::empty(),
            classes: FixedText::empty(),
        }
    }
}

#[derive(Clone, Copy)]
struct CascadeState {
    priorities: [u32; STYLE_PROPERTY_COUNT],
}

impl CascadeState {
    const fn empty() -> Self {
        Self {
            priorities: [0; STYLE_PROPERTY_COUNT],
        }
    }
}

#[derive(Clone, Copy)]
struct ScriptText {
    bytes: [u8; MAX_HANDLER_BYTES],
    length: u16,
}

impl ScriptText {
    const fn empty() -> Self {
        Self {
            bytes: [0; MAX_HANDLER_BYTES],
            length: 0,
        }
    }

    fn new(value: &str) -> Result<Self, ScriptRejection> {
        if value.len() > MAX_HANDLER_BYTES {
            return Err(ScriptRejection::ScriptTooLong);
        }
        let mut script = Self::empty();
        script.bytes[..value.len()].copy_from_slice(value.as_bytes());
        script.length = value.len() as u16;
        Ok(script)
    }

    fn as_str(&self) -> &str {
        // Script sources are checked for ASCII before being stored.
        unsafe { core::str::from_utf8_unchecked(&self.bytes[..self.length as usize]) }
    }
}

#[derive(Clone, Copy)]
struct ClickHandler {
    node_index: u8,
    script: ScriptText,
}

#[derive(Clone, Copy)]
enum Selector<'a> {
    Tag(&'a str),
    Class(&'a str),
    Id(&'a str),
}

impl Selector<'_> {
    const fn specificity(self) -> u16 {
        match self {
            Self::Tag(_) => 1,
            Self::Class(_) => 10,
            Self::Id(_) => 100,
        }
    }
}

#[derive(Clone, Copy)]
enum StyleValue {
    Color(CssColor),
    Background(CssColor),
    Border(BorderStyle),
    FontSize(u16),
    FontWeight(u16),
    Display(DisplayMode),
    Visibility(CssVisibility),
    Margin(BoxEdges),
    Padding(BoxEdges),
    TextAlign(TextAlign),
}

impl StyleValue {
    const fn property_index(self) -> usize {
        match self {
            Self::Color(_) => 0,
            Self::Background(_) => 1,
            Self::Border(_) => 2,
            Self::FontSize(_) => 3,
            Self::FontWeight(_) => 4,
            Self::Display(_) => 5,
            Self::Visibility(_) => 6,
            Self::Margin(_) => 7,
            Self::Padding(_) => 8,
            Self::TextAlign(_) => 9,
        }
    }
}

#[derive(Clone, Copy)]
enum ScriptCommand<'a> {
    SetTitle(&'a str),
    SetText(usize, &'a str),
    SetStyle(usize, StyleValue),
    SetCssText(usize, &'a str),
    Hide(usize),
    Show(usize),
    RegisterClick {
        node_index: usize,
        body: &'a str,
        replace: bool,
    },
}

struct ValidationBudget {
    statements: usize,
    handlers: usize,
}

pub struct Document {
    url: BrowserText,
    title: BrowserText,
    nodes: [Option<DocumentNode>; MAX_BROWSER_NODES],
    elements: [ElementMeta; MAX_BROWSER_NODES],
    styles: [ComputedStyle; MAX_BROWSER_NODES],
    cascade: [CascadeState; MAX_BROWSER_NODES],
    hidden_display: [Option<DisplayMode>; MAX_BROWSER_NODES],
    handlers: [Option<ClickHandler>; MAX_CLICK_HANDLERS],
    report: ScriptReport,
    count: usize,
    style_order: u16,
    style_rule_count: u16,
}

impl Document {
    pub fn parse(url: &str, source: &str) -> Result<Self, BrowserError> {
        if !url.starts_with("expos://")
            && !url.starts_with("data:text/html,")
            && !url.starts_with("http://")
            && !url.starts_with("https://")
        {
            return Err(BrowserError::InvalidUrl);
        }
        if !source.is_ascii() {
            return Err(BrowserError::InvalidDocument);
        }
        if source.len() > MAX_BROWSER_DOCUMENT_BYTES {
            return Err(BrowserError::DocumentTooLarge);
        }

        let mut document = Self {
            url: BrowserText::new(url)?,
            title: BrowserText::empty(),
            nodes: [None; MAX_BROWSER_NODES],
            elements: [ElementMeta::empty(); MAX_BROWSER_NODES],
            styles: [ComputedStyle::DEFAULT; MAX_BROWSER_NODES],
            cascade: [CascadeState::empty(); MAX_BROWSER_NODES],
            hidden_display: [None; MAX_BROWSER_NODES],
            handlers: [None; MAX_CLICK_HANDLERS],
            report: ScriptReport::EMPTY,
            count: 0,
            style_order: 0,
            style_rule_count: 0,
        };
        let mut node_tag_offsets = [0usize; MAX_BROWSER_NODES];
        document.parse_nodes(source, &mut node_tag_offsets)?;
        if document.count == 0 {
            return Err(BrowserError::InvalidDocument);
        }
        document.apply_stylesheets(source)?;
        document.apply_inline_styles(source, &node_tag_offsets);
        document.register_inline_handlers(source, &node_tag_offsets);
        document.execute_embedded_scripts(source);
        Ok(document)
    }

    /// Project DuckDuckGo's non-JavaScript HTML results into a small local
    /// document containing only result titles and authenticated link targets.
    /// Remote scripts, styles, forms, images, and tracking markup are dropped.
    pub fn parse_duckduckgo_results(
        url: &str,
        source: &str,
    ) -> Result<SearchResultsDocument, BrowserError> {
        if !is_duckduckgo_html_url(url) {
            return Err(BrowserError::InvalidUrl);
        }
        if !source.is_ascii() {
            return Err(BrowserError::InvalidDocument);
        }
        if source.len() > MAX_BROWSER_DOCUMENT_BYTES {
            return Err(BrowserError::DocumentTooLarge);
        }

        let mut document = Self {
            url: BrowserText::new(url)?,
            title: BrowserText::new("DuckDuckGo results")?,
            nodes: [None; MAX_BROWSER_NODES],
            elements: [ElementMeta::empty(); MAX_BROWSER_NODES],
            styles: [ComputedStyle::DEFAULT; MAX_BROWSER_NODES],
            cascade: [CascadeState::empty(); MAX_BROWSER_NODES],
            hidden_display: [None; MAX_BROWSER_NODES],
            handlers: [None; MAX_CLICK_HANDLERS],
            report: ScriptReport::EMPTY,
            count: 0,
            style_order: 0,
            style_rule_count: 0,
        };
        document.push_projected_node(
            NodeKind::Title,
            "title",
            BrowserText::new("DuckDuckGo results")?,
            BrowserText::empty(),
            "",
        )?;
        document.push_projected_node(
            NodeKind::Heading,
            "h1",
            BrowserText::new("DuckDuckGo results")?,
            BrowserText::empty(),
            "",
        )?;

        let mut cursor = 0;
        let mut result_count = 0_u8;
        while (result_count as usize) < MAX_SEARCH_RESULTS {
            let Some((_, tag_end, tag)) = next_tag(source, cursor) else {
                break;
            };
            cursor = tag_end + 1;
            let tag = tag.trim();
            if !eq_ascii(tag_name(tag), "a")
                || !attribute(tag, "class").is_some_and(|classes| {
                    classes
                        .split_ascii_whitespace()
                        .any(|class| eq_ascii(class, "result__a"))
                })
            {
                continue;
            }
            let Some(href) = attribute(tag, "href") else {
                continue;
            };
            let Some((closing_start, closing_end)) = find_closing_tag(source, cursor, "a") else {
                break;
            };
            let mut decoded_href = [0_u8; BROWSER_TEXT_CAPACITY];
            let Some(href_len) = decode_search_href(href, &mut decoded_href) else {
                cursor = closing_end;
                continue;
            };
            let mut visible_title = [0_u8; BROWSER_TEXT_CAPACITY];
            let title_len =
                flatten_search_title(&source[cursor..closing_start], &mut visible_title);
            cursor = closing_end;
            if href_len == 0 || title_len == 0 {
                continue;
            }
            let href = core::str::from_utf8(&decoded_href[..href_len])
                .map_err(|_| BrowserError::InvalidDocument)?;
            let title = core::str::from_utf8(&visible_title[..title_len])
                .map_err(|_| BrowserError::InvalidDocument)?;
            document.push_projected_node(
                NodeKind::Link,
                "a",
                BrowserText::new(title)?,
                BrowserText::new(href)?,
                "result",
            )?;
            let index = document.count - 1;
            document.styles[index].display = DisplayMode::Block;
            document.styles[index].color = CssColor::rgb(217, 221, 217);
            document.styles[index].background = CssColor::rgb(21, 28, 32);
            document.styles[index].border = BorderStyle {
                width: 1,
                color: CssColor::rgb(53, 67, 63),
            };
            document.styles[index].margin = BoxEdges::all(4);
            document.styles[index].padding = BoxEdges::all(6);
            result_count = result_count.saturating_add(1);
        }
        if result_count == 0 {
            return Err(BrowserError::InvalidDocument);
        }
        Ok(SearchResultsDocument {
            document,
            result_count,
        })
    }

    fn push_projected_node(
        &mut self,
        kind: NodeKind,
        tag: &str,
        text: BrowserText,
        target: BrowserText,
        classes: &str,
    ) -> Result<(), BrowserError> {
        let index = self.push(DocumentNode { kind, text, target })?;
        self.elements[index].tag.set_truncated(tag);
        self.elements[index].classes.set_truncated(classes);
        self.styles[index] = default_style(kind, tag);
        Ok(())
    }

    pub fn nodes(&self) -> impl Iterator<Item = &DocumentNode> {
        self.nodes[..self.count].iter().flatten()
    }

    pub fn styled_nodes(&self) -> impl Iterator<Item = StyledNode<'_>> {
        (0..self.count).filter_map(move |index| self.styled_node(index))
    }

    pub fn styled_node(&self, index: usize) -> Option<StyledNode<'_>> {
        let node = self.nodes.get(index)?.as_ref()?;
        let element = &self.elements[index];
        Some(StyledNode {
            index,
            node,
            tag: element.tag.as_str(),
            id: element.id.as_str(),
            classes: element.classes.as_str(),
            style: &self.styles[index],
            clickable: self.has_click_handler(index),
        })
    }

    pub fn element_by_id(&self, id: &str) -> Option<StyledNode<'_>> {
        self.find_id(id).and_then(|index| self.styled_node(index))
    }

    pub fn style_of(&self, id: &str) -> Option<&ComputedStyle> {
        self.find_id(id).map(|index| &self.styles[index])
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn url(&self) -> &str {
        self.url.as_str()
    }

    pub fn title(&self) -> &str {
        self.title.as_str()
    }

    pub const fn script_report(&self) -> ScriptReport {
        self.report
    }

    pub const fn style_rule_count(&self) -> u16 {
        self.style_rule_count
    }

    /// Executes the supported deterministic script subset.
    ///
    /// Rejected programs are validated before execution and make no DOM or
    /// style changes. The rejection is also reflected in `script_report()`.
    pub fn execute_script(&mut self, source: &str) -> Result<(), ScriptRejection> {
        self.note_script_seen()?;
        if source.trim().is_empty() {
            return Err(self.reject_script(ScriptRejection::EmptyScript));
        }
        if !source.is_ascii() {
            return Err(self.reject_script(ScriptRejection::NonAscii));
        }
        if source.len() > MAX_SCRIPT_BYTES {
            return Err(self.reject_script(ScriptRejection::ScriptTooLong));
        }
        if contains_forbidden_api(source) {
            return Err(self.reject_script(ScriptRejection::ForbiddenApi));
        }
        let mut budget = ValidationBudget {
            statements: 0,
            handlers: 0,
        };
        if let Err(error) = self.validate_program(source, None, 0, &mut budget) {
            return Err(self.reject_script(error));
        }
        self.report.scripts_executed = self.report.scripts_executed.saturating_add(1);
        // Validation guarantees execution cannot fail or partially reject.
        let _ = self.run_program(source, None);
        Ok(())
    }

    pub fn dispatch_event(&mut self, id: &str, event: DomEvent) -> bool {
        let Some(index) = self.find_id(id) else {
            return false;
        };
        self.dispatch_event_at_node(index, event)
    }

    pub fn dispatch_event_at_node(&mut self, index: usize, event: DomEvent) -> bool {
        if index >= self.count {
            return false;
        }
        match event {
            DomEvent::Click => {
                self.report.clicks_dispatched = self.report.clicks_dispatched.saturating_add(1);
                let mut handled = false;
                for handler_index in 0..MAX_CLICK_HANDLERS {
                    let Some(handler) = self.handlers[handler_index] else {
                        continue;
                    };
                    if handler.node_index as usize != index {
                        continue;
                    }
                    handled = true;
                    self.report.scripts_executed = self.report.scripts_executed.saturating_add(1);
                    let _ = self.run_program(handler.script.as_str(), Some(index));
                }
                handled
            }
        }
    }

    pub fn dispatch_click(&mut self, id: &str) -> bool {
        self.dispatch_event(id, DomEvent::Click)
    }

    pub fn dispatch_click_at_node(&mut self, index: usize) -> bool {
        self.dispatch_event_at_node(index, DomEvent::Click)
    }

    fn parse_nodes(
        &mut self,
        source: &str,
        offsets: &mut [usize; MAX_BROWSER_NODES],
    ) -> Result<(), BrowserError> {
        let mut cursor = 0;
        while let Some((tag_start, tag_end, tag)) = next_tag(source, cursor) {
            cursor = tag_end + 1;
            let tag = tag.trim();
            if tag.starts_with('/') || tag.starts_with('!') || tag.starts_with('?') {
                continue;
            }
            let name = tag_name(tag);
            if eq_ascii(name, "style") || eq_ascii(name, "script") {
                if let Some((_, closing_end)) = find_closing_tag(source, cursor, name) {
                    cursor = closing_end;
                } else {
                    // Never reinterpret unterminated style or script text as DOM.
                    cursor = source.len();
                }
                continue;
            }
            let Some(kind) = node_kind(name) else {
                continue;
            };
            let content_end = source[cursor..]
                .find('<')
                .map(|offset| cursor + offset)
                .unwrap_or(source.len());
            let content = source[cursor..content_end].trim();
            let id = attribute(tag, "id").unwrap_or("");
            let has_behavior = !id.is_empty()
                || attribute(tag, "onclick").is_some()
                || attribute(tag, "style").is_some();
            if content.is_empty() && !has_behavior {
                continue;
            }
            let target = if kind == NodeKind::Link {
                attribute(tag, "href")
                    .and_then(|value| BrowserText::new(value).ok())
                    .unwrap_or(BrowserText::empty())
            } else {
                BrowserText::empty()
            };
            let text = if content.is_empty() {
                BrowserText::empty()
            } else {
                BrowserText::new(content)?
            };
            let index = self.push(DocumentNode { kind, text, target })?;
            offsets[index] = tag_start;
            self.elements[index].tag.set_truncated(name);
            self.elements[index].id.set_truncated(id);
            self.elements[index]
                .classes
                .set_truncated(attribute(tag, "class").unwrap_or(""));
            self.styles[index] = default_style(kind, name);
            if kind == NodeKind::Title && self.title.as_str().is_empty() {
                self.title = text;
            }
        }
        Ok(())
    }

    fn push(&mut self, node: DocumentNode) -> Result<usize, BrowserError> {
        let index = self.count;
        let slot = self
            .nodes
            .get_mut(index)
            .ok_or(BrowserError::TooManyNodes)?;
        *slot = Some(node);
        self.count += 1;
        Ok(index)
    }

    fn apply_stylesheets(&mut self, source: &str) -> Result<(), BrowserError> {
        let mut cursor = 0;
        let mut rules = 0usize;
        while let Some((_, tag_end, tag)) = next_tag(source, cursor) {
            cursor = tag_end + 1;
            let tag = tag.trim();
            if tag.starts_with('/') || !eq_ascii(tag_name(tag), "style") {
                continue;
            }
            let Some((closing_start, closing_end)) = find_closing_tag(source, cursor, "style")
            else {
                break;
            };
            let css = &source[cursor..closing_start];
            self.apply_stylesheet(css, &mut rules)?;
            cursor = closing_end;
        }
        self.style_rule_count = rules as u16;
        Ok(())
    }

    fn apply_stylesheet(&mut self, css: &str, rules: &mut usize) -> Result<(), BrowserError> {
        let mut cursor = 0;
        while let Some(open_offset) = css[cursor..].find('{') {
            let open = cursor + open_offset;
            let Some(close_offset) = css[open + 1..].find('}') else {
                break;
            };
            let close = open + 1 + close_offset;
            let selectors = css[cursor..open].trim();
            let declarations = &css[open + 1..close];
            for selector_text in selectors.split(',') {
                let Some(selector) = parse_selector(selector_text.trim()) else {
                    continue;
                };
                if *rules >= MAX_STYLE_RULES {
                    return Err(BrowserError::TooManyStyleRules);
                }
                *rules += 1;
                self.style_order = self.style_order.saturating_add(1);
                let order = self.style_order;
                for index in 0..self.count {
                    if self.selector_matches(index, selector) {
                        self.apply_declarations(
                            index,
                            declarations,
                            selector.specificity(),
                            order,
                            false,
                        );
                    }
                }
            }
            cursor = close + 1;
        }
        Ok(())
    }

    fn apply_inline_styles(&mut self, source: &str, offsets: &[usize; MAX_BROWSER_NODES]) {
        for (index, offset) in offsets.iter().copied().enumerate().take(self.count) {
            let Some((_, _, tag)) = next_tag(source, offset) else {
                continue;
            };
            let Some(declarations) = attribute(tag.trim(), "style") else {
                continue;
            };
            self.style_order = self.style_order.saturating_add(1);
            self.apply_declarations(
                index,
                declarations,
                INLINE_SPECIFICITY,
                self.style_order,
                false,
            );
        }
    }

    fn register_inline_handlers(&mut self, source: &str, offsets: &[usize; MAX_BROWSER_NODES]) {
        for (index, offset) in offsets.iter().copied().enumerate().take(self.count) {
            let Some((_, _, tag)) = next_tag(source, offset) else {
                continue;
            };
            let Some(handler) = attribute(tag.trim(), "onclick") else {
                continue;
            };
            let _ = self.submit_inline_handler(index, handler);
        }
    }

    fn execute_embedded_scripts(&mut self, source: &str) {
        let mut cursor = 0;
        while let Some((_, tag_end, tag)) = next_tag(source, cursor) {
            cursor = tag_end + 1;
            let tag = tag.trim();
            if tag.starts_with('/') || !eq_ascii(tag_name(tag), "script") {
                continue;
            }
            let closing = find_closing_tag(source, cursor, "script");
            let (script_end, closing_end) = closing.unwrap_or((source.len(), source.len()));
            if attribute(tag, "src").is_some() {
                if self.note_script_seen().is_ok() {
                    self.reject_script(ScriptRejection::ForbiddenApi);
                }
            } else {
                let _ = self.execute_script(&source[cursor..script_end]);
            }
            cursor = closing_end;
        }
    }

    fn apply_declarations(
        &mut self,
        index: usize,
        declarations: &str,
        specificity: u16,
        order: u16,
        strict: bool,
    ) -> bool {
        let mut applied = false;
        for declaration in declarations.split(';') {
            let declaration = declaration.trim();
            if declaration.is_empty() {
                continue;
            }
            let Some((property, value)) = declaration.split_once(':') else {
                if strict {
                    return false;
                }
                continue;
            };
            let Some(style_value) = parse_style_value(property.trim(), value.trim()) else {
                if strict {
                    return false;
                }
                continue;
            };
            self.apply_style_value(index, style_value, specificity, order);
            applied = true;
        }
        applied
    }

    fn apply_style_value(&mut self, index: usize, value: StyleValue, specificity: u16, order: u16) {
        let property = value.property_index();
        let priority = ((specificity as u32) << 16) | order as u32;
        if priority < self.cascade[index].priorities[property] {
            return;
        }
        self.cascade[index].priorities[property] = priority;
        let style = &mut self.styles[index];
        match value {
            StyleValue::Color(value) => style.color = value,
            StyleValue::Background(value) => style.background = value,
            StyleValue::Border(value) => style.border = value,
            StyleValue::FontSize(value) => style.font_size = value,
            StyleValue::FontWeight(value) => style.font_weight = value,
            StyleValue::Display(value) => style.display = value,
            StyleValue::Visibility(value) => style.visibility = value,
            StyleValue::Margin(value) => style.margin = value,
            StyleValue::Padding(value) => style.padding = value,
            StyleValue::TextAlign(value) => style.text_align = value,
        }
    }

    fn selector_matches(&self, index: usize, selector: Selector<'_>) -> bool {
        let element = &self.elements[index];
        match selector {
            Selector::Tag(tag) => eq_ascii(element.tag.as_str(), tag),
            Selector::Class(class) => element
                .classes
                .as_str()
                .split_ascii_whitespace()
                .any(|candidate| candidate == class),
            Selector::Id(id) => element.id.as_str() == id,
        }
    }

    fn find_id(&self, id: &str) -> Option<usize> {
        if id.is_empty() {
            return None;
        }
        (0..self.count).find(|index| self.elements[*index].id.as_str() == id)
    }

    fn has_click_handler(&self, index: usize) -> bool {
        self.handlers
            .iter()
            .flatten()
            .any(|handler| handler.node_index as usize == index)
    }

    fn note_script_seen(&mut self) -> Result<(), ScriptRejection> {
        self.report.scripts_seen = self.report.scripts_seen.saturating_add(1);
        if self.report.scripts_seen as usize > MAX_BROWSER_SCRIPTS {
            return Err(self.reject_script(ScriptRejection::ScriptLimit));
        }
        Ok(())
    }

    fn reject_script(&mut self, rejection: ScriptRejection) -> ScriptRejection {
        self.report.scripts_rejected = self.report.scripts_rejected.saturating_add(1);
        self.report.last_rejection = Some(rejection);
        if matches!(
            rejection,
            ScriptRejection::ScriptLimit
                | ScriptRejection::ScriptTooLong
                | ScriptRejection::TooManyStatements
                | ScriptRejection::TooManyHandlers
                | ScriptRejection::NestingLimit
                | ScriptRejection::ValueTooLong
        ) {
            self.report.limit_hits = self.report.limit_hits.saturating_add(1);
        }
        rejection
    }

    fn submit_inline_handler(
        &mut self,
        node_index: usize,
        source: &str,
    ) -> Result<(), ScriptRejection> {
        self.note_script_seen()?;
        if source.trim().is_empty() {
            return Err(self.reject_script(ScriptRejection::EmptyScript));
        }
        if !source.is_ascii() {
            return Err(self.reject_script(ScriptRejection::NonAscii));
        }
        if source.len() > MAX_HANDLER_BYTES {
            return Err(self.reject_script(ScriptRejection::ScriptTooLong));
        }
        if contains_forbidden_api(source) {
            return Err(self.reject_script(ScriptRejection::ForbiddenApi));
        }
        let mut budget = ValidationBudget {
            statements: 0,
            handlers: 0,
        };
        if let Err(error) = self.validate_program(source, Some(node_index), 1, &mut budget) {
            return Err(self.reject_script(error));
        }
        if self.handler_count() >= MAX_CLICK_HANDLERS {
            return Err(self.reject_script(ScriptRejection::TooManyHandlers));
        }
        let script = ScriptText::new(source).map_err(|error| self.reject_script(error))?;
        self.install_handler(node_index, script, true)
            .map_err(|error| self.reject_script(error))?;
        Ok(())
    }

    fn validate_program(
        &self,
        source: &str,
        implicit_node: Option<usize>,
        depth: usize,
        budget: &mut ValidationBudget,
    ) -> Result<(), ScriptRejection> {
        if depth > 2 {
            return Err(ScriptRejection::NestingLimit);
        }
        let mut cursor = 0;
        let mut found = false;
        while let Some(statement) = next_statement(source, &mut cursor)? {
            found = true;
            budget.statements += 1;
            if budget.statements > MAX_SCRIPT_STATEMENTS {
                return Err(ScriptRejection::TooManyStatements);
            }
            let command = self.parse_command(statement, implicit_node)?;
            match command {
                ScriptCommand::SetTitle(value) | ScriptCommand::SetText(_, value) => {
                    BrowserText::script_value(value)?;
                }
                ScriptCommand::SetStyle(_, _) | ScriptCommand::Hide(_) | ScriptCommand::Show(_) => {
                }
                ScriptCommand::SetCssText(_, declarations) => {
                    if !validate_css_text(declarations) {
                        return Err(ScriptRejection::InvalidStyle);
                    }
                }
                ScriptCommand::RegisterClick {
                    node_index, body, ..
                } => {
                    budget.handlers += 1;
                    if self.handler_count() + budget.handlers > MAX_CLICK_HANDLERS {
                        return Err(ScriptRejection::TooManyHandlers);
                    }
                    if body.len() > MAX_HANDLER_BYTES {
                        return Err(ScriptRejection::ScriptTooLong);
                    }
                    if contains_forbidden_api(body) {
                        return Err(ScriptRejection::ForbiddenApi);
                    }
                    self.validate_program(body, Some(node_index), depth + 1, budget)?;
                }
            }
        }
        if !found {
            return Err(ScriptRejection::EmptyScript);
        }
        Ok(())
    }

    fn parse_command<'a>(
        &self,
        statement: &'a str,
        implicit_node: Option<usize>,
    ) -> Result<ScriptCommand<'a>, ScriptRejection> {
        let statement = statement.trim();

        if let Some(arguments) = call_arguments(statement, "title") {
            let [value] = parse_arguments::<1>(arguments)?;
            return Ok(ScriptCommand::SetTitle(value));
        }
        if let Some(arguments) = call_arguments(statement, "text") {
            let [id, value] = parse_arguments::<2>(arguments)?;
            return Ok(ScriptCommand::SetText(self.require_id(id)?, value));
        }
        if let Some(arguments) = call_arguments(statement, "style") {
            let [id, property, value] = parse_arguments::<3>(arguments)?;
            let style = parse_style_value(property, value).ok_or(ScriptRejection::InvalidStyle)?;
            return Ok(ScriptCommand::SetStyle(self.require_id(id)?, style));
        }
        if let Some(arguments) = call_arguments(statement, "hide") {
            let [id] = parse_arguments::<1>(arguments)?;
            return Ok(ScriptCommand::Hide(self.require_id(id)?));
        }
        if let Some(arguments) = call_arguments(statement, "show") {
            let [id] = parse_arguments::<1>(arguments)?;
            return Ok(ScriptCommand::Show(self.require_id(id)?));
        }
        if let Some(arguments) = call_arguments(statement, "onClick") {
            let [id, body] = parse_arguments::<2>(arguments)?;
            return Ok(ScriptCommand::RegisterClick {
                node_index: self.require_id(id)?,
                body,
                replace: true,
            });
        }

        let Some((left, right)) = split_assignment(statement) else {
            if let Some((node_index, tail)) =
                self.parse_element_expression(statement, implicit_node)?
            {
                if eq_ascii(tail.trim(), ".hide()") {
                    return Ok(ScriptCommand::Hide(node_index));
                }
                if eq_ascii(tail.trim(), ".show()") {
                    return Ok(ScriptCommand::Show(node_index));
                }
                if let Some(body) = parse_add_event_listener(tail)? {
                    return Ok(ScriptCommand::RegisterClick {
                        node_index,
                        body,
                        replace: false,
                    });
                }
            }
            return Err(ScriptRejection::UnsupportedSyntax);
        };
        if eq_ascii(left.trim(), "document.title") {
            return Ok(ScriptCommand::SetTitle(parse_quoted_value(right)?));
        }
        let (node_index, tail) = self
            .parse_element_expression(left.trim(), implicit_node)?
            .ok_or(ScriptRejection::UnsupportedSyntax)?;
        let tail = tail.trim();
        if eq_ascii(tail, ".textContent") || eq_ascii(tail, ".innerText") {
            return Ok(ScriptCommand::SetText(
                node_index,
                parse_quoted_value(right)?,
            ));
        }
        if eq_ascii(tail, ".hidden") {
            return match right.trim() {
                "true" => Ok(ScriptCommand::Hide(node_index)),
                "false" => Ok(ScriptCommand::Show(node_index)),
                _ => Err(ScriptRejection::UnsupportedSyntax),
            };
        }
        if eq_ascii(tail, ".onclick") {
            return Ok(ScriptCommand::RegisterClick {
                node_index,
                body: parse_handler_body(right)?,
                replace: true,
            });
        }
        let Some(property) = strip_prefix_ascii(tail, ".style.") else {
            return Err(ScriptRejection::UnsupportedSyntax);
        };
        let value = parse_quoted_value(right)?;
        if eq_ascii(property.trim(), "cssText") {
            return Ok(ScriptCommand::SetCssText(node_index, value));
        }
        let style =
            parse_style_value(property.trim(), value).ok_or(ScriptRejection::InvalidStyle)?;
        Ok(ScriptCommand::SetStyle(node_index, style))
    }

    fn parse_element_expression<'a>(
        &self,
        expression: &'a str,
        implicit_node: Option<usize>,
    ) -> Result<Option<(usize, &'a str)>, ScriptRejection> {
        let expression = expression.trim();
        if let Some(tail) = strip_prefix_ascii(expression, "this") {
            return implicit_node
                .map(|index| Some((index, tail)))
                .ok_or(ScriptRejection::UnsupportedSyntax);
        }
        let Some(mut rest) = strip_prefix_ascii(expression, "document.getElementById") else {
            return Ok(None);
        };
        rest = rest.trim_start();
        let Some(arguments) = rest.strip_prefix('(') else {
            return Err(ScriptRejection::UnsupportedSyntax);
        };
        let (id, consumed) = parse_quoted(arguments)?;
        let tail = arguments[consumed..].trim_start();
        let Some(tail) = tail.strip_prefix(')') else {
            return Err(ScriptRejection::UnsupportedSyntax);
        };
        Ok(Some((self.require_id(id)?, tail)))
    }

    fn require_id(&self, id: &str) -> Result<usize, ScriptRejection> {
        self.find_id(id).ok_or(ScriptRejection::TargetNotFound)
    }

    fn run_program(
        &mut self,
        source: &str,
        implicit_node: Option<usize>,
    ) -> Result<(), ScriptRejection> {
        let mut cursor = 0;
        while let Some(statement) = next_statement(source, &mut cursor)? {
            let command = self.parse_command(statement, implicit_node)?;
            self.run_command(command)?;
            self.report.statements_executed = self.report.statements_executed.saturating_add(1);
        }
        Ok(())
    }

    fn run_command(&mut self, command: ScriptCommand<'_>) -> Result<(), ScriptRejection> {
        match command {
            ScriptCommand::SetTitle(value) => {
                let text = BrowserText::script_value(value)?;
                self.title = text;
                for node in self.nodes[..self.count].iter_mut().flatten() {
                    if node.kind == NodeKind::Title {
                        node.text = text;
                        break;
                    }
                }
            }
            ScriptCommand::SetText(index, value) => {
                self.nodes[index].as_mut().unwrap().text = BrowserText::script_value(value)?;
            }
            ScriptCommand::SetStyle(index, value) => self.apply_script_style(index, value),
            ScriptCommand::SetCssText(index, declarations) => {
                self.style_order = self.style_order.saturating_add(1);
                let order = self.style_order;
                if !self.apply_declarations(index, declarations, SCRIPT_SPECIFICITY, order, true) {
                    return Err(ScriptRejection::InvalidStyle);
                }
            }
            ScriptCommand::Hide(index) => {
                let current = self.styles[index].display;
                if current != DisplayMode::None {
                    self.hidden_display[index] = Some(current);
                }
                self.apply_script_style(index, StyleValue::Display(DisplayMode::None));
            }
            ScriptCommand::Show(index) => {
                let display = self.hidden_display[index]
                    .take()
                    .unwrap_or_else(|| default_display(self.elements[index].tag.as_str()));
                self.apply_script_style(index, StyleValue::Display(display));
            }
            ScriptCommand::RegisterClick {
                node_index,
                body,
                replace,
            } => {
                let script = ScriptText::new(body)?;
                self.install_handler(node_index, script, replace)?;
            }
        }
        Ok(())
    }

    fn apply_script_style(&mut self, index: usize, value: StyleValue) {
        self.style_order = self.style_order.saturating_add(1);
        self.apply_style_value(index, value, SCRIPT_SPECIFICITY, self.style_order);
    }

    fn handler_count(&self) -> usize {
        self.handlers.iter().flatten().count()
    }

    fn install_handler(
        &mut self,
        node_index: usize,
        script: ScriptText,
        replace: bool,
    ) -> Result<(), ScriptRejection> {
        if replace {
            let mut replacement = None;
            for (index, slot) in self.handlers.iter_mut().enumerate() {
                if slot
                    .as_ref()
                    .is_some_and(|handler| handler.node_index as usize == node_index)
                {
                    if replacement.is_none() {
                        replacement = Some(index);
                    } else {
                        *slot = None;
                    }
                }
            }
            if let Some(index) = replacement {
                self.handlers[index] = Some(ClickHandler {
                    node_index: node_index as u8,
                    script,
                });
                self.report.handlers_registered = self.report.handlers_registered.saturating_add(1);
                return Ok(());
            }
        }
        let slot = self
            .handlers
            .iter_mut()
            .find(|handler| handler.is_none())
            .ok_or(ScriptRejection::TooManyHandlers)?;
        *slot = Some(ClickHandler {
            node_index: node_index as u8,
            script,
        });
        self.report.handlers_registered = self.report.handlers_registered.saturating_add(1);
        Ok(())
    }
}

fn default_style(kind: NodeKind, tag: &str) -> ComputedStyle {
    let mut style = ComputedStyle::DEFAULT;
    style.display = default_display(tag);
    match kind {
        NodeKind::Title => style.display = DisplayMode::None,
        NodeKind::Heading => {
            style.font_size = if eq_ascii(tag, "h1") { 28 } else { 22 };
            style.font_weight = 700;
        }
        NodeKind::Link => style.color = CssColor::BLUE,
        NodeKind::Paragraph | NodeKind::ListItem => {}
    }
    style
}

fn default_display(tag: &str) -> DisplayMode {
    if eq_ascii(tag, "a") || eq_ascii(tag, "span") || eq_ascii(tag, "button") {
        DisplayMode::Inline
    } else if eq_ascii(tag, "title") {
        DisplayMode::None
    } else {
        DisplayMode::Block
    }
}

fn node_kind(name: &str) -> Option<NodeKind> {
    if eq_ascii(name, "title") {
        Some(NodeKind::Title)
    } else if ["h1", "h2", "h3", "h4", "h5", "h6"]
        .iter()
        .any(|heading| eq_ascii(name, heading))
    {
        Some(NodeKind::Heading)
    } else if eq_ascii(name, "p")
        || eq_ascii(name, "div")
        || eq_ascii(name, "span")
        || eq_ascii(name, "label")
    {
        Some(NodeKind::Paragraph)
    } else if eq_ascii(name, "a") || eq_ascii(name, "button") {
        Some(NodeKind::Link)
    } else if eq_ascii(name, "li") {
        Some(NodeKind::ListItem)
    } else {
        None
    }
}

fn next_tag(source: &str, cursor: usize) -> Option<(usize, usize, &str)> {
    let relative_start = source.get(cursor..)?.find('<')?;
    let start = cursor + relative_start;
    let relative_end = source.get(start + 1..)?.find('>')?;
    let end = start + 1 + relative_end;
    Some((start, end, &source[start + 1..end]))
}

fn find_closing_tag(source: &str, cursor: usize, name: &str) -> Option<(usize, usize)> {
    let mut search = cursor;
    while let Some((start, end, tag)) = next_tag(source, search) {
        let tag = tag.trim();
        if let Some(closing) = tag.strip_prefix('/') {
            if eq_ascii(tag_name(closing.trim_start()), name) {
                return Some((start, end + 1));
            }
        }
        search = end + 1;
    }
    None
}

fn tag_name(tag: &str) -> &str {
    let end = tag
        .bytes()
        .position(|byte| byte.is_ascii_whitespace() || byte == b'/')
        .unwrap_or(tag.len());
    &tag[..end]
}

fn attribute<'a>(tag: &'a str, wanted: &str) -> Option<&'a str> {
    let mut cursor = tag_name(tag).len();
    let bytes = tag.as_bytes();
    while cursor < tag.len() {
        while cursor < tag.len() && (bytes[cursor].is_ascii_whitespace() || bytes[cursor] == b'/') {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < tag.len()
            && !bytes[cursor].is_ascii_whitespace()
            && bytes[cursor] != b'='
            && bytes[cursor] != b'/'
        {
            cursor += 1;
        }
        if name_start == cursor {
            break;
        }
        let name = &tag[name_start..cursor];
        while cursor < tag.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= tag.len() || bytes[cursor] != b'=' {
            if eq_ascii(name, wanted) {
                return Some("");
            }
            continue;
        }
        cursor += 1;
        while cursor < tag.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        let value;
        if cursor < tag.len() && (bytes[cursor] == b'\'' || bytes[cursor] == b'"') {
            let quote = bytes[cursor];
            cursor += 1;
            let start = cursor;
            while cursor < tag.len() && bytes[cursor] != quote {
                cursor += 1;
            }
            value = &tag[start..cursor];
            if cursor < tag.len() {
                cursor += 1;
            }
        } else {
            let start = cursor;
            while cursor < tag.len()
                && !bytes[cursor].is_ascii_whitespace()
                && bytes[cursor] != b'/'
            {
                cursor += 1;
            }
            value = &tag[start..cursor];
        }
        if eq_ascii(name, wanted) {
            return Some(value);
        }
    }
    None
}

fn is_duckduckgo_html_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let host = &rest[..authority_end];
    let path = rest.get(authority_end..).unwrap_or("");
    (eq_ascii(host, "duckduckgo.com") || eq_ascii(host, "html.duckduckgo.com"))
        && (path.starts_with("/html/") || path.starts_with("/html?"))
}

fn decode_search_href(value: &str, output: &mut [u8]) -> Option<usize> {
    let bytes = value.as_bytes();
    let mut input = 0;
    let mut length = 0;
    while input < bytes.len() {
        let (byte, consumed) =
            if bytes[input..].starts_with(b"&amp;") || bytes[input..].starts_with(b"&#38;") {
                (b'&', 5)
            } else if bytes[input..].starts_with(b"&#x26;") {
                (b'&', 6)
            } else {
                (bytes[input], 1)
            };
        if !(0x21..=0x7e).contains(&byte) || matches!(byte, b'<' | b'>') {
            return None;
        }
        *output.get_mut(length)? = byte;
        length += 1;
        input += consumed;
    }
    Some(length)
}

fn flatten_search_title(value: &str, output: &mut [u8]) -> usize {
    let bytes = value.as_bytes();
    let mut input = 0;
    let mut length = 0;
    let mut inside_tag = false;
    let mut pending_space = false;
    while input < bytes.len() && length < output.len() {
        if bytes[input] == b'<' {
            inside_tag = true;
            pending_space |= length != 0;
            input += 1;
            continue;
        }
        if inside_tag {
            inside_tag = bytes[input] != b'>';
            input += 1;
            continue;
        }
        if bytes[input].is_ascii_whitespace() {
            pending_space |= length != 0;
            input += 1;
            continue;
        }
        let (byte, consumed) = if bytes[input..].starts_with(b"&amp;") {
            (b'&', 5)
        } else if bytes[input..].starts_with(b"&quot;") {
            (b'"', 6)
        } else if bytes[input..].starts_with(b"&#39;") {
            (b'\'', 5)
        } else if bytes[input..].starts_with(b"&#x27;") {
            (b'\'', 6)
        } else if bytes[input..].starts_with(b"&lt;") {
            (b'[', 4)
        } else if bytes[input..].starts_with(b"&gt;") {
            (b']', 4)
        } else {
            (bytes[input], 1)
        };
        input += consumed;
        if !byte.is_ascii_graphic() || matches!(byte, b'<' | b'>') {
            pending_space |= length != 0;
            continue;
        }
        if pending_space && length < output.len() {
            output[length] = b' ';
            length += 1;
        }
        pending_space = false;
        if length < output.len() {
            output[length] = byte;
            length += 1;
        }
    }
    length
}

fn parse_selector(selector: &str) -> Option<Selector<'_>> {
    if selector.is_empty()
        || selector.bytes().any(|byte| {
            !(byte.is_ascii_alphanumeric()
                || byte == b'-'
                || byte == b'_'
                || byte == b'.'
                || byte == b'#')
        })
    {
        return None;
    }
    if let Some(class) = selector.strip_prefix('.') {
        (!class.is_empty() && !class.contains('.') && !class.contains('#'))
            .then_some(Selector::Class(class))
    } else if let Some(id) = selector.strip_prefix('#') {
        (!id.is_empty() && !id.contains('.') && !id.contains('#')).then_some(Selector::Id(id))
    } else if !selector.contains('.') && !selector.contains('#') {
        Some(Selector::Tag(selector))
    } else {
        None
    }
}

fn parse_style_value(property: &str, value: &str) -> Option<StyleValue> {
    if eq_ascii(property, "color") {
        parse_color(value).map(StyleValue::Color)
    } else if eq_ascii(property, "background")
        || eq_ascii(property, "background-color")
        || eq_ascii(property, "backgroundColor")
    {
        parse_color(value).map(StyleValue::Background)
    } else if eq_ascii(property, "border") {
        parse_border(value).map(StyleValue::Border)
    } else if eq_ascii(property, "font-size") || eq_ascii(property, "fontSize") {
        parse_positive_px(value, 1, 96).map(StyleValue::FontSize)
    } else if eq_ascii(property, "font-weight") || eq_ascii(property, "fontWeight") {
        parse_font_weight(value).map(StyleValue::FontWeight)
    } else if eq_ascii(property, "display") {
        if eq_ascii(value, "none") {
            Some(StyleValue::Display(DisplayMode::None))
        } else if eq_ascii(value, "block") {
            Some(StyleValue::Display(DisplayMode::Block))
        } else if eq_ascii(value, "inline") || eq_ascii(value, "inline-block") {
            Some(StyleValue::Display(DisplayMode::Inline))
        } else {
            None
        }
    } else if eq_ascii(property, "visibility") {
        if eq_ascii(value, "visible") {
            Some(StyleValue::Visibility(CssVisibility::Visible))
        } else if eq_ascii(value, "hidden") {
            Some(StyleValue::Visibility(CssVisibility::Hidden))
        } else {
            None
        }
    } else if eq_ascii(property, "margin") {
        parse_edges(value, true).map(StyleValue::Margin)
    } else if eq_ascii(property, "padding") {
        parse_edges(value, false).map(StyleValue::Padding)
    } else if eq_ascii(property, "text-align") || eq_ascii(property, "textAlign") {
        if eq_ascii(value, "left") || eq_ascii(value, "start") {
            Some(StyleValue::TextAlign(TextAlign::Left))
        } else if eq_ascii(value, "center") {
            Some(StyleValue::TextAlign(TextAlign::Center))
        } else if eq_ascii(value, "right") || eq_ascii(value, "end") {
            Some(StyleValue::TextAlign(TextAlign::Right))
        } else if eq_ascii(value, "justify") {
            Some(StyleValue::TextAlign(TextAlign::Justify))
        } else {
            None
        }
    } else {
        None
    }
}

fn parse_color(value: &str) -> Option<CssColor> {
    let value = value.trim();
    if let Some(hex) = value.strip_prefix('#') {
        return match hex.len() {
            3 => Some(CssColor::rgb(
                hex_digit(hex.as_bytes()[0])? * 17,
                hex_digit(hex.as_bytes()[1])? * 17,
                hex_digit(hex.as_bytes()[2])? * 17,
            )),
            6 => Some(CssColor::rgb(
                hex_digit(hex.as_bytes()[0])? * 16 + hex_digit(hex.as_bytes()[1])?,
                hex_digit(hex.as_bytes()[2])? * 16 + hex_digit(hex.as_bytes()[3])?,
                hex_digit(hex.as_bytes()[4])? * 16 + hex_digit(hex.as_bytes()[5])?,
            )),
            _ => None,
        };
    }
    if eq_ascii(value, "transparent") {
        Some(CssColor::TRANSPARENT)
    } else if eq_ascii(value, "black") {
        Some(CssColor::BLACK)
    } else if eq_ascii(value, "white") {
        Some(CssColor::WHITE)
    } else if eq_ascii(value, "red") {
        Some(CssColor::RED)
    } else if eq_ascii(value, "green") {
        Some(CssColor::GREEN)
    } else if eq_ascii(value, "blue") {
        Some(CssColor::BLUE)
    } else if eq_ascii(value, "gray") || eq_ascii(value, "grey") {
        Some(CssColor::rgb(128, 128, 128))
    } else if eq_ascii(value, "yellow") {
        Some(CssColor::rgb(255, 255, 0))
    } else if eq_ascii(value, "cyan") {
        Some(CssColor::rgb(0, 255, 255))
    } else if eq_ascii(value, "magenta") {
        Some(CssColor::rgb(255, 0, 255))
    } else {
        None
    }
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn parse_border(value: &str) -> Option<BorderStyle> {
    if eq_ascii(value.trim(), "none") || value.trim() == "0" || value.trim() == "0px" {
        return Some(BorderStyle::NONE);
    }
    let mut width = None;
    let mut color = None;
    let mut solid = false;
    for part in value.split_ascii_whitespace() {
        if eq_ascii(part, "solid") {
            solid = true;
        } else if let Some(parsed) = parse_positive_px(part, 0, 32) {
            width = Some(parsed);
        } else {
            let parsed = parse_color(part)?;
            color = Some(parsed);
        }
    }
    if !solid {
        return None;
    }
    Some(BorderStyle {
        width: width.unwrap_or(1),
        color: color.unwrap_or(CssColor::BLACK),
    })
}

fn parse_positive_px(value: &str, minimum: u16, maximum: u16) -> Option<u16> {
    let value = value.trim();
    let digits = value.strip_suffix("px").unwrap_or(value);
    let parsed = digits.parse::<u16>().ok()?;
    (minimum..=maximum).contains(&parsed).then_some(parsed)
}

fn parse_font_weight(value: &str) -> Option<u16> {
    if eq_ascii(value.trim(), "normal") {
        return Some(400);
    }
    if eq_ascii(value.trim(), "bold") {
        return Some(700);
    }
    let weight = value.trim().parse::<u16>().ok()?;
    ((100..=900).contains(&weight) && weight % 100 == 0).then_some(weight)
}

fn parse_edges(value: &str, allow_negative: bool) -> Option<BoxEdges> {
    let mut values = [0i16; 4];
    let mut count = 0;
    for part in value.split_ascii_whitespace() {
        if count == values.len() {
            return None;
        }
        let digits = part.strip_suffix("px").unwrap_or(part);
        let parsed = digits.parse::<i16>().ok()?;
        if (parsed < 0 && !allow_negative) || !(-256..=512).contains(&parsed) {
            return None;
        }
        values[count] = parsed;
        count += 1;
    }
    match count {
        1 => Some(BoxEdges::all(values[0])),
        2 => Some(BoxEdges {
            top: values[0],
            right: values[1],
            bottom: values[0],
            left: values[1],
        }),
        3 => Some(BoxEdges {
            top: values[0],
            right: values[1],
            bottom: values[2],
            left: values[1],
        }),
        4 => Some(BoxEdges {
            top: values[0],
            right: values[1],
            bottom: values[2],
            left: values[3],
        }),
        _ => None,
    }
}

fn validate_css_text(declarations: &str) -> bool {
    let mut found = false;
    for declaration in declarations.split(';') {
        let declaration = declaration.trim();
        if declaration.is_empty() {
            continue;
        }
        let Some((property, value)) = declaration.split_once(':') else {
            return false;
        };
        if parse_style_value(property.trim(), value.trim()).is_none() {
            return false;
        }
        found = true;
    }
    found
}

fn next_statement<'a>(
    source: &'a str,
    cursor: &mut usize,
) -> Result<Option<&'a str>, ScriptRejection> {
    let bytes = source.as_bytes();
    loop {
        while *cursor < bytes.len()
            && (bytes[*cursor].is_ascii_whitespace() || bytes[*cursor] == b';')
        {
            *cursor += 1;
        }
        if *cursor >= bytes.len() {
            return Ok(None);
        }
        let start = *cursor;
        let mut quote = 0u8;
        let mut escaped = false;
        let mut braces = 0i16;
        let mut parentheses = 0i16;
        while *cursor < bytes.len() {
            let byte = bytes[*cursor];
            if quote != 0 {
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == quote {
                    quote = 0;
                }
                *cursor += 1;
                continue;
            }
            match byte {
                b'\'' | b'"' => quote = byte,
                b'{' => braces += 1,
                b'}' => {
                    braces -= 1;
                    if braces < 0 {
                        return Err(ScriptRejection::UnsupportedSyntax);
                    }
                }
                b'(' => parentheses += 1,
                b')' => {
                    parentheses -= 1;
                    if parentheses < 0 {
                        return Err(ScriptRejection::UnsupportedSyntax);
                    }
                }
                b';' if braces == 0 && parentheses == 0 => {
                    let statement = source[start..*cursor].trim();
                    *cursor += 1;
                    if statement.is_empty() {
                        break;
                    }
                    return Ok(Some(statement));
                }
                _ => {}
            }
            *cursor += 1;
        }
        if quote != 0 || braces != 0 || parentheses != 0 {
            return Err(ScriptRejection::UnsupportedSyntax);
        }
        let statement = source[start..*cursor].trim();
        if !statement.is_empty() {
            return Ok(Some(statement));
        }
    }
}

fn split_assignment(statement: &str) -> Option<(&str, &str)> {
    let bytes = statement.as_bytes();
    let mut quote = 0u8;
    let mut escaped = false;
    let mut braces = 0i16;
    let mut parentheses = 0i16;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if quote != 0 {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                quote = 0;
            }
            continue;
        }
        match byte {
            b'\'' | b'"' => quote = byte,
            b'{' => braces += 1,
            b'}' => braces -= 1,
            b'(' => parentheses += 1,
            b')' => parentheses -= 1,
            b'=' if braces == 0 && parentheses == 0 => {
                if bytes.get(index + 1) == Some(&b'>') {
                    continue;
                }
                return Some((&statement[..index], &statement[index + 1..]));
            }
            _ => {}
        }
    }
    None
}

fn call_arguments<'a>(statement: &'a str, name: &str) -> Option<&'a str> {
    let rest = strip_prefix_ascii(statement.trim(), name)?;
    let rest = rest.trim_start();
    let inner = rest.strip_prefix('(')?.strip_suffix(')')?;
    Some(inner)
}

fn parse_arguments<const N: usize>(source: &str) -> Result<[&str; N], ScriptRejection> {
    let mut arguments = [""; N];
    let mut cursor = 0;
    for (index, argument) in arguments.iter_mut().enumerate() {
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        let (value, consumed) = parse_quoted(&source[cursor..])?;
        *argument = value;
        cursor += consumed;
        while source
            .as_bytes()
            .get(cursor)
            .is_some_and(u8::is_ascii_whitespace)
        {
            cursor += 1;
        }
        if index + 1 < N {
            if source.as_bytes().get(cursor) != Some(&b',') {
                return Err(ScriptRejection::UnsupportedSyntax);
            }
            cursor += 1;
        }
    }
    if !source[cursor..].trim().is_empty() {
        return Err(ScriptRejection::UnsupportedSyntax);
    }
    Ok(arguments)
}

fn parse_quoted(source: &str) -> Result<(&str, usize), ScriptRejection> {
    let bytes = source.as_bytes();
    let quote = bytes
        .first()
        .copied()
        .ok_or(ScriptRejection::UnsupportedSyntax)?;
    if quote != b'\'' && quote != b'"' {
        return Err(ScriptRejection::UnsupportedSyntax);
    }
    let mut cursor = 1;
    while cursor < bytes.len() {
        if bytes[cursor] == b'\\' {
            // Escaped strings would require allocating an unescaped result.
            return Err(ScriptRejection::UnsupportedSyntax);
        }
        if bytes[cursor] == quote {
            return Ok((&source[1..cursor], cursor + 1));
        }
        cursor += 1;
    }
    Err(ScriptRejection::UnsupportedSyntax)
}

fn parse_quoted_value(source: &str) -> Result<&str, ScriptRejection> {
    let source = source.trim();
    let (value, consumed) = parse_quoted(source)?;
    if !source[consumed..].trim().is_empty() {
        return Err(ScriptRejection::UnsupportedSyntax);
    }
    Ok(value)
}

fn parse_handler_body(source: &str) -> Result<&str, ScriptRejection> {
    let source = source.trim();
    let braces = if strip_prefix_ascii(source, "function").is_some() || source.contains("=>") {
        source.find('{').map(|open| (open, source.rfind('}')))
    } else {
        None
    };
    let Some((open, Some(close))) = braces else {
        return Err(ScriptRejection::UnsupportedSyntax);
    };
    if close <= open || !source[close + 1..].trim().is_empty() {
        return Err(ScriptRejection::UnsupportedSyntax);
    }
    Ok(source[open + 1..close].trim())
}

fn parse_add_event_listener(tail: &str) -> Result<Option<&str>, ScriptRejection> {
    let Some(arguments) = strip_prefix_ascii(tail.trim(), ".addEventListener") else {
        return Ok(None);
    };
    let arguments = arguments
        .trim_start()
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .ok_or(ScriptRejection::UnsupportedSyntax)?;
    let trimmed = arguments.trim_start();
    let (event, consumed) = parse_quoted(trimmed)?;
    if !eq_ascii(event, "click") {
        return Err(ScriptRejection::UnsupportedSyntax);
    }
    let rest = trimmed[consumed..].trim_start();
    let rest = rest
        .strip_prefix(',')
        .ok_or(ScriptRejection::UnsupportedSyntax)?;
    Ok(Some(parse_handler_body(rest)?))
}

fn contains_forbidden_api(source: &str) -> bool {
    const FORBIDDEN: [&str; 15] = [
        "fetch(",
        "xmlhttprequest",
        "websocket",
        "eval(",
        "new function",
        "localstorage",
        "sessionstorage",
        "indexeddb",
        "document.cookie",
        "navigator.",
        "window.",
        "globalthis",
        "import(",
        "require(",
        "javascript:",
    ];
    let bytes = source.as_bytes();
    let mut quote = 0u8;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if quote != 0 {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                quote = 0;
            }
            index += 1;
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = byte;
            index += 1;
            continue;
        }
        if FORBIDDEN.iter().any(|pattern| {
            source
                .get(index..index + pattern.len())
                .is_some_and(|candidate| eq_ascii(candidate, pattern))
        }) {
            return true;
        }
        index += 1;
    }
    false
}

fn strip_prefix_ascii<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let candidate = value.get(..prefix.len())?;
    eq_ascii(candidate, prefix).then_some(&value[prefix.len()..])
}

fn eq_ascii(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_safe_local_document_and_links() {
        let document = Document::parse(
            "expos://home",
            "<title>Exp Home</title><h1>Welcome</h1><p>Forms make the environment.</p><a href='expos://about'>About</a>",
        )
        .unwrap();
        assert_eq!(document.url(), "expos://home");
        assert_eq!(document.title(), "Exp Home");
        assert_eq!(document.len(), 4);
        let link = document.nodes().last().unwrap();
        assert_eq!(link.kind, NodeKind::Link);
        assert_eq!(link.target.as_str(), "expos://about");
        assert_eq!(
            document
                .nodes()
                .nth(2)
                .and_then(|_| document.styled_node(2))
                .unwrap()
                .style
                .color,
            CssColor::rgb(217, 221, 217)
        );
    }

    #[test]
    fn accepts_http_and_https_as_document_identifiers_without_fetching() {
        let http = Document::parse(
            "http://example.com/",
            "<title>Example</title><h1>Example Domain</h1><p>Provided source.</p>",
        )
        .unwrap();
        assert_eq!(http.url(), "http://example.com/");
        let https = Document::parse(
            "https://example.com/",
            "<title>Secure</title><p>Provided source.</p>",
        )
        .unwrap();
        assert_eq!(https.url(), "https://example.com/");
    }

    #[test]
    fn css_cascades_by_tag_class_id_and_inline_per_property() {
        let document = Document::parse(
            "expos://styles",
            "<style>
                p { color: red; margin: 1px 2px; font-size: 14px; }
                .notice { color: blue; padding: 3px 4px 5px; font-weight: bold; }
                #hero { color: #123456; border: 2px solid #abcdef; text-align: center; visibility: hidden; }
             </style>
             <p id='hero' class='notice wide' style='color: white; display: block; background: #010203'>Hello</p>",
        )
        .unwrap();
        let hero = document.element_by_id("hero").unwrap();
        assert_eq!(hero.tag, "p");
        assert_eq!(hero.id, "hero");
        assert_eq!(hero.style.color, CssColor::WHITE);
        assert_eq!(hero.style.background, CssColor::rgb(1, 2, 3));
        assert_eq!(hero.style.font_size, 14);
        assert_eq!(hero.style.font_weight, 700);
        assert_eq!(hero.style.border.width, 2);
        assert_eq!(hero.style.border.color, CssColor::rgb(0xab, 0xcd, 0xef));
        assert_eq!(
            hero.style.margin,
            BoxEdges {
                top: 1,
                right: 2,
                bottom: 1,
                left: 2
            }
        );
        assert_eq!(
            hero.style.padding,
            BoxEdges {
                top: 3,
                right: 4,
                bottom: 5,
                left: 4
            }
        );
        assert_eq!(hero.style.text_align, TextAlign::Center);
        assert_eq!(hero.style.visibility, CssVisibility::Hidden);
    }

    #[test]
    fn class_matching_uses_tokens_and_equal_specificity_uses_source_order() {
        let document = Document::parse(
            "expos://classes",
            "<style>.note { color: red } .notebook { color: green } .note { color: blue }</style><p id='one' class='note'>One</p><p id='two' class='notebook'>Two</p>",
        )
        .unwrap();
        assert_eq!(document.style_of("one").unwrap().color, CssColor::BLUE);
        assert_eq!(document.style_of("two").unwrap().color, CssColor::GREEN);
    }

    #[test]
    fn embedded_script_updates_title_text_and_style_with_exact_stats() {
        let document = Document::parse(
            "expos://script",
            "<title>Before</title><p id='status'>Waiting</p><p id='extra'>Visible</p>
             <script>
               document.title = 'After';
               document.getElementById('status').textContent = 'Ready';
               document.getElementById('status').style.cssText = 'color: #112233; fontWeight: 700; padding: 2px';
               hide('extra');
             </script>",
        )
        .unwrap();
        assert_eq!(document.title(), "After");
        assert_eq!(
            document.element_by_id("status").unwrap().node.text.as_str(),
            "Ready"
        );
        let status = document.style_of("status").unwrap();
        assert_eq!(status.color, CssColor::rgb(0x11, 0x22, 0x33));
        assert_eq!(status.font_weight, 700);
        assert_eq!(status.padding, BoxEdges::all(2));
        assert_eq!(
            document.style_of("extra").unwrap().display,
            DisplayMode::None
        );
        assert_eq!(
            document.script_report(),
            ScriptReport {
                scripts_seen: 1,
                scripts_executed: 1,
                scripts_rejected: 0,
                statements_executed: 4,
                handlers_registered: 0,
                clicks_dispatched: 0,
                limit_hits: 0,
                last_rejection: None,
            }
        );
    }

    #[test]
    fn automatic_scripts_are_counted_once_including_rejections() {
        let document = Document::parse(
            "expos://stats",
            "<title>Zero</title><p id='status'>Waiting</p>
             <script>title('One')</script>
             <script>fetch('https://example.com')</script>
             <script>text('status', 'Ready')</script>",
        )
        .unwrap();
        let report = document.script_report();
        assert_eq!(report.scripts_seen, 3);
        assert_eq!(report.scripts_executed, 2);
        assert_eq!(report.scripts_rejected, 1);
        assert_eq!(report.statements_executed, 2);
        assert_eq!(report.last_rejection, Some(ScriptRejection::ForbiddenApi));
        assert_eq!(document.title(), "One");
        assert_eq!(
            document.element_by_id("status").unwrap().node.text.as_str(),
            "Ready"
        );
    }

    #[test]
    fn click_handlers_dispatch_by_id_or_stable_node_index() {
        let mut document = Document::parse(
            "expos://events",
            "<button id='action' onclick=\"this.textContent = 'Clicked'; this.style.backgroundColor = '#334455'; hide('victim')\">Go</button><p id='victim'>Visible</p>",
        )
        .unwrap();
        let action = document.element_by_id("action").unwrap();
        assert!(action.clickable);
        let index = action.index;
        assert!(document.dispatch_click_at_node(index));
        assert_eq!(
            document.styled_node(index).unwrap().node.text.as_str(),
            "Clicked"
        );
        assert_eq!(
            document.styled_node(index).unwrap().style.background,
            CssColor::rgb(0x33, 0x44, 0x55)
        );
        assert_eq!(
            document.style_of("victim").unwrap().display,
            DisplayMode::None
        );
        assert!(!document.dispatch_click("missing"));
        let report = document.script_report();
        assert_eq!(report.scripts_seen, 1);
        assert_eq!(report.scripts_executed, 1);
        assert_eq!(report.scripts_rejected, 0);
        assert_eq!(report.handlers_registered, 1);
        assert_eq!(report.clicks_dispatched, 1);
        assert_eq!(report.statements_executed, 3);
    }

    #[test]
    fn script_can_install_a_click_handler_and_show_restores_display() {
        let mut document = Document::parse(
            "expos://handler",
            "<button id='toggle'>Toggle</button><span id='target'>Target</span>",
        )
        .unwrap();
        document
            .execute_script(
                "document.getElementById('toggle').onclick = function() { hide('target'); this.textContent = 'Done'; }",
            )
            .unwrap();
        assert!(document.dispatch_click("toggle"));
        assert_eq!(
            document.style_of("target").unwrap().display,
            DisplayMode::None
        );
        document.execute_script("show('target')").unwrap();
        assert_eq!(
            document.style_of("target").unwrap().display,
            DisplayMode::Inline
        );
    }

    #[test]
    fn forbidden_or_invalid_script_is_atomic_and_reported() {
        let mut document = Document::parse(
            "expos://safe",
            "<title>Safe</title><p id='status'>Unchanged</p>",
        )
        .unwrap();
        assert_eq!(
            document.execute_script("text('status', 'Changed'); localStorage.setItem('x', 'y')"),
            Err(ScriptRejection::ForbiddenApi)
        );
        assert_eq!(
            document.element_by_id("status").unwrap().node.text.as_str(),
            "Unchanged"
        );
        assert_eq!(
            document.execute_script("text('status', 'Changed'); launchProcess('shell')"),
            Err(ScriptRejection::UnsupportedSyntax)
        );
        assert_eq!(
            document.element_by_id("status").unwrap().node.text.as_str(),
            "Unchanged"
        );
        let report = document.script_report();
        assert_eq!(report.scripts_seen, 2);
        assert_eq!(report.scripts_executed, 0);
        assert_eq!(report.scripts_rejected, 2);
    }

    #[test]
    fn external_script_sources_are_never_loaded() {
        let document = Document::parse(
            "expos://external",
            "<title>Safe</title><script src='https://example.com/code.js'></script>",
        )
        .unwrap();
        let report = document.script_report();
        assert_eq!(report.scripts_seen, 1);
        assert_eq!(report.scripts_executed, 0);
        assert_eq!(report.scripts_rejected, 1);
        assert_eq!(report.last_rejection, Some(ScriptRejection::ForbiddenApi));
    }

    #[test]
    fn integration_fixture_exercises_css_script_and_click_paths() {
        let mut document = Document::parse(
            "http://10.0.2.2:18080/browser-fixture.html",
            include_str!("../../../tests/browser-fixture.html"),
        )
        .unwrap();
        assert_eq!(document.len(), 4);
        assert_eq!(document.style_rule_count(), 5);
        assert_eq!(document.title(), "ExpOS Fixture Ready");
        assert_eq!(
            document.element_by_id("status").unwrap().node.text.as_str(),
            "CSS and JavaScript are active"
        );
        assert_eq!(
            document.style_of("status").unwrap().color,
            CssColor::rgb(0x6f, 0xb8, 0x73)
        );
        let report = document.script_report();
        assert_eq!(report.scripts_seen, 1);
        assert_eq!(report.scripts_executed, 1);
        assert_eq!(report.scripts_rejected, 0);
        assert_eq!(report.handlers_registered, 1);
        assert!(document.dispatch_click("action"));
        assert_eq!(
            document.element_by_id("status").unwrap().node.text.as_str(),
            "Clicked in ExpOS"
        );
    }

    #[test]
    fn duckduckgo_projection_keeps_only_bounded_result_titles_and_links() {
        let source = r#"
            <html><script>fetch('https://tracker.invalid')</script>
            <a rel="nofollow" class="result__a other" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs&amp;rut=abc">Example <b>Result</b> &amp; docs</a>
            <a class="result__a" href="https://example.org/second">Second result</a>
            <a class="result__a" href="https://example.org/third">Third result</a>
            <a class="result__a" href="https://example.org/fourth">Fourth result</a>
            <a class="result__a" href="https://example.org/fifth">Fifth result</a>
            <a class="result__a" href="https://example.org/sixth">Sixth result</a>
            <a class="result__a" href="https://example.org/seventh">Seventh result</a>
            <a class="result__a" href="https://example.org/eighth">Eighth result</a>
            <a class="result__a" href="https://example.org/ninth">Must be truncated</a>
            <img src="https://tracker.invalid/pixel">
            </html>
        "#;
        let projected = Document::parse_duckduckgo_results(
            "https://html.duckduckgo.com/html/?q=example",
            source,
        )
        .unwrap();
        assert_eq!(projected.result_count as usize, MAX_SEARCH_RESULTS);
        assert_eq!(projected.document.len(), MAX_SEARCH_RESULTS + 2);
        let first = projected.document.styled_node(2).unwrap();
        assert_eq!(first.node.text.as_str(), "Example Result & docs");
        assert_eq!(
            first.node.target.as_str(),
            "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs&rut=abc"
        );
        assert_eq!(first.style.display, DisplayMode::Block);
        assert_eq!(first.style.background, CssColor::rgb(21, 28, 32));
        let last = projected
            .document
            .styled_node(MAX_SEARCH_RESULTS + 1)
            .unwrap();
        assert_eq!(last.node.text.as_str(), "Eighth result");
        assert_eq!(last.node.target.as_str(), "https://example.org/eighth");
        assert_eq!(projected.document.script_report(), ScriptReport::EMPTY);
        assert!(matches!(
            Document::parse_duckduckgo_results("https://example.com/html/?q=x", source),
            Err(BrowserError::InvalidUrl)
        ));
    }

    #[test]
    fn bounded_inputs_fail_without_overflowing_storage() {
        let oversized = "x".repeat(MAX_BROWSER_DOCUMENT_BYTES + 1);
        assert!(matches!(
            Document::parse("expos://large", &oversized),
            Err(BrowserError::DocumentTooLarge)
        ));

        let mut source = std::string::String::from("<title>Capacity</title>");
        for _ in 0..MAX_BROWSER_NODES {
            source.push_str("<p>node</p>");
        }
        assert!(matches!(
            Document::parse("expos://capacity", &source),
            Err(BrowserError::TooManyNodes)
        ));
    }

    #[test]
    fn script_and_stylesheet_limits_are_enforced_and_reported() {
        let mut document =
            Document::parse("expos://limits", "<title id='title'>Limits</title>").unwrap();
        let too_long = "x".repeat(MAX_SCRIPT_BYTES + 1);
        assert_eq!(
            document.execute_script(&too_long),
            Err(ScriptRejection::ScriptTooLong)
        );

        let too_many_statements = "title('ok');".repeat(MAX_SCRIPT_STATEMENTS + 1);
        assert_eq!(
            document.execute_script(&too_many_statements),
            Err(ScriptRejection::TooManyStatements)
        );
        assert_eq!(document.script_report().limit_hits, 2);
        assert_eq!(document.title(), "Limits");

        let mut fresh = Document::parse("expos://script-limit", "<title>Limits</title>").unwrap();
        for _ in 0..MAX_BROWSER_SCRIPTS {
            fresh.execute_script("title('ok')").unwrap();
        }
        assert_eq!(
            fresh.execute_script("title('blocked')"),
            Err(ScriptRejection::ScriptLimit)
        );
        assert_eq!(fresh.script_report().scripts_seen, 17);
        assert_eq!(fresh.script_report().scripts_executed, 16);
        assert_eq!(fresh.script_report().scripts_rejected, 1);
        assert_eq!(fresh.title(), "ok");

        let mut css = std::string::String::from("<style>");
        for _ in 0..=MAX_STYLE_RULES {
            css.push_str("p { color: red }");
        }
        css.push_str("</style><p>node</p>");
        assert!(matches!(
            Document::parse("expos://style-limit", &css),
            Err(BrowserError::TooManyStyleRules)
        ));
    }
}
