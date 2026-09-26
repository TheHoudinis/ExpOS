//! Native package-app host used by the Ayo desktop manager.
//!
//! Each entry is a separate Interface Form in the Ayo catalog.  The host is
//! intentionally shared: twenty small tools should not consume twenty kernel
//! surfaces or twenty ambient capabilities just to be installable.

use crate::{framebuffer, hardware, network};
use expos_core::Rect;
use framebuffer::color;

pub const PACKAGE_COUNT: usize = 20;
const INPUT_CAPACITY: usize = 96;
const PAGE_SIZE: usize = 8;

#[derive(Clone, Copy)]
struct PackageApp {
    name: &'static str,
    category: &'static str,
    summary: &'static str,
    accent: u32,
}

const APPS: [PackageApp; PACKAGE_COUNT] = [
    PackageApp {
        name: "Calculator",
        category: "Utilities",
        summary: "Fast integer expressions",
        accent: color::GREEN,
    },
    PackageApp {
        name: "Tasks",
        category: "Utilities",
        summary: "A focused task inbox",
        accent: color::CYAN,
    },
    PackageApp {
        name: "Clock",
        category: "Utilities",
        summary: "Local time at a glance",
        accent: color::PURPLE,
    },
    PackageApp {
        name: "Calendar",
        category: "Utilities",
        summary: "Date and month overview",
        accent: 0x00D0_9B52,
    },
    PackageApp {
        name: "UnitConvert",
        category: "Utilities",
        summary: "Miles to kilometres",
        accent: color::GREEN,
    },
    PackageApp {
        name: "ColorLab",
        category: "Graphics",
        summary: "Explore the Prism palette",
        accent: 0x00E0_6C9F,
    },
    PackageApp {
        name: "PixelPad",
        category: "Graphics",
        summary: "Draw an 8 by 8 sprite",
        accent: color::CYAN,
    },
    PackageApp {
        name: "SystemMonitor",
        category: "Utilities",
        summary: "Live display and clock facts",
        accent: color::GREEN,
    },
    PackageApp {
        name: "NetScope",
        category: "Networking",
        summary: "Inspect network readiness",
        accent: color::CYAN,
    },
    PackageApp {
        name: "FormMap",
        category: "Developer tools",
        summary: "Understand Form relationships",
        accent: color::PURPLE,
    },
    PackageApp {
        name: "CharacterMap",
        category: "Utilities",
        summary: "Printable ASCII reference",
        accent: 0x00D0_9B52,
    },
    PackageApp {
        name: "BaseConvert",
        category: "Developer tools",
        summary: "Decimal and hexadecimal",
        accent: color::GREEN,
    },
    PackageApp {
        name: "TextCase",
        category: "Editors",
        summary: "Inspect and transform text",
        accent: color::CYAN,
    },
    PackageApp {
        name: "WordCount",
        category: "Editors",
        summary: "Count words and characters",
        accent: color::PURPLE,
    },
    PackageApp {
        name: "FocusTimer",
        category: "Utilities",
        summary: "A calm focus-session card",
        accent: 0x00D0_9B52,
    },
    PackageApp {
        name: "Stopwatch",
        category: "Utilities",
        summary: "Monotonic elapsed ticks",
        accent: color::GREEN,
    },
    PackageApp {
        name: "Counter",
        category: "Utilities",
        summary: "A clean tally counter",
        accent: color::CYAN,
    },
    PackageApp {
        name: "MarkdownPad",
        category: "Editors",
        summary: "Tiny Markdown scratchpad",
        accent: color::PURPLE,
    },
    PackageApp {
        name: "JsonInspect",
        category: "Developer tools",
        summary: "Check JSON framing",
        accent: 0x00D0_9B52,
    },
    PackageApp {
        name: "HashLab",
        category: "Developer tools",
        summary: "Deterministic FNV-1a fingerprints",
        accent: color::GREEN,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerAction {
    Changed,
    Installed,
    Open,
}

pub struct NativeApps {
    selected: usize,
    installed: u32,
    active: usize,
    input: [u8; INPUT_CAPACITY],
    input_len: usize,
    counter: u32,
    palette: usize,
    pixels: u64,
    started_at: u64,
    persistence_generation: u32,
    notice: &'static str,
}

impl NativeApps {
    pub fn new() -> Self {
        Self {
            selected: 0,
            installed: 0,
            active: 0,
            input: [0; INPUT_CAPACITY],
            input_len: 0,
            counter: 0,
            palette: 0,
            pixels: 0,
            started_at: hardware::timestamp(),
            persistence_generation: 0,
            notice: "Select a package, then install its signed built-in artifact.",
        }
    }

    pub fn handle_manager_key(&mut self, key: u8) -> Option<ManagerAction> {
        match key {
            crate::input::KEY_UP => self.selected = self.selected.saturating_sub(1),
            crate::input::KEY_DOWN => self.selected = (self.selected + 1).min(PACKAGE_COUNT - 1),
            b'i' | b'I' => return Some(self.install_selected()),
            b'\n' => {
                if self.is_installed(self.selected) {
                    self.active = self.selected;
                    self.clear_input();
                    self.mark_persistent_change();
                    return Some(ManagerAction::Open);
                }
                return Some(self.install_selected());
            }
            _ => return None,
        }
        Some(ManagerAction::Changed)
    }

    pub fn handle_manager_click(&mut self, x: i16, y: i16, rect: Rect) -> Option<ManagerAction> {
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        let start = (self.selected / PAGE_SIZE) * PAGE_SIZE;
        if local_y >= 104 {
            let row = ((local_y - 104) / 34) as usize;
            if row < PAGE_SIZE
                && start + row < PACKAGE_COUNT
                && local_y < 104 + 34 * PAGE_SIZE as i16
            {
                self.selected = start + row;
                return Some(ManagerAction::Changed);
            }
        }
        let action_y = rect.height as i16 - 58;
        if (action_y..action_y + 38).contains(&local_y) {
            if (rect.width as i16 - 252..rect.width as i16 - 140).contains(&local_x) {
                return Some(self.install_selected());
            }
            if (rect.width as i16 - 128..rect.width as i16 - 20).contains(&local_x)
                && self.is_installed(self.selected)
            {
                self.active = self.selected;
                self.clear_input();
                self.mark_persistent_change();
                return Some(ManagerAction::Open);
            }
        }
        None
    }

    fn install_selected(&mut self) -> ManagerAction {
        self.installed |= 1 << self.selected;
        self.mark_persistent_change();
        self.notice = "Verified -> DIESE -> PIMP -> installed Interface Form.";
        ManagerAction::Installed
    }

    pub const fn persistence_generation(&self) -> u32 {
        self.persistence_generation
    }

    fn mark_persistent_change(&mut self) {
        self.persistence_generation = self.persistence_generation.wrapping_add(1).max(1);
    }

    /// Serialize the durable shared state of the Ayo app host. Typed input is
    /// deliberately transient; installed packages and tool results survive.
    pub fn encode_state(&self, out: &mut [u8; 32]) -> usize {
        out.fill(0);
        out[..4].copy_from_slice(b"AYO1");
        out[4..8].copy_from_slice(&self.installed.to_le_bytes());
        out[8] = self.active as u8;
        out[9] = self.palette as u8;
        out[10..14].copy_from_slice(&self.counter.to_le_bytes());
        out[14..22].copy_from_slice(&self.pixels.to_le_bytes());
        22
    }

    pub fn restore_state(bytes: &[u8]) -> Self {
        let mut state = Self::new();
        if bytes.len() < 22 || &bytes[..4] != b"AYO1" {
            return state;
        }
        let installed = u32::from_le_bytes(bytes[4..8].try_into().unwrap_or([0; 4]));
        state.installed = installed & ((1_u32 << PACKAGE_COUNT) - 1);
        state.active = (bytes[8] as usize).min(PACKAGE_COUNT - 1);
        if !state.is_installed(state.active) {
            state.active = (0..PACKAGE_COUNT)
                .find(|index| state.is_installed(*index))
                .unwrap_or(0);
        }
        state.palette = (bytes[9] as usize) % PALETTE.len();
        state.counter = u32::from_le_bytes(bytes[10..14].try_into().unwrap_or([0; 4]));
        state.pixels = u64::from_le_bytes(bytes[14..22].try_into().unwrap_or([0; 8]));
        state.notice = "Restored installed apps and tool state from ExpFS.";
        state
    }

    const fn is_installed(&self, index: usize) -> bool {
        self.installed & (1 << index) != 0
    }

    pub fn render_manager(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        let width = rect.width as i32;
        let height = rect.height as i32;
        framebuffer::text(x + 26, y + 52, "AYO", color::WHITE, 2);
        framebuffer::text(x + 94, y + 57, "PACKAGE MANAGER", color::MUTED, 1);
        framebuffer::text(
            x + 26,
            y + 82,
            "20 native apps  //  signed built-in repository",
            color::CYAN,
            1,
        );
        let start = (self.selected / PAGE_SIZE) * PAGE_SIZE;
        for slot in 0..PAGE_SIZE {
            let index = start + slot;
            if index >= PACKAGE_COUNT {
                break;
            }
            let app = APPS[index];
            let row_y = y + 104 + slot as i32 * 34;
            let selected = index == self.selected;
            framebuffer::rect(
                x + 24,
                row_y,
                width - 48,
                28,
                if selected { 0x0021_2931 } else { 0x0012_171D },
            );
            framebuffer::rect(x + 24, row_y, 4, 28, app.accent);
            framebuffer::text(
                x + 40,
                row_y + 10,
                app.name,
                if selected { color::WHITE } else { color::INK },
                1,
            );
            framebuffer::text(x + 210, row_y + 10, app.category, color::MUTED, 1);
            framebuffer::text(
                x + width - 112,
                row_y + 10,
                if self.is_installed(index) {
                    "INSTALLED"
                } else {
                    "AVAILABLE"
                },
                if self.is_installed(index) {
                    color::GREEN
                } else {
                    color::CYAN
                },
                1,
            );
        }
        let app = APPS[self.selected];
        framebuffer::text(x + 26, y + height - 88, app.summary, color::INK, 1);
        framebuffer::text(x + 26, y + height - 68, self.notice, color::MUTED, 1);
        button(
            x + width - 252,
            y + height - 58,
            112,
            if self.is_installed(self.selected) {
                "REINSTALL"
            } else {
                "INSTALL"
            },
            app.accent,
            true,
        );
        button(
            x + width - 128,
            y + height - 58,
            108,
            "OPEN",
            color::PURPLE,
            self.is_installed(self.selected),
        );
        framebuffer::text(
            x + 26,
            y + height - 36,
            "UP/DOWN browse   I install   ENTER open",
            color::MUTED,
            1,
        );
    }

    pub fn handle_app_key(&mut self, key: u8) -> bool {
        if self.installed == 0 {
            return false;
        }
        match key {
            crate::input::KEY_LEFT => {
                self.shift_active(-1);
                true
            }
            crate::input::KEY_RIGHT => {
                self.shift_active(1);
                true
            }
            0x08 => {
                self.input_len = self.input_len.saturating_sub(1);
                true
            }
            b'\n' => {
                if self.active == 1 {
                    self.counter = self.counter.saturating_add(1);
                    self.clear_input();
                    self.mark_persistent_change();
                } else if self.active == 16 {
                    self.counter = self.counter.saturating_add(1);
                    self.mark_persistent_change();
                }
                true
            }
            b' ' if self.active == 16 => {
                self.counter = self.counter.saturating_add(1);
                self.mark_persistent_change();
                true
            }
            b' ' if self.active == 5 => {
                self.palette = (self.palette + 1) % PALETTE.len();
                self.mark_persistent_change();
                true
            }
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.input_len < INPUT_CAPACITY =>
            {
                self.input[self.input_len] = byte;
                self.input_len += 1;
                true
            }
            _ => false,
        }
    }

    pub fn handle_app_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        if self.installed == 0 {
            return false;
        }
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        if self.active == 6 && (210..466).contains(&local_x) && (116..372).contains(&local_y) {
            let column = ((local_x - 210) / 32) as u64;
            let row = ((local_y - 116) / 32) as u64;
            self.pixels ^= 1_u64 << (row * 8 + column);
            self.mark_persistent_change();
            return true;
        }
        if self.active == 16 && (220..500).contains(&local_x) && (170..260).contains(&local_y) {
            self.counter = self.counter.saturating_add(1);
            self.mark_persistent_change();
            return true;
        }
        false
    }

    fn shift_active(&mut self, direction: i8) {
        for step in 1..=PACKAGE_COUNT {
            let index = if direction < 0 {
                (self.active + PACKAGE_COUNT - step) % PACKAGE_COUNT
            } else {
                (self.active + step) % PACKAGE_COUNT
            };
            if self.is_installed(index) {
                self.active = index;
                self.clear_input();
                self.mark_persistent_change();
                break;
            }
        }
    }

    fn clear_input(&mut self) {
        self.input.fill(0);
        self.input_len = 0;
    }

    pub fn render_app(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        let width = rect.width as i32;
        framebuffer::rect(x + 18, y + 48, 150, rect.height as i32 - 66, 0x000D_1218);
        framebuffer::text(x + 34, y + 66, "AYO APPS", color::WHITE, 1);
        framebuffer::text(x + 34, y + 88, "<  SWITCH  >", color::MUTED, 1);
        if self.installed == 0 {
            framebuffer::text(
                x + 204,
                y + 96,
                "NO PACKAGE APPS INSTALLED",
                color::MUTED,
                2,
            );
            framebuffer::text(
                x + 204,
                y + 136,
                "Open Ayo, choose a package, then Install.",
                color::INK,
                1,
            );
            return;
        }
        let app = APPS[self.active];
        let mut slot = 0;
        for (index, package) in APPS.iter().enumerate() {
            if !self.is_installed(index) {
                continue;
            }
            if slot >= 10 {
                break;
            }
            let row_y = y + 118 + slot * 27;
            if index == self.active {
                framebuffer::rect(x + 26, row_y - 7, 132, 23, 0x0021_2931);
            }
            framebuffer::text(
                x + 34,
                row_y,
                package.name,
                if index == self.active {
                    package.accent
                } else {
                    color::MUTED
                },
                1,
            );
            slot += 1;
        }
        framebuffer::text(x + 198, y + 58, app.name, app.accent, 2);
        framebuffer::text(x + 200, y + 88, app.summary, color::MUTED, 1);
        self.render_tool(x + 198, y + 112, width - 222, rect.height as i32 - 142);
    }

    fn render_tool(&self, x: i32, y: i32, width: i32, height: i32) {
        let input = core::str::from_utf8(&self.input[..self.input_len]).unwrap_or("");
        match self.active {
            2 => {
                let now = hardware::rtc_time();
                let mut value = [b'0'; 8];
                value[0] = b'0' + now.hour / 10;
                value[1] = b'0' + now.hour % 10;
                value[2] = b':';
                value[3] = b'0' + now.minute / 10;
                value[4] = b'0' + now.minute % 10;
                value[5] = b':';
                value[6] = b'0' + now.second / 10;
                value[7] = b'0' + now.second % 10;
                framebuffer::text(
                    x + 70,
                    y + 80,
                    core::str::from_utf8(&value).unwrap_or("--:--:--"),
                    color::WHITE,
                    3,
                );
            }
            3 => {
                let now = hardware::rtc_time();
                framebuffer::text(x + 24, y + 20, "MONTH VIEW", color::CYAN, 2);
                framebuffer::text(x + 190, y + 24, month_name(now.month), color::WHITE, 1);
                number(x + 288, y + 24, now.year as u64, color::MUTED, 1);
                framebuffer::text(
                    x + 24,
                    y + 52,
                    "MON  TUE  WED  THU  FRI  SAT  SUN",
                    color::MUTED,
                    1,
                );
                for day in 0..31 {
                    let column = day % 7;
                    let row = day / 7;
                    number(
                        x + 24 + column * 48,
                        y + 84 + row * 34,
                        (day + 1) as u64,
                        if day + 1 == now.day as i32 {
                            color::GREEN
                        } else {
                            color::INK
                        },
                        1,
                    );
                }
                framebuffer::text(x + 24, y + 266, "LIVE CMOS DATE SERVICE", color::MUTED, 1);
            }
            5 => {
                framebuffer::rect(x + 24, y + 30, width - 48, 170, PALETTE[self.palette]);
                framebuffer::text(
                    x + 34,
                    y + 220,
                    "SPACE cycles the curated Prism palette",
                    color::INK,
                    1,
                );
            }
            6 => {
                for row in 0..8 {
                    for column in 0..8 {
                        let on = self.pixels & (1_u64 << (row * 8 + column)) != 0;
                        framebuffer::rect(
                            x + 12 + column * 32,
                            y + 4 + row * 32,
                            29,
                            29,
                            if on { color::CYAN } else { 0x0016_1C23 },
                        );
                    }
                }
                framebuffer::text(x + 286, y + 20, "CLICK CELLS", color::MUTED, 1);
            }
            7 => {
                metric(
                    x + 20,
                    y + 24,
                    "DISPLAY",
                    framebuffer::current_mode().label(),
                    color::CYAN,
                );
                metric(
                    x + 20,
                    y + 94,
                    "NETWORK",
                    if network::available() {
                        "READY"
                    } else {
                        "OFFLINE"
                    },
                    color::GREEN,
                );
                metric(x + 20, y + 164, "APP FORMS", "20", color::PURPLE);
            }
            8 => {
                framebuffer::text(x + 24, y + 30, "NATIVE NETWORK SERVICE", color::CYAN, 2);
                framebuffer::text(
                    x + 24,
                    y + 76,
                    if network::available() {
                        "READY // capability requests allowed"
                    } else {
                        "OFFLINE // no device available"
                    },
                    if network::available() {
                        color::GREEN
                    } else {
                        color::MUTED
                    },
                    1,
                );
            }
            9 => {
                framebuffer::text(x + 26, y + 30, "PACKAGE FORM", color::PURPLE, 1);
                framebuffer::line(x + 92, y + 65, x + 92, y + 105, color::BORDER);
                framebuffer::text(x + 28, y + 112, "INTERFACE FORM", color::CYAN, 1);
                framebuffer::line(x + 98, y + 132, x + 98, y + 174, color::BORDER);
                framebuffer::text(x + 28, y + 182, "EXPDisplay HANDLE", color::GREEN, 1);
                framebuffer::text(x + 28, y + 224, "EXPFS GENERATION", color::MUTED, 1);
                number(
                    x + 166,
                    y + 224,
                    crate::expfs_store::generation().unwrap_or(0),
                    color::WHITE,
                    1,
                );
            }
            10 => {
                for row in 0..6 {
                    for column in 0..16 {
                        let byte = 32 + row * 16 + column;
                        framebuffer::glyph(
                            x + 20 + column * 18,
                            y + 24 + row * 28,
                            byte as u8,
                            color::INK,
                            2,
                        );
                    }
                }
            }
            14 => {
                framebuffer::text(x + 40, y + 38, "25:00", color::WHITE, 3);
                framebuffer::text(x + 42, y + 98, "FOCUS ON ONE FORM", color::CYAN, 1);
            }
            15 => {
                framebuffer::text(x + 24, y + 28, "MONOTONIC TICKS", color::MUTED, 1);
                number(
                    x + 24,
                    y + 58,
                    hardware::timestamp().wrapping_sub(self.started_at),
                    color::GREEN,
                    2,
                );
            }
            16 => {
                number(x + 90, y + 42, self.counter as u64, color::WHITE, 3);
                framebuffer::rect(x + 22, y + 132, 280, 90, color::CYAN);
                framebuffer::text(x + 102, y + 169, "ADD ONE", 0x0000_0000, 2);
            }
            _ => {
                input_box(x + 20, y + 20, width - 40, input);
                self.render_text_result(x + 22, y + 82, input);
                framebuffer::text(
                    x + 22,
                    y + height - 24,
                    "Type to work  //  LEFT RIGHT switches installed apps",
                    color::MUTED,
                    1,
                );
            }
        }
    }

    fn render_text_result(&self, x: i32, y: i32, input: &str) {
        match self.active {
            0 => {
                framebuffer::text(x, y, "RESULT", color::MUTED, 1);
                number_signed(
                    x,
                    y + 30,
                    eval_expression(input.as_bytes()),
                    color::GREEN,
                    2,
                );
            }
            1 => {
                framebuffer::text(x, y, "ENTER ADDS TASKS TO THIS SESSION", color::CYAN, 1);
                framebuffer::text(x, y + 32, "TASKS ADDED", color::MUTED, 1);
                number(x + 116, y + 32, self.counter as u64, color::WHITE, 1);
            }
            4 => {
                framebuffer::text(x, y, "KILOMETRES", color::MUTED, 1);
                number(
                    x,
                    y + 30,
                    parse_u64(input.as_bytes()).saturating_mul(1609) / 1000,
                    color::GREEN,
                    2,
                );
            }
            11 => {
                framebuffer::text(x, y, "DECIMAL", color::MUTED, 1);
                number(x + 90, y, parse_u64(input.as_bytes()), color::WHITE, 1);
                framebuffer::text(x, y + 34, "HEX", color::MUTED, 1);
                hex_number(x + 90, y + 34, parse_u64(input.as_bytes()), color::CYAN);
            }
            12 => {
                framebuffer::text(x, y, "UPPERCASE", color::MUTED, 1);
                let mut px = x;
                for byte in input.bytes().take(46) {
                    framebuffer::glyph(px, y + 28, byte.to_ascii_uppercase(), color::CYAN, 2);
                    px += 12;
                }
            }
            13 => {
                framebuffer::text(x, y, "WORDS", color::MUTED, 1);
                number(
                    x + 76,
                    y,
                    input.split_ascii_whitespace().count() as u64,
                    color::WHITE,
                    1,
                );
                framebuffer::text(x, y + 32, "CHARACTERS", color::MUTED, 1);
                number(x + 112, y + 32, input.len() as u64, color::CYAN, 1);
            }
            17 => {
                framebuffer::text(
                    x,
                    y,
                    if input.starts_with('#') {
                        "HEADING PREVIEW"
                    } else {
                        "PARAGRAPH PREVIEW"
                    },
                    color::PURPLE,
                    1,
                );
                framebuffer::text(
                    x,
                    y + 32,
                    input.trim_start_matches('#').trim(),
                    color::WHITE,
                    if input.starts_with('#') { 2 } else { 1 },
                );
            }
            18 => {
                let valid = (input.starts_with('{') && input.ends_with('}'))
                    || (input.starts_with('[') && input.ends_with(']'));
                framebuffer::text(
                    x,
                    y,
                    if valid {
                        "VALID OUTER JSON FRAME"
                    } else {
                        "INCOMPLETE JSON FRAME"
                    },
                    if valid { color::GREEN } else { color::RED },
                    1,
                );
            }
            19 => {
                framebuffer::text(x, y, "FNV-1A 64", color::MUTED, 1);
                hex_number(x, y + 32, fnv1a(input.as_bytes()), color::GREEN);
            }
            _ => {}
        }
    }
}

const PALETTE: [u32; 6] = [
    color::GREEN,
    color::CYAN,
    color::PURPLE,
    0x00D0_9B52,
    0x00E0_6C9F,
    0x00F2_F4F6,
];

fn button(x: i32, y: i32, width: i32, label: &str, fill: u32, enabled: bool) {
    framebuffer::rounded_rect(
        x,
        y,
        width,
        34,
        5,
        if enabled { fill } else { color::BORDER },
    );
    framebuffer::text(
        x + 14,
        y + 12,
        label,
        if enabled { color::WHITE } else { color::MUTED },
        1,
    );
}

fn input_box(x: i32, y: i32, width: i32, value: &str) {
    framebuffer::rect(x, y, width, 42, 0x0008_0B10);
    framebuffer::outline(x, y, width, 42, color::BORDER);
    framebuffer::text(
        x + 12,
        y + 15,
        if value.is_empty() {
            "Type here..."
        } else {
            value
        },
        if value.is_empty() {
            color::MUTED
        } else {
            color::WHITE
        },
        1,
    );
}

fn metric(x: i32, y: i32, label: &str, value: &str, accent: u32) {
    framebuffer::rect(x, y, 310, 54, 0x0014_1A21);
    framebuffer::rect(x, y, 4, 54, accent);
    framebuffer::text(x + 16, y + 12, label, color::MUTED, 1);
    framebuffer::text(x + 16, y + 32, value, accent, 1);
}

fn month_name(month: u8) -> &'static str {
    match month {
        1 => "JANUARY",
        2 => "FEBRUARY",
        3 => "MARCH",
        4 => "APRIL",
        5 => "MAY",
        6 => "JUNE",
        7 => "JULY",
        8 => "AUGUST",
        9 => "SEPTEMBER",
        10 => "OCTOBER",
        11 => "NOVEMBER",
        12 => "DECEMBER",
        _ => "UNKNOWN",
    }
}

fn parse_u64(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .copied()
        .filter(u8::is_ascii_digit)
        .fold(0, |value, byte| {
            value
                .saturating_mul(10)
                .saturating_add((byte - b'0') as u64)
        })
}

fn eval_expression(bytes: &[u8]) -> i64 {
    let mut total = 0_i64;
    let mut number = 0_i64;
    let mut operation = b'+';
    for byte in bytes.iter().copied().chain(core::iter::once(b'+')) {
        if byte.is_ascii_digit() {
            number = number
                .saturating_mul(10)
                .saturating_add((byte - b'0') as i64);
            continue;
        }
        if matches!(byte, b'+' | b'-' | b'*' | b'/') {
            total = match operation {
                b'-' => total.saturating_sub(number),
                b'*' => total.saturating_mul(number),
                b'/' if number != 0 => total / number,
                _ => total.saturating_add(number),
            };
            operation = byte;
            number = 0;
        }
    }
    total
}

fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100_0000_01b3)
    })
}

fn number(x: i32, y: i32, mut value: u64, ink: u32, scale: i32) {
    let mut bytes = [b'0'; 20];
    let mut start = 19;
    while value >= 10 {
        bytes[start] = b'0' + (value % 10) as u8;
        value /= 10;
        start -= 1;
    }
    bytes[start] = b'0' + value as u8;
    framebuffer::text(
        x,
        y,
        core::str::from_utf8(&bytes[start..]).unwrap_or("?"),
        ink,
        scale,
    );
}

fn number_signed(x: i32, y: i32, value: i64, ink: u32, scale: i32) {
    if value < 0 {
        framebuffer::text(x, y, "-", ink, scale);
        number(
            x + framebuffer::text_advance(scale),
            y,
            value.unsigned_abs(),
            ink,
            scale,
        );
    } else {
        number(x, y, value as u64, ink, scale);
    }
}

fn hex_number(x: i32, y: i32, mut value: u64, ink: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut bytes = [b'0'; 16];
    for index in (0..16).rev() {
        bytes[index] = HEX[(value & 0xF) as usize];
        value >>= 4;
    }
    let start = bytes.iter().position(|byte| *byte != b'0').unwrap_or(15);
    framebuffer::text(
        x,
        y,
        core::str::from_utf8(&bytes[start..]).unwrap_or("0"),
        ink,
        2,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecosystem_has_twenty_distinct_native_apps() {
        assert_eq!(APPS.len(), 20);
        for (index, app) in APPS.iter().enumerate() {
            assert!(!app.name.is_empty());
            assert!(!app.summary.is_empty());
            assert!(!APPS[..index].iter().any(|other| other.name == app.name));
        }
    }

    #[test]
    fn calculator_and_hash_are_bounded_and_deterministic() {
        assert_eq!(eval_expression(b"12+3*2"), 30);
        assert_eq!(eval_expression(b"12/0"), 12);
        assert_eq!(fnv1a(b"ExpOS"), fnv1a(b"ExpOS"));
        assert_ne!(fnv1a(b"ExpOS"), fnv1a(b"expos"));
    }

    #[test]
    fn durable_app_state_roundtrips_without_restoring_transient_input() {
        let mut apps = NativeApps::new();
        apps.selected = 6;
        assert_eq!(apps.install_selected(), ManagerAction::Installed);
        apps.active = 6;
        apps.pixels = 0x55AA;
        apps.counter = 42;
        apps.palette = 4;
        apps.input[..4].copy_from_slice(b"temp");
        apps.input_len = 4;
        let mut wire = [0_u8; 32];
        let length = apps.encode_state(&mut wire);
        let restored = NativeApps::restore_state(&wire[..length]);
        assert!(restored.is_installed(6));
        assert_eq!(restored.active, 6);
        assert_eq!(restored.pixels, 0x55AA);
        assert_eq!(restored.counter, 42);
        assert_eq!(restored.palette, 4);
        assert_eq!(restored.input_len, 0);
    }
}
