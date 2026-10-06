//! Native package-app host used by the Ayo desktop manager.
//!
//! Each entry is a separate Interface Form in the Ayo catalog.  The host is
//! intentionally shared: thirty tools should not consume thirty kernel
//! surfaces or thirty ambient capabilities just to be installable.

use crate::{framebuffer, hardware, network};
use expos_core::Rect;
use framebuffer::color;

pub const PACKAGE_COUNT: usize = 30;
const INPUT_CAPACITY: usize = 96;
const PAGE_SIZE: usize = 8;
const TASK_CAPACITY: usize = 6;
const TASK_TEXT_CAPACITY: usize = 28;
const WEATHER_CITY_CAPACITY: usize = 40;
const WEATHER_LOCATION_CAPACITY: usize = 48;
const WEATHER_COORDINATE_CAPACITY: usize = 20;
const WEATHER_FORECAST_DAYS: usize = 5;

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
        summary: "Start, pause and reset focus sessions",
        accent: 0x00D0_9B52,
    },
    PackageApp {
        name: "Stopwatch",
        category: "Utilities",
        summary: "Accurate monotonic elapsed time",
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
    PackageApp {
        name: "Weather",
        category: "Internet",
        summary: "Live Open-Meteo weather and local forecast",
        accent: 0x004D_A3FF,
    },
    PackageApp {
        name: "Timer",
        category: "Utilities",
        summary: "A configurable countdown with pause and reset",
        accent: 0x00FF_A657,
    },
    PackageApp {
        name: "TipCalculator",
        category: "Utilities",
        summary: "Split a bill and calculate a tip",
        accent: color::GREEN,
    },
    PackageApp {
        name: "PasswordGen",
        category: "Security",
        summary: "Generate passwords from hardware entropy",
        accent: color::PURPLE,
    },
    PackageApp {
        name: "Dice",
        category: "Utilities",
        summary: "Roll one or more fair virtual dice",
        accent: 0x00D0_9B52,
    },
    PackageApp {
        name: "DateCalc",
        category: "Utilities",
        summary: "Find the weekday for a Gregorian date",
        accent: color::CYAN,
    },
    PackageApp {
        name: "TextDiff",
        category: "Editors",
        summary: "Compare two short strings side by side",
        accent: 0x00E0_6C9F,
    },
    PackageApp {
        name: "SubnetCalc",
        category: "Networking",
        summary: "Calculate IPv4 network, mask and broadcast",
        accent: color::GREEN,
    },
    PackageApp {
        name: "Morse",
        category: "Utilities",
        summary: "Encode text as International Morse code",
        accent: color::CYAN,
    },
    PackageApp {
        name: "UUIDGen",
        category: "Developer tools",
        summary: "Create random RFC 4122 version 4 UUIDs",
        accent: color::PURPLE,
    },
];

pub fn package_name(index: usize) -> Option<&'static str> {
    APPS.get(index).map(|package| package.name)
}

pub fn package_category(index: usize) -> Option<&'static str> {
    APPS.get(index).map(|package| package.category)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagerAction {
    Changed,
    InstallRequested,
    Open,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppAction {
    Changed,
    WeatherRefresh,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeatherState {
    Empty,
    Loading,
    Ready,
    Error,
}

#[derive(Clone, Copy)]
struct Task {
    text: [u8; TASK_TEXT_CAPACITY],
    length: u8,
    done: bool,
}

impl Task {
    const EMPTY: Self = Self {
        text: [0; TASK_TEXT_CAPACITY],
        length: 0,
        done: false,
    };
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
    stopwatch_started_at: u64,
    stopwatch_elapsed_ticks: u64,
    stopwatch_running: bool,
    countdown_started_at: u64,
    countdown_seconds: u32,
    countdown_remaining: u32,
    countdown_running: bool,
    focus_minutes: u16,
    focus_started_at: u64,
    focus_remaining: u32,
    focus_running: bool,
    calendar_offset: i16,
    text_mode: u8,
    converter_mode: u8,
    clock_24h: bool,
    tasks: [Task; TASK_CAPACITY],
    task_count: usize,
    task_selected: usize,
    generated: [u8; 40],
    generated_len: usize,
    weather_state: WeatherState,
    weather_city: [u8; WEATHER_CITY_CAPACITY],
    weather_city_len: usize,
    weather_location: [u8; WEATHER_LOCATION_CAPACITY],
    weather_location_len: usize,
    weather_latitude: [u8; WEATHER_COORDINATE_CAPACITY],
    weather_latitude_len: usize,
    weather_longitude: [u8; WEATHER_COORDINATE_CAPACITY],
    weather_longitude_len: usize,
    weather_temperature_tenths: i16,
    weather_apparent_tenths: i16,
    weather_humidity: u8,
    weather_wind_tenths: u16,
    weather_code: u8,
    weather_is_day: bool,
    weather_fahrenheit: bool,
    weather_resolve_location: bool,
    weather_forecast_count: usize,
    weather_forecast_selected: usize,
    weather_forecast_highs: [i16; WEATHER_FORECAST_DAYS],
    weather_forecast_lows: [i16; WEATHER_FORECAST_DAYS],
    weather_forecast_codes: [u8; WEATHER_FORECAST_DAYS],
    weather_forecast_precipitation: [u8; WEATHER_FORECAST_DAYS],
    weather_notice: &'static str,
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
            stopwatch_started_at: 0,
            stopwatch_elapsed_ticks: 0,
            stopwatch_running: false,
            countdown_started_at: 0,
            countdown_seconds: 300,
            countdown_remaining: 300,
            countdown_running: false,
            focus_minutes: 25,
            focus_started_at: 0,
            focus_remaining: 25 * 60,
            focus_running: false,
            calendar_offset: 0,
            text_mode: 0,
            converter_mode: 0,
            clock_24h: true,
            tasks: [Task::EMPTY; TASK_CAPACITY],
            task_count: 0,
            task_selected: 0,
            generated: [0; 40],
            generated_len: 0,
            weather_state: WeatherState::Empty,
            weather_city: [0; WEATHER_CITY_CAPACITY],
            weather_city_len: 0,
            weather_location: [0; WEATHER_LOCATION_CAPACITY],
            weather_location_len: 0,
            weather_latitude: [0; WEATHER_COORDINATE_CAPACITY],
            weather_latitude_len: 0,
            weather_longitude: [0; WEATHER_COORDINATE_CAPACITY],
            weather_longitude_len: 0,
            weather_temperature_tenths: 0,
            weather_apparent_tenths: 0,
            weather_humidity: 0,
            weather_wind_tenths: 0,
            weather_code: 0,
            weather_is_day: true,
            weather_fahrenheit: false,
            weather_resolve_location: true,
            weather_forecast_count: 0,
            weather_forecast_selected: 0,
            weather_forecast_highs: [0; WEATHER_FORECAST_DAYS],
            weather_forecast_lows: [0; WEATHER_FORECAST_DAYS],
            weather_forecast_codes: [0; WEATHER_FORECAST_DAYS],
            weather_forecast_precipitation: [0; WEATHER_FORECAST_DAYS],
            weather_notice: "Type a city and press Enter for live weather.",
            persistence_generation: 0,
            notice: "Select a package, then install its signed built-in artifact.",
        }
    }

    pub fn handle_manager_key(&mut self, key: u8) -> Option<ManagerAction> {
        match key {
            crate::input::KEY_UP | b'k' | b'K' => self.selected = self.selected.saturating_sub(1),
            crate::input::KEY_DOWN | b'j' | b'J' => {
                self.selected = (self.selected + 1).min(PACKAGE_COUNT - 1)
            }
            b'i' | b'I' => return Some(ManagerAction::InstallRequested),
            b'\n' => {
                if self.is_installed(self.selected) {
                    self.active = self.selected;
                    self.prepare_active_input();
                    self.mark_persistent_change();
                    return Some(ManagerAction::Open);
                }
                return Some(ManagerAction::InstallRequested);
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
                return Some(ManagerAction::InstallRequested);
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

    pub fn commit_install_selected(&mut self) {
        self.install(self.selected);
        self.notice = "Verified -> DIESE -> PIMP -> installed Interface Form.";
    }

    pub fn install(&mut self, index: usize) -> bool {
        if index >= PACKAGE_COUNT {
            return false;
        }
        let changed = !self.is_installed(index);
        self.installed |= 1 << index;
        self.selected = index;
        self.active = index;
        self.mark_persistent_change();
        changed
    }

    pub fn uninstall(&mut self, index: usize) -> bool {
        if index >= PACKAGE_COUNT || !self.is_installed(index) {
            return false;
        }
        self.installed &= !(1 << index);
        if self.active == index {
            self.active = (0..PACKAGE_COUNT)
                .find(|candidate| self.is_installed(*candidate))
                .unwrap_or(0);
        }
        self.mark_persistent_change();
        true
    }

    pub fn find_package(name: &str) -> Option<usize> {
        let needle = name.trim();
        APPS.iter()
            .position(|package| package.name.eq_ignore_ascii_case(needle))
    }

    pub const fn installed(&self, index: usize) -> bool {
        self.is_installed(index)
    }

    pub const fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn select_package(&mut self, index: usize) -> bool {
        if index >= PACKAGE_COUNT {
            return false;
        }
        self.selected = index;
        true
    }

    pub const fn active_index(&self) -> usize {
        self.active
    }

    pub const fn installed_count(&self) -> usize {
        self.installed.count_ones() as usize
    }

    pub const fn installed_at(&self, ordinal: usize) -> Option<usize> {
        let mut index = 0;
        let mut seen = 0;
        while index < PACKAGE_COUNT {
            if self.is_installed(index) {
                if seen == ordinal {
                    return Some(index);
                }
                seen += 1;
            }
            index += 1;
        }
        None
    }

    pub fn activate_installed(&mut self, index: usize) -> bool {
        if index >= PACKAGE_COUNT || !self.is_installed(index) {
            return false;
        }
        self.active = index;
        self.prepare_active_input();
        true
    }

    pub const fn app_name(index: usize) -> &'static str {
        if index < PACKAGE_COUNT {
            APPS[index].name
        } else {
            "App"
        }
    }

    pub const fn app_category(index: usize) -> &'static str {
        if index < PACKAGE_COUNT {
            APPS[index].category
        } else {
            "Other"
        }
    }

    pub const fn app_accent(index: usize) -> u32 {
        if index < PACKAGE_COUNT {
            APPS[index].accent
        } else {
            color::CYAN
        }
    }

    pub const fn persistence_generation(&self) -> u32 {
        self.persistence_generation
    }

    fn mark_persistent_change(&mut self) {
        self.persistence_generation = self.persistence_generation.wrapping_add(1).max(1);
    }

    /// Serialize the durable shared state of the Ayo app host. Typed input is
    /// deliberately transient; installed packages and tool results survive.
    pub const STATE_CAPACITY: usize = 448;

    pub fn encode_state(&self, out: &mut [u8; Self::STATE_CAPACITY]) -> usize {
        out.fill(0);
        out[..4].copy_from_slice(b"AYO3");
        out[4..8].copy_from_slice(&self.installed.to_le_bytes());
        out[8] = self.active as u8;
        out[9] = self.palette as u8;
        out[10..14].copy_from_slice(&self.counter.to_le_bytes());
        out[14..22].copy_from_slice(&self.pixels.to_le_bytes());
        out[22..26].copy_from_slice(&self.countdown_seconds.to_le_bytes());
        out[26..28].copy_from_slice(&self.focus_minutes.to_le_bytes());
        out[28] = self.converter_mode;
        out[29] = self.text_mode;
        out[30] = u8::from(self.clock_24h);
        out[31] = self.task_count as u8;
        out[32] = self.task_selected as u8;
        let mut offset = 33;
        for task in self.tasks {
            out[offset] = task.length;
            out[offset + 1] = u8::from(task.done);
            out[offset + 2..offset + 2 + TASK_TEXT_CAPACITY].copy_from_slice(&task.text);
            offset += TASK_TEXT_CAPACITY + 2;
        }
        out[offset] = self.weather_city_len as u8;
        offset += 1;
        out[offset..offset + WEATHER_CITY_CAPACITY].copy_from_slice(&self.weather_city);
        offset += WEATHER_CITY_CAPACITY;
        out[offset] = self.weather_location_len as u8;
        offset += 1;
        out[offset..offset + WEATHER_LOCATION_CAPACITY].copy_from_slice(&self.weather_location);
        offset += WEATHER_LOCATION_CAPACITY;
        out[offset..offset + 2].copy_from_slice(&self.weather_temperature_tenths.to_le_bytes());
        out[offset + 2..offset + 4].copy_from_slice(&self.weather_apparent_tenths.to_le_bytes());
        out[offset + 4] = self.weather_humidity;
        out[offset + 5..offset + 7].copy_from_slice(&self.weather_wind_tenths.to_le_bytes());
        out[offset + 7] = self.weather_code;
        out[offset + 8] = u8::from(self.weather_is_day);
        out[offset + 9] = u8::from(matches!(self.weather_state, WeatherState::Ready));
        offset += 10;
        out[offset] = self.weather_latitude_len as u8;
        offset += 1;
        out[offset..offset + WEATHER_COORDINATE_CAPACITY].copy_from_slice(&self.weather_latitude);
        offset += WEATHER_COORDINATE_CAPACITY;
        out[offset] = self.weather_longitude_len as u8;
        offset += 1;
        out[offset..offset + WEATHER_COORDINATE_CAPACITY].copy_from_slice(&self.weather_longitude);
        offset += WEATHER_COORDINATE_CAPACITY;
        out[offset] = u8::from(self.weather_fahrenheit);
        out[offset + 1] = self.weather_forecast_count as u8;
        out[offset + 2] = self.weather_forecast_selected as u8;
        offset += 3;
        for value in self.weather_forecast_highs {
            out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            offset += 2;
        }
        for value in self.weather_forecast_lows {
            out[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
            offset += 2;
        }
        out[offset..offset + WEATHER_FORECAST_DAYS].copy_from_slice(&self.weather_forecast_codes);
        offset += WEATHER_FORECAST_DAYS;
        out[offset..offset + WEATHER_FORECAST_DAYS]
            .copy_from_slice(&self.weather_forecast_precipitation);
        offset + WEATHER_FORECAST_DAYS
    }

    pub fn restore_state(bytes: &[u8]) -> Self {
        let mut state = Self::new();
        if bytes.len() < 22
            || (&bytes[..4] != b"AYO1" && &bytes[..4] != b"AYO2" && &bytes[..4] != b"AYO3")
        {
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
        if (&bytes[..4] == b"AYO2" || &bytes[..4] == b"AYO3") && bytes.len() >= 22 + 4 + 2 + 5 {
            state.countdown_seconds =
                u32::from_le_bytes(bytes[22..26].try_into().unwrap_or(300_u32.to_le_bytes()))
                    .clamp(1, 24 * 60 * 60);
            state.countdown_remaining = state.countdown_seconds;
            state.focus_minutes =
                u16::from_le_bytes(bytes[26..28].try_into().unwrap_or(25_u16.to_le_bytes()))
                    .clamp(1, 180);
            state.focus_remaining = state.focus_minutes as u32 * 60;
            state.converter_mode = bytes[28] % 4;
            state.text_mode = bytes[29] % 3;
            state.clock_24h = bytes[30] != 0;
            state.task_count = (bytes[31] as usize).min(TASK_CAPACITY);
            state.task_selected = (bytes[32] as usize).min(state.task_count.saturating_sub(1));
            let mut offset = 33;
            for task in &mut state.tasks {
                if offset + TASK_TEXT_CAPACITY + 2 > bytes.len() {
                    break;
                }
                task.length = bytes[offset].min(TASK_TEXT_CAPACITY as u8);
                task.done = bytes[offset + 1] != 0;
                task.text
                    .copy_from_slice(&bytes[offset + 2..offset + 2 + TASK_TEXT_CAPACITY]);
                offset += TASK_TEXT_CAPACITY + 2;
            }
            if offset < bytes.len() {
                state.weather_city_len = (bytes[offset] as usize)
                    .min(WEATHER_CITY_CAPACITY)
                    .min(bytes.len().saturating_sub(offset + 1));
                offset += 1;
                state.weather_city[..state.weather_city_len]
                    .copy_from_slice(&bytes[offset..offset + state.weather_city_len]);
                offset += WEATHER_CITY_CAPACITY.min(bytes.len().saturating_sub(offset));
            }
            if offset < bytes.len() {
                state.weather_location_len = (bytes[offset] as usize)
                    .min(WEATHER_LOCATION_CAPACITY)
                    .min(bytes.len().saturating_sub(offset + 1));
                offset += 1;
                state.weather_location[..state.weather_location_len]
                    .copy_from_slice(&bytes[offset..offset + state.weather_location_len]);
                offset += WEATHER_LOCATION_CAPACITY.min(bytes.len().saturating_sub(offset));
            }
            if offset + 10 <= bytes.len() {
                state.weather_temperature_tenths =
                    i16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap_or([0; 2]));
                state.weather_apparent_tenths =
                    i16::from_le_bytes(bytes[offset + 2..offset + 4].try_into().unwrap_or([0; 2]));
                state.weather_humidity = bytes[offset + 4];
                state.weather_wind_tenths =
                    u16::from_le_bytes(bytes[offset + 5..offset + 7].try_into().unwrap_or([0; 2]));
                state.weather_code = bytes[offset + 7];
                state.weather_is_day = bytes[offset + 8] != 0;
                if bytes[offset + 9] != 0 {
                    state.weather_state = WeatherState::Ready;
                    state.weather_notice = "Restored latest weather; Enter refreshes.";
                }
                offset += 10;
            }
            if &bytes[..4] == b"AYO3" && offset + 2 <= bytes.len() {
                state.weather_latitude_len = (bytes[offset] as usize)
                    .min(WEATHER_COORDINATE_CAPACITY)
                    .min(bytes.len().saturating_sub(offset + 1));
                offset += 1;
                state.weather_latitude[..state.weather_latitude_len]
                    .copy_from_slice(&bytes[offset..offset + state.weather_latitude_len]);
                offset += WEATHER_COORDINATE_CAPACITY.min(bytes.len().saturating_sub(offset));
                if offset < bytes.len() {
                    state.weather_longitude_len = (bytes[offset] as usize)
                        .min(WEATHER_COORDINATE_CAPACITY)
                        .min(bytes.len().saturating_sub(offset + 1));
                    offset += 1;
                    state.weather_longitude[..state.weather_longitude_len]
                        .copy_from_slice(&bytes[offset..offset + state.weather_longitude_len]);
                    offset += WEATHER_COORDINATE_CAPACITY.min(bytes.len().saturating_sub(offset));
                }
                if offset + 3 + WEATHER_FORECAST_DAYS * 6 <= bytes.len() {
                    state.weather_fahrenheit = bytes[offset] != 0;
                    state.weather_forecast_count =
                        (bytes[offset + 1] as usize).min(WEATHER_FORECAST_DAYS);
                    state.weather_forecast_selected = (bytes[offset + 2] as usize)
                        .min(state.weather_forecast_count.saturating_sub(1));
                    offset += 3;
                    for value in &mut state.weather_forecast_highs {
                        *value = i16::from_le_bytes(
                            bytes[offset..offset + 2].try_into().unwrap_or([0; 2]),
                        );
                        offset += 2;
                    }
                    for value in &mut state.weather_forecast_lows {
                        *value = i16::from_le_bytes(
                            bytes[offset..offset + 2].try_into().unwrap_or([0; 2]),
                        );
                        offset += 2;
                    }
                    state
                        .weather_forecast_codes
                        .copy_from_slice(&bytes[offset..offset + WEATHER_FORECAST_DAYS]);
                    offset += WEATHER_FORECAST_DAYS;
                    state
                        .weather_forecast_precipitation
                        .copy_from_slice(&bytes[offset..offset + WEATHER_FORECAST_DAYS]);
                }
                state.weather_resolve_location =
                    state.weather_latitude_len == 0 || state.weather_longitude_len == 0;
            }
        }
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
            "30 functional apps  //  installed only when you choose",
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

    pub fn handle_app_key(&mut self, key: u8) -> Option<AppAction> {
        if self.installed == 0 {
            return None;
        }
        if self.active == 1 {
            return self.handle_tasks_key(key);
        }
        if self.active == 20 && key == b'\n' {
            let city = &self.input[..self.input_len];
            if !valid_weather_city(city) {
                self.weather_failed(
                    "Use 1-40 letters, numbers, spaces, apostrophes, commas, dots, or hyphens.",
                );
                return Some(AppAction::Changed);
            }
            let same_city = city.eq_ignore_ascii_case(&self.weather_city[..self.weather_city_len]);
            self.weather_resolve_location =
                !same_city || self.weather_latitude_len == 0 || self.weather_longitude_len == 0;
            self.weather_city.fill(0);
            self.weather_city_len = self.input_len.min(WEATHER_CITY_CAPACITY);
            self.weather_city[..self.weather_city_len]
                .copy_from_slice(&self.input[..self.weather_city_len]);
            self.weather_state = WeatherState::Loading;
            self.weather_notice = "Finding the city with Open-Meteo...";
            self.mark_persistent_change();
            return Some(AppAction::WeatherRefresh);
        }
        match key {
            0x08 => {
                self.input_len = self.input_len.saturating_sub(1);
                Some(AppAction::Changed)
            }
            b'\n' if self.active == 16 => {
                self.counter = self.counter.saturating_add(1);
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b'\n' | b' ' if self.active == 23 => {
                self.generate_password();
                Some(AppAction::Changed)
            }
            b'\n' | b' ' if self.active == 24 => {
                self.counter = (hardware::random_u32() % 6) + 1;
                Some(AppAction::Changed)
            }
            b'\n' | b' ' if self.active == 29 => {
                self.generate_uuid();
                Some(AppAction::Changed)
            }
            b'\t' if self.active == 20 => {
                self.weather_fahrenheit = !self.weather_fahrenheit;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            crate::input::KEY_LEFT if self.active == 20 && self.weather_forecast_count != 0 => {
                self.weather_forecast_selected = self.weather_forecast_selected.saturating_sub(1);
                Some(AppAction::Changed)
            }
            crate::input::KEY_RIGHT if self.active == 20 && self.weather_forecast_count != 0 => {
                self.weather_forecast_selected =
                    (self.weather_forecast_selected + 1).min(self.weather_forecast_count - 1);
                Some(AppAction::Changed)
            }
            b'\n' if self.active == 21 => {
                let seconds =
                    parse_u64(&self.input[..self.input_len]).clamp(1, 24 * 60 * 60) as u32;
                self.countdown_seconds = seconds;
                self.countdown_remaining = seconds;
                self.countdown_started_at = hardware::timestamp();
                self.countdown_running = true;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 2 => {
                self.clock_24h = !self.clock_24h;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 4 => {
                self.converter_mode = (self.converter_mode + 1) % 4;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 12 => {
                self.text_mode = (self.text_mode + 1) % 3;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 14 => {
                self.toggle_focus();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 15 => {
                self.toggle_stopwatch();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 16 => {
                self.counter = self.counter.saturating_add(1);
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 21 => {
                self.toggle_countdown();
                Some(AppAction::Changed)
            }
            b' ' if self.active == 5 => {
                self.palette = (self.palette + 1) % PALETTE.len();
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b'c' | b'C' if self.active == 6 => {
                self.pixels = 0;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b'i' | b'I' if self.active == 6 => {
                self.pixels = !self.pixels;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b'r' | b'R' if self.active == 14 => {
                self.focus_running = false;
                self.focus_remaining = self.focus_minutes as u32 * 60;
                Some(AppAction::Changed)
            }
            b'+' | b'=' if self.active == 14 => {
                self.focus_minutes = (self.focus_minutes + 5).min(180);
                self.focus_remaining = self.focus_minutes as u32 * 60;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b'-' if self.active == 14 => {
                self.focus_minutes = self.focus_minutes.saturating_sub(5).max(5);
                self.focus_remaining = self.focus_minutes as u32 * 60;
                self.mark_persistent_change();
                Some(AppAction::Changed)
            }
            b'r' | b'R' if self.active == 15 => {
                self.stopwatch_running = false;
                self.stopwatch_elapsed_ticks = 0;
                Some(AppAction::Changed)
            }
            b'r' | b'R' if self.active == 21 => {
                self.countdown_running = false;
                self.countdown_remaining = self.countdown_seconds;
                Some(AppAction::Changed)
            }
            crate::input::KEY_LEFT if self.active == 3 => {
                self.calendar_offset = self.calendar_offset.saturating_sub(1);
                Some(AppAction::Changed)
            }
            crate::input::KEY_RIGHT if self.active == 3 => {
                self.calendar_offset = self.calendar_offset.saturating_add(1);
                Some(AppAction::Changed)
            }
            b'+' | b'=' if self.active == 23 => {
                self.counter = (self.counter + 1).clamp(8, 32);
                self.generate_password();
                Some(AppAction::Changed)
            }
            b'-' if self.active == 23 => {
                self.counter = self.counter.saturating_sub(1).clamp(8, 32);
                self.generate_password();
                Some(AppAction::Changed)
            }
            b'\n' => {
                if self.active == 16 {
                    self.counter = self.counter.saturating_add(1);
                    self.mark_persistent_change();
                }
                Some(AppAction::Changed)
            }
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.input_len < INPUT_CAPACITY =>
            {
                self.input[self.input_len] = byte;
                self.input_len += 1;
                Some(AppAction::Changed)
            }
            _ => None,
        }
    }

    pub fn handle_app_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        if self.installed == 0 {
            return false;
        }
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        if self.active == 6 && (40..296).contains(&local_x) && (116..372).contains(&local_y) {
            let column = ((local_x - 40) / 32) as u64;
            let row = ((local_y - 116) / 32) as u64;
            self.pixels ^= 1_u64 << (row * 8 + column);
            self.mark_persistent_change();
            return true;
        }
        if self.active == 16 && (50..330).contains(&local_x) && (240..334).contains(&local_y) {
            self.counter = self.counter.saturating_add(1);
            self.mark_persistent_change();
            return true;
        }
        false
    }

    fn clear_input(&mut self) {
        self.input.fill(0);
        self.input_len = 0;
    }

    fn prepare_active_input(&mut self) {
        self.clear_input();
        if self.active == 20 && self.weather_city_len != 0 {
            self.input_len = self.weather_city_len.min(INPUT_CAPACITY);
            self.input[..self.input_len].copy_from_slice(&self.weather_city[..self.input_len]);
        }
    }

    fn handle_tasks_key(&mut self, key: u8) -> Option<AppAction> {
        match key {
            crate::input::KEY_UP if self.task_count != 0 => {
                self.task_selected = self.task_selected.saturating_sub(1);
            }
            crate::input::KEY_DOWN if self.task_count != 0 => {
                self.task_selected = (self.task_selected + 1).min(self.task_count - 1);
            }
            b'\n' if self.input_len != 0 && self.task_count < TASK_CAPACITY => {
                let length = self.input_len.min(TASK_TEXT_CAPACITY);
                let task = &mut self.tasks[self.task_count];
                task.text[..length].copy_from_slice(&self.input[..length]);
                task.length = length as u8;
                task.done = false;
                self.task_selected = self.task_count;
                self.task_count += 1;
                self.clear_input();
                self.mark_persistent_change();
            }
            b'\n' | b' ' if self.input_len == 0 && self.task_count != 0 => {
                self.tasks[self.task_selected].done = !self.tasks[self.task_selected].done;
                self.mark_persistent_change();
            }
            0x08 if self.input_len != 0 => {
                self.input_len -= 1;
            }
            0x08 if self.task_count != 0 => {
                for index in self.task_selected..self.task_count - 1 {
                    self.tasks[index] = self.tasks[index + 1];
                }
                self.task_count -= 1;
                self.tasks[self.task_count] = Task::EMPTY;
                self.task_selected = self.task_selected.min(self.task_count.saturating_sub(1));
                self.mark_persistent_change();
            }
            byte if (byte.is_ascii_graphic() || byte == b' ')
                && self.input_len < TASK_TEXT_CAPACITY =>
            {
                self.input[self.input_len] = byte;
                self.input_len += 1;
            }
            _ => return None,
        }
        Some(AppAction::Changed)
    }

    fn toggle_focus(&mut self) {
        if self.focus_running {
            self.focus_remaining = self.current_focus_seconds();
            self.focus_running = false;
        } else {
            if self.focus_remaining == 0 {
                self.focus_remaining = self.focus_minutes as u32 * 60;
            }
            self.focus_started_at = hardware::timestamp();
            self.focus_running = true;
        }
    }

    fn current_focus_seconds(&self) -> u32 {
        if !self.focus_running {
            return self.focus_remaining;
        }
        self.focus_remaining
            .saturating_sub(elapsed_seconds(self.focus_started_at) as u32)
    }

    fn toggle_stopwatch(&mut self) {
        if self.stopwatch_running {
            self.stopwatch_elapsed_ticks = self
                .stopwatch_elapsed_ticks
                .saturating_add(hardware::timestamp().saturating_sub(self.stopwatch_started_at));
            self.stopwatch_running = false;
        } else {
            self.stopwatch_started_at = hardware::timestamp();
            self.stopwatch_running = true;
        }
    }

    fn stopwatch_seconds(&self) -> u64 {
        let current = if self.stopwatch_running {
            hardware::timestamp().saturating_sub(self.stopwatch_started_at)
        } else {
            0
        };
        let ticks = self.stopwatch_elapsed_ticks.saturating_add(current);
        ticks / hardware::clock_info().tsc_hz.max(1)
    }

    fn toggle_countdown(&mut self) {
        if self.countdown_running {
            self.countdown_remaining = self.current_countdown_seconds();
            self.countdown_running = false;
        } else {
            if self.countdown_remaining == 0 {
                self.countdown_remaining = self.countdown_seconds;
            }
            self.countdown_started_at = hardware::timestamp();
            self.countdown_running = true;
        }
    }

    fn current_countdown_seconds(&self) -> u32 {
        if !self.countdown_running {
            return self.countdown_remaining;
        }
        self.countdown_remaining
            .saturating_sub(elapsed_seconds(self.countdown_started_at) as u32)
    }

    fn generate_password(&mut self) {
        const ALPHABET: &[u8] =
            b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%+-_";
        let material = crate::crypto::random_material::<32>(b"Ayo PasswordGen v1");
        let length = if self.counter == 0 {
            16
        } else {
            self.counter.clamp(8, 32) as usize
        };
        self.counter = length as u32;
        self.generated_len = length;
        for index in 0..length {
            self.generated[index] = ALPHABET[material[index] as usize % ALPHABET.len()];
        }
    }

    fn generate_uuid(&mut self) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut material = crate::crypto::random_material::<16>(b"Ayo UUIDGen v4");
        material[6] = (material[6] & 0x0F) | 0x40;
        material[8] = (material[8] & 0x3F) | 0x80;
        let mut output = 0;
        for (index, byte) in material.into_iter().enumerate() {
            if matches!(index, 4 | 6 | 8 | 10) {
                self.generated[output] = b'-';
                output += 1;
            }
            self.generated[output] = HEX[(byte >> 4) as usize];
            self.generated[output + 1] = HEX[(byte & 0x0F) as usize];
            output += 2;
        }
        self.generated_len = output;
    }

    pub fn weather_search_url(&self, output: &mut [u8; 512]) -> Option<usize> {
        let mut length = push_bytes(
            output,
            0,
            b"https://geocoding-api.open-meteo.com/v1/search?count=1&language=en&format=json&name=",
        )?;
        length = push_percent_encoded(output, length, &self.weather_city[..self.weather_city_len])?;
        Some(length)
    }

    pub fn apply_weather_location(&mut self, body: &[u8]) -> bool {
        let Some(latitude) = json_number(body, b"\"latitude\"") else {
            self.weather_failed("City was not found by Open-Meteo.");
            return false;
        };
        let Some(longitude) = json_number(body, b"\"longitude\"") else {
            self.weather_failed("The weather location response was incomplete.");
            return false;
        };
        if !coordinate_in_range(latitude, 900) || !coordinate_in_range(longitude, 1_800) {
            self.weather_failed("Open-Meteo returned coordinates outside Earth bounds.");
            return false;
        }
        self.weather_latitude_len = latitude.len().min(WEATHER_COORDINATE_CAPACITY);
        self.weather_latitude[..self.weather_latitude_len]
            .copy_from_slice(&latitude[..self.weather_latitude_len]);
        self.weather_longitude_len = longitude.len().min(WEATHER_COORDINATE_CAPACITY);
        self.weather_longitude[..self.weather_longitude_len]
            .copy_from_slice(&longitude[..self.weather_longitude_len]);
        self.weather_location.fill(0);
        self.weather_location_len = 0;
        if let Some(name) = json_string(body, b"\"name\"") {
            self.weather_location_len = name.len().min(WEATHER_LOCATION_CAPACITY);
            self.weather_location[..self.weather_location_len]
                .copy_from_slice(&name[..self.weather_location_len]);
        }
        if let Some(country) = json_string(body, b"\"country\"") {
            if self.weather_location_len + 2 < WEATHER_LOCATION_CAPACITY {
                self.weather_location[self.weather_location_len..self.weather_location_len + 2]
                    .copy_from_slice(b", ");
                self.weather_location_len += 2;
                let count = country
                    .len()
                    .min(WEATHER_LOCATION_CAPACITY - self.weather_location_len);
                self.weather_location[self.weather_location_len..self.weather_location_len + count]
                    .copy_from_slice(&country[..count]);
                self.weather_location_len += count;
            }
        }
        self.weather_notice = "Location found. Downloading current conditions...";
        self.weather_resolve_location = false;
        true
    }

    pub fn weather_forecast_url(&self, output: &mut [u8; 512]) -> Option<usize> {
        let mut length = push_bytes(
            output,
            0,
            b"https://api.open-meteo.com/v1/forecast?latitude=",
        )?;
        length = push_bytes(
            output,
            length,
            &self.weather_latitude[..self.weather_latitude_len],
        )?;
        length = push_bytes(output, length, b"&longitude=")?;
        length = push_bytes(
            output,
            length,
            &self.weather_longitude[..self.weather_longitude_len],
        )?;
        push_bytes(
            output,
            length,
            b"&current=temperature_2m,relative_humidity_2m,apparent_temperature,is_day,weather_code,cloud_cover,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&forecast_days=5&timezone=auto",
        )
    }

    pub fn apply_weather_forecast(&mut self, body: &[u8]) -> bool {
        // Open-Meteo places `current_units` before `current`. Restrict scalar
        // lookup to the current object so unit strings such as `\"°C\"` cannot
        // shadow the numeric observations with the same field names.
        let Some(current_start) = json_value_start(body, b"\"current\"") else {
            self.weather_failed("Open-Meteo returned weather data ExpOS could not read.");
            return false;
        };
        let current = &body[current_start..];
        let Some(temperature) =
            json_number(current, b"\"temperature_2m\"").and_then(parse_decimal_tenths)
        else {
            self.weather_failed("Open-Meteo returned weather data ExpOS could not read.");
            return false;
        };
        if !(-1_000..=700).contains(&temperature) {
            self.weather_failed("Open-Meteo returned a temperature outside supported bounds.");
            return false;
        }
        self.weather_temperature_tenths = temperature;
        self.weather_apparent_tenths = json_number(current, b"\"apparent_temperature\"")
            .and_then(parse_decimal_tenths)
            .unwrap_or(temperature);
        self.weather_humidity = json_number(current, b"\"relative_humidity_2m\"")
            .and_then(parse_u8_ascii)
            .unwrap_or(0)
            .min(100);
        self.weather_wind_tenths = json_number(current, b"\"wind_speed_10m\"")
            .and_then(parse_decimal_tenths)
            .unwrap_or(0)
            .unsigned_abs();
        self.weather_code = json_number(current, b"\"weather_code\"")
            .and_then(parse_u8_ascii)
            .unwrap_or(0);
        self.weather_is_day = json_number(current, b"\"is_day\"")
            .and_then(parse_u8_ascii)
            .unwrap_or(1)
            != 0;
        self.weather_forecast_count = 0;
        if let Some(daily_start) = json_value_start(body, b"\"daily\"") {
            let daily = &body[daily_start..];
            for day in 0..WEATHER_FORECAST_DAYS {
                let Some(high) = json_array_number(daily, b"\"temperature_2m_max\"", day)
                    .and_then(parse_decimal_tenths)
                else {
                    break;
                };
                let Some(low) = json_array_number(daily, b"\"temperature_2m_min\"", day)
                    .and_then(parse_decimal_tenths)
                else {
                    break;
                };
                let Some(code) =
                    json_array_number(daily, b"\"weather_code\"", day).and_then(parse_u8_ascii)
                else {
                    break;
                };
                if !(-1_000..=700).contains(&high) || !(-1_000..=700).contains(&low) {
                    break;
                }
                self.weather_forecast_highs[day] = high;
                self.weather_forecast_lows[day] = low;
                self.weather_forecast_codes[day] = code;
                self.weather_forecast_precipitation[day] =
                    json_array_number(daily, b"\"precipitation_probability_max\"", day)
                        .and_then(parse_u8_ascii)
                        .unwrap_or(0)
                        .min(100);
                self.weather_forecast_count += 1;
            }
        }
        self.weather_forecast_selected = self
            .weather_forecast_selected
            .min(self.weather_forecast_count.saturating_sub(1));
        self.weather_state = WeatherState::Ready;
        self.weather_notice = "Live data from Open-Meteo. Enter refreshes.";
        self.mark_persistent_change();
        true
    }

    pub fn weather_failed(&mut self, notice: &'static str) {
        self.weather_state = WeatherState::Error;
        self.weather_notice = notice;
    }

    pub const fn weather_temperature(&self) -> Option<i16> {
        if matches!(self.weather_state, WeatherState::Ready) {
            Some(self.weather_temperature_tenths)
        } else {
            None
        }
    }

    pub const fn weather_temperature_display(&self) -> Option<i16> {
        match self.weather_temperature() {
            Some(value) => Some(self.weather_display_temperature(value)),
            None => None,
        }
    }

    pub const fn weather_unit_label(&self) -> &'static str {
        if self.weather_fahrenheit {
            "F"
        } else {
            "C"
        }
    }

    pub const fn weather_forecast_days(&self) -> usize {
        self.weather_forecast_count
    }

    pub const fn weather_needs_location(&self) -> bool {
        self.weather_resolve_location
            || self.weather_latitude_len == 0
            || self.weather_longitude_len == 0
    }

    pub const fn weather_animating(&self) -> bool {
        self.active == 20
            && self.is_installed(20)
            && matches!(
                self.weather_state,
                WeatherState::Loading | WeatherState::Ready
            )
    }

    pub fn clear_weather_data(&mut self) {
        self.weather_state = WeatherState::Empty;
        self.weather_city.fill(0);
        self.weather_city_len = 0;
        self.weather_location.fill(0);
        self.weather_location_len = 0;
        self.weather_latitude.fill(0);
        self.weather_latitude_len = 0;
        self.weather_longitude.fill(0);
        self.weather_longitude_len = 0;
        self.weather_forecast_count = 0;
        self.weather_forecast_selected = 0;
        self.weather_resolve_location = true;
        self.weather_notice = "Weather location and cached forecast cleared.";
        self.clear_input();
        self.mark_persistent_change();
    }

    const fn weather_display_temperature(&self, celsius_tenths: i16) -> i16 {
        if self.weather_fahrenheit {
            ((celsius_tenths as i32 * 9) / 5 + 320) as i16
        } else {
            celsius_tenths
        }
    }

    pub const fn weather_condition(&self) -> &'static str {
        weather_condition(self.weather_code)
    }

    pub fn render_app(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        let width = rect.width as i32;
        if self.installed == 0 {
            framebuffer::text(x + 34, y + 96, "NO PACKAGE APPS INSTALLED", color::MUTED, 2);
            framebuffer::text(
                x + 34,
                y + 136,
                "Open Ayo, choose a package, then Install.",
                color::INK,
                1,
            );
            return;
        }
        let app = APPS[self.active];
        framebuffer::text(x + 28, y + 58, app.name, app.accent, 2);
        framebuffer::text(x + 30, y + 88, app.summary, color::MUTED, 1);
        self.render_tool(x + 28, y + 112, width - 56, rect.height as i32 - 142);
    }

    fn render_tool(&self, x: i32, y: i32, width: i32, height: i32) {
        let input = core::str::from_utf8(&self.input[..self.input_len]).unwrap_or("");
        match self.active {
            1 => {
                input_box(x + 16, y + 8, width - 32, input);
                framebuffer::text(
                    x + 18,
                    y + 58,
                    "ENTER adds  |  arrows select  |  SPACE completes  |  Backspace deletes",
                    color::MUTED,
                    1,
                );
                if self.task_count == 0 {
                    framebuffer::text(x + 18, y + 104, "Your task list is empty.", color::INK, 1);
                }
                for index in 0..self.task_count {
                    let row_y = y + 88 + index as i32 * 34;
                    let task = self.tasks[index];
                    if index == self.task_selected {
                        framebuffer::rect(x + 12, row_y - 7, width - 24, 28, 0x001C_2630);
                    }
                    framebuffer::text(
                        x + 20,
                        row_y,
                        if task.done { "[x]" } else { "[ ]" },
                        if task.done { color::GREEN } else { color::CYAN },
                        1,
                    );
                    let text =
                        core::str::from_utf8(&task.text[..task.length as usize]).unwrap_or("?");
                    framebuffer::text(
                        x + 54,
                        row_y,
                        text,
                        if task.done {
                            color::MUTED
                        } else {
                            color::WHITE
                        },
                        1,
                    );
                }
            }
            2 => {
                let now = hardware::rtc_time();
                let mut value = [b'0'; 8];
                let mut hour = now.hour;
                let mut suffix = "24 HOUR";
                if !self.clock_24h {
                    suffix = if hour >= 12 { "PM" } else { "AM" };
                    hour %= 12;
                    if hour == 0 {
                        hour = 12;
                    }
                }
                value[0] = b'0' + hour / 10;
                value[1] = b'0' + hour % 10;
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
                framebuffer::text(x + 76, y + 132, suffix, color::CYAN, 1);
                framebuffer::text(
                    x + 40,
                    y + 190,
                    "SPACE switches 12 / 24 hour time",
                    color::MUTED,
                    1,
                );
            }
            3 => {
                let now = hardware::rtc_time();
                let (year, month) = shift_month(now.year, now.month, self.calendar_offset);
                let days = days_in_month(year, month);
                let first = weekday_monday_zero(year, month, 1) as i32;
                framebuffer::text(x + 24, y + 20, "MONTH VIEW", color::CYAN, 2);
                framebuffer::text(x + 190, y + 24, month_name(month), color::WHITE, 1);
                number(x + 288, y + 24, year as u64, color::MUTED, 1);
                framebuffer::text(
                    x + 24,
                    y + 52,
                    "MON  TUE  WED  THU  FRI  SAT  SUN",
                    color::MUTED,
                    1,
                );
                for day in 1..=days {
                    let cell = first + day as i32 - 1;
                    let column = cell % 7;
                    let row = cell / 7;
                    number(
                        x + 24 + column * 48,
                        y + 84 + row * 34,
                        day as u64,
                        if day == now.day && month == now.month && year == now.year {
                            color::GREEN
                        } else {
                            color::INK
                        },
                        1,
                    );
                }
                framebuffer::text(
                    x + 24,
                    y + 286,
                    "LEFT / RIGHT changes month",
                    color::MUTED,
                    1,
                );
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
                    framebuffer::active_output_label(),
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
                metric(x + 20, y + 164, "APP FORMS", "30", color::PURPLE);
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
                let remaining = self.current_focus_seconds();
                draw_duration(x + 40, y + 38, remaining, color::WHITE, 3);
                framebuffer::text(
                    x + 42,
                    y + 104,
                    if remaining == 0 {
                        "SESSION COMPLETE"
                    } else if self.focus_running {
                        "FOCUS SESSION RUNNING"
                    } else {
                        "READY TO FOCUS"
                    },
                    if remaining == 0 {
                        color::GREEN
                    } else {
                        color::CYAN
                    },
                    1,
                );
                framebuffer::text(
                    x + 42,
                    y + 142,
                    "SPACE start/pause  R reset  +/- duration",
                    color::MUTED,
                    1,
                );
            }
            15 => {
                framebuffer::text(x + 24, y + 28, "MONOTONIC STOPWATCH", color::MUTED, 1);
                draw_duration(
                    x + 24,
                    y + 64,
                    self.stopwatch_seconds().min(u32::MAX as u64) as u32,
                    color::GREEN,
                    3,
                );
                framebuffer::text(x + 24, y + 126, "SPACE start/pause  R reset", color::INK, 1);
            }
            16 => {
                number(x + 90, y + 42, self.counter as u64, color::WHITE, 3);
                framebuffer::rect(x + 22, y + 132, 280, 90, color::CYAN);
                framebuffer::text(x + 102, y + 169, "ADD ONE", 0x0000_0000, 2);
            }
            20 => self.render_weather(x, y, width, height, input),
            21 => {
                input_box(x + 20, y + 12, width - 40, input);
                framebuffer::text(x + 24, y + 68, "SECONDS", color::MUTED, 1);
                let remaining = self.current_countdown_seconds();
                draw_duration(
                    x + 24,
                    y + 98,
                    remaining,
                    if remaining == 0 {
                        color::GREEN
                    } else {
                        color::WHITE
                    },
                    3,
                );
                framebuffer::text(
                    x + 24,
                    y + 166,
                    "ENTER starts typed seconds  SPACE pause/resume  R reset",
                    color::MUTED,
                    1,
                );
            }
            23 => {
                framebuffer::text(
                    x + 20,
                    y + 22,
                    "HARDWARE-ENTROPY PASSWORD",
                    color::PURPLE,
                    1,
                );
                let generated =
                    core::str::from_utf8(&self.generated[..self.generated_len]).unwrap_or("");
                input_box(
                    x + 20,
                    y + 54,
                    width - 40,
                    if generated.is_empty() {
                        "Press ENTER to generate"
                    } else {
                        generated
                    },
                );
                framebuffer::text(
                    x + 22,
                    y + 118,
                    "ENTER regenerates  +/- changes length",
                    color::MUTED,
                    1,
                );
                framebuffer::text(x + 22, y + 148, "Length", color::INK, 1);
                number(x + 86, y + 148, self.counter.max(16) as u64, color::CYAN, 1);
            }
            24 => {
                framebuffer::text(x + 26, y + 24, "FAIR D6 ROLL", color::MUTED, 1);
                number(x + 110, y + 62, self.counter.max(1) as u64, color::WHITE, 4);
                framebuffer::text(
                    x + 28,
                    y + 164,
                    "SPACE or ENTER rolls again",
                    color::CYAN,
                    1,
                );
            }
            29 => {
                framebuffer::text(x + 20, y + 24, "RFC 4122 VERSION 4", color::PURPLE, 1);
                let generated =
                    core::str::from_utf8(&self.generated[..self.generated_len]).unwrap_or("");
                input_box(
                    x + 20,
                    y + 62,
                    width - 40,
                    if generated.is_empty() {
                        "Press ENTER to generate"
                    } else {
                        generated
                    },
                );
                framebuffer::text(
                    x + 22,
                    y + 126,
                    "Backed by ExpOS storage entropy",
                    color::MUTED,
                    1,
                );
            }
            _ => {
                input_box(x + 20, y + 20, width - 40, input);
                self.render_text_result(x + 22, y + 82, input);
                framebuffer::text(
                    x + 22,
                    y + height - 24,
                    "Type to work  //  reopen another app from the main menu",
                    color::MUTED,
                    1,
                );
            }
        }
    }

    fn render_weather(&self, x: i32, y: i32, width: i32, height: i32, input: &str) {
        input_box(x + 12, y + 4, width - 24, input);
        let card_y = y + 58;
        let card_height = (height - 74).max(180);
        let (sky_top, sky_bottom) = if self.weather_is_day {
            (0x001B_6FB0, 0x0048_B5D8)
        } else {
            (0x0007_1536, 0x001A_315C)
        };
        for band in 0..8 {
            let blend = band as u32;
            let mixed = blend_rgb(sky_top, sky_bottom, blend, 7);
            framebuffer::rect(
                x + 12,
                card_y + band * card_height / 8,
                width - 24,
                (card_height + 7) / 8,
                mixed,
            );
        }
        framebuffer::outline(x + 12, card_y, width - 24, card_height, 0x007D_CAE8);

        let seconds = hardware::timestamp() / hardware::clock_info().tsc_hz.max(1);
        let bob = match seconds % 8 {
            0 | 7 => 0,
            1 | 6 => -2,
            2 | 5 => -4,
            _ => -6,
        };
        if self.weather_is_day {
            framebuffer::rounded_rect(x + width - 108, card_y + 24 + bob, 52, 52, 26, 0x00FF_D66B);
        } else {
            framebuffer::rounded_rect(x + width - 108, card_y + 24 + bob, 48, 48, 24, 0x00E8_ECF8);
            framebuffer::rounded_rect(x + width - 92, card_y + 14 + bob, 48, 48, 24, sky_top);
        }

        match self.weather_state {
            WeatherState::Ready => {
                let location =
                    core::str::from_utf8(&self.weather_location[..self.weather_location_len])
                        .unwrap_or("Current location");
                framebuffer::text(x + 32, card_y + 24, location, color::WHITE, 2);
                decimal_tenths(
                    x + 32,
                    card_y + 70,
                    self.weather_display_temperature(self.weather_temperature_tenths),
                    color::WHITE,
                    3,
                );
                framebuffer::text(
                    x + 142,
                    card_y + 88,
                    self.weather_unit_label(),
                    color::WHITE,
                    2,
                );
                framebuffer::text(
                    x + 34,
                    card_y + 126,
                    weather_condition(self.weather_code),
                    0x00F5_FBFF,
                    2,
                );
                draw_weather_icon(x + width - 152, card_y + 104 + bob, self.weather_code);
                if self.weather_forecast_count != 0 {
                    let forecast_y = card_y + card_height - 154;
                    let available = width - 56;
                    let forecast_width = (available / WEATHER_FORECAST_DAYS as i32).max(54);
                    for day in 0..self.weather_forecast_count {
                        let day_x = x + 28 + day as i32 * forecast_width;
                        framebuffer::rounded_rect(
                            day_x,
                            forecast_y,
                            forecast_width - 5,
                            62,
                            8,
                            if day == self.weather_forecast_selected {
                                0x0041_6680
                            } else {
                                0x0020_3040
                            },
                        );
                        framebuffer::text(
                            day_x + 8,
                            forecast_y + 8,
                            forecast_day_label(day),
                            if day == self.weather_forecast_selected {
                                color::WHITE
                            } else {
                                0x00D6_EAF5
                            },
                            1,
                        );
                        compact_high_low(
                            day_x + 8,
                            forecast_y + 27,
                            self.weather_display_temperature(self.weather_forecast_highs[day]),
                            self.weather_display_temperature(self.weather_forecast_lows[day]),
                        );
                        framebuffer::text(day_x + 8, forecast_y + 45, "RAIN", color::MUTED, 1);
                        number(
                            day_x + 42,
                            forecast_y + 45,
                            self.weather_forecast_precipitation[day] as u64,
                            color::CYAN,
                            1,
                        );
                    }
                }
                framebuffer::rounded_rect(
                    x + 28,
                    card_y + card_height - 78,
                    width - 56,
                    54,
                    10,
                    0x0020_3040,
                );
                framebuffer::text(x + 42, card_y + card_height - 62, "FEELS", 0x00D6_EAF5, 1);
                decimal_tenths(
                    x + 96,
                    card_y + card_height - 62,
                    self.weather_display_temperature(self.weather_apparent_tenths),
                    color::WHITE,
                    1,
                );
                framebuffer::text(x + 164, card_y + card_height - 62, "HUMID", 0x00D6_EAF5, 1);
                number(
                    x + 224,
                    card_y + card_height - 62,
                    self.weather_humidity as u64,
                    color::WHITE,
                    1,
                );
                framebuffer::text(x + 248, card_y + card_height - 62, "%", color::WHITE, 1);
                framebuffer::text(x + 286, card_y + card_height - 62, "WIND", 0x00D6_EAF5, 1);
                decimal_tenths(
                    x + 332,
                    card_y + card_height - 62,
                    self.weather_wind_tenths as i16,
                    color::WHITE,
                    1,
                );
                framebuffer::text(
                    x + 18,
                    y + height - 25,
                    "TAB C/F  LEFT/RIGHT forecast day  ENTER refresh/search",
                    0x00C7_DCE8,
                    1,
                );
            }
            WeatherState::Loading => {
                framebuffer::text(
                    x + 34,
                    card_y + 86,
                    "FETCHING LIVE WEATHER",
                    color::WHITE,
                    2,
                );
                for dot in 0..3 {
                    let active = dot <= (seconds % 3) as i32;
                    framebuffer::rounded_rect(
                        x + 38 + dot * 28,
                        card_y + 132,
                        12,
                        12,
                        6,
                        if active { color::WHITE } else { 0x006A_91AA },
                    );
                }
            }
            WeatherState::Error => {
                framebuffer::text(x + 34, card_y + 86, "WEATHER UNAVAILABLE", color::WHITE, 2);
                framebuffer::text(x + 36, card_y + 130, self.weather_notice, 0x00FF_D0D0, 1);
            }
            WeatherState::Empty => {
                framebuffer::text(x + 34, card_y + 84, "YOUR WEATHER, LIVE", color::WHITE, 2);
                framebuffer::text(
                    x + 36,
                    card_y + 126,
                    "Type a city above and press Enter.",
                    0x00E0_F3FA,
                    1,
                );
            }
        }
        framebuffer::text(
            x + 18,
            y + height - 10,
            self.weather_notice,
            color::MUTED,
            1,
        );
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
                let value = parse_decimal_tenths(input.as_bytes()).unwrap_or(0) as i64;
                let (label, converted) = match self.converter_mode {
                    1 => ("FAHRENHEIT", value.saturating_mul(9) / 5 + 320),
                    2 => ("KILOGRAMS", value.saturating_mul(4536) / 10_000),
                    3 => ("LITRES", value.saturating_mul(3785) / 1000),
                    _ => ("KILOMETRES", value.saturating_mul(1609) / 1000),
                };
                framebuffer::text(x, y, label, color::MUTED, 1);
                decimal_tenths(x, y + 30, converted as i16, color::GREEN, 2);
                framebuffer::text(
                    x,
                    y + 74,
                    match self.converter_mode {
                        1 => "Celsius -> Fahrenheit",
                        2 => "Pounds -> Kilograms",
                        3 => "Gallons -> Litres",
                        _ => "Miles -> Kilometres",
                    },
                    color::CYAN,
                    1,
                );
                framebuffer::text(x, y + 98, "SPACE changes conversion", color::MUTED, 1);
            }
            11 => {
                framebuffer::text(x, y, "DECIMAL", color::MUTED, 1);
                number(x + 90, y, parse_u64(input.as_bytes()), color::WHITE, 1);
                framebuffer::text(x, y + 34, "HEX", color::MUTED, 1);
                hex_number(x + 90, y + 34, parse_u64(input.as_bytes()), color::CYAN);
            }
            12 => {
                framebuffer::text(
                    x,
                    y,
                    match self.text_mode {
                        1 => "LOWERCASE",
                        2 => "TITLE CASE",
                        _ => "UPPERCASE",
                    },
                    color::MUTED,
                    1,
                );
                let mut px = x;
                let mut word_start = true;
                for byte in input.bytes().take(46) {
                    let transformed = match self.text_mode {
                        1 => byte.to_ascii_lowercase(),
                        2 if word_start => byte.to_ascii_uppercase(),
                        2 => byte.to_ascii_lowercase(),
                        _ => byte.to_ascii_uppercase(),
                    };
                    framebuffer::glyph(px, y + 28, transformed, color::CYAN, 2);
                    word_start = byte.is_ascii_whitespace();
                    px += 12;
                }
                framebuffer::text(x, y + 62, "SPACE changes transform", color::MUTED, 1);
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
                let valid = valid_json_structure(input.as_bytes());
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
            22 => {
                let (amount, percent, people) = parse_tip_input(input.as_bytes());
                let tip = amount.saturating_mul(percent as u64) / 100;
                let total = amount.saturating_add(tip);
                framebuffer::text(x, y, "FORMAT: amount tip% people", color::MUTED, 1);
                framebuffer::text(x, y + 32, "TIP", color::CYAN, 1);
                number(x + 80, y + 32, tip, color::WHITE, 1);
                framebuffer::text(x, y + 62, "TOTAL", color::CYAN, 1);
                number(x + 80, y + 62, total, color::WHITE, 1);
                framebuffer::text(x, y + 92, "EACH", color::CYAN, 1);
                number(
                    x + 80,
                    y + 92,
                    total / people.max(1) as u64,
                    color::GREEN,
                    1,
                );
            }
            25 => {
                framebuffer::text(x, y, "FORMAT: YYYY-MM-DD", color::MUTED, 1);
                let label = parse_date(input.as_bytes())
                    .filter(|(year, month, day)| valid_date(*year, *month, *day))
                    .map(|(year, month, day)| weekday_name(weekday_monday_zero(year, month, day)))
                    .unwrap_or("ENTER A VALID DATE");
                framebuffer::text(x, y + 38, label, color::CYAN, 2);
            }
            26 => {
                framebuffer::text(x, y, "FORMAT: first | second", color::MUTED, 1);
                let (left, right) = input.split_once('|').unwrap_or((input, ""));
                framebuffer::text(x, y + 34, left.trim(), color::CYAN, 1);
                framebuffer::text(x, y + 62, right.trim(), color::PURPLE, 1);
                let differences = left
                    .trim()
                    .bytes()
                    .zip(right.trim().bytes())
                    .filter(|(a, b)| a != b)
                    .count()
                    + left.trim().len().abs_diff(right.trim().len());
                framebuffer::text(x, y + 96, "DIFFERENT POSITIONS", color::MUTED, 1);
                number(x + 174, y + 96, differences as u64, color::GREEN, 1);
            }
            27 => {
                framebuffer::text(x, y, "FORMAT: 192.168.1.42/24", color::MUTED, 1);
                if let Some((address, prefix)) = parse_cidr(input.as_bytes()) {
                    let mask = if prefix == 0 {
                        0
                    } else {
                        u32::MAX << (32 - prefix)
                    };
                    let network = address & mask;
                    let broadcast = network | !mask;
                    framebuffer::text(x, y + 34, "NETWORK", color::CYAN, 1);
                    draw_ipv4(x + 102, y + 34, network, color::WHITE);
                    framebuffer::text(x, y + 64, "MASK", color::CYAN, 1);
                    draw_ipv4(x + 102, y + 64, mask, color::WHITE);
                    framebuffer::text(x, y + 94, "BROADCAST", color::CYAN, 1);
                    draw_ipv4(x + 102, y + 94, broadcast, color::GREEN);
                }
            }
            28 => {
                framebuffer::text(x, y, "INTERNATIONAL MORSE", color::MUTED, 1);
                draw_morse(x, y + 32, input.as_bytes());
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
    let mut term = 0_i64;
    let mut number = 0_i64;
    let mut multiply = b'+';
    let mut add = b'+';
    let mut have_number = false;
    for byte in bytes.iter().copied().chain(core::iter::once(b'+')) {
        if byte.is_ascii_digit() {
            number = number
                .saturating_mul(10)
                .saturating_add((byte - b'0') as i64);
            have_number = true;
            continue;
        }
        if !matches!(byte, b'+' | b'-' | b'*' | b'/') || !have_number {
            if matches!(byte, b'-') && !have_number && term == 0 {
                add = b'-';
            }
            continue;
        }
        term = match multiply {
            b'*' => term.saturating_mul(number),
            b'/' if number != 0 => term / number,
            b'/' => term,
            _ => number,
        };
        if matches!(byte, b'+' | b'-') {
            total = match add {
                b'-' => total.saturating_sub(term),
                _ => total.saturating_add(term),
            };
            add = byte;
            multiply = b'+';
            term = 0;
        } else {
            multiply = byte;
        }
        number = 0;
        have_number = false;
    }
    total
}

fn elapsed_seconds(started_at: u64) -> u64 {
    hardware::timestamp().saturating_sub(started_at) / hardware::clock_info().tsc_hz.max(1)
}

fn draw_duration(x: i32, y: i32, seconds: u32, ink: u32, scale: i32) {
    let hours = seconds / 3600;
    let minutes = (seconds / 60) % 60;
    let seconds = seconds % 60;
    let mut value = [b'0'; 8];
    value[0] = b'0' + (hours.min(99) / 10) as u8;
    value[1] = b'0' + (hours.min(99) % 10) as u8;
    value[2] = b':';
    value[3] = b'0' + (minutes / 10) as u8;
    value[4] = b'0' + (minutes % 10) as u8;
    value[5] = b':';
    value[6] = b'0' + (seconds / 10) as u8;
    value[7] = b'0' + (seconds % 10) as u8;
    framebuffer::text(
        x,
        y,
        core::str::from_utf8(&value).unwrap_or("00:00:00"),
        ink,
        scale,
    );
}

fn decimal_tenths(x: i32, y: i32, value: i16, ink: u32, scale: i32) {
    let negative = value < 0;
    let absolute = value.unsigned_abs();
    let mut digits = [b'0'; 10];
    let mut cursor = digits.len();
    cursor -= 1;
    digits[cursor] = b'0' + (absolute % 10) as u8;
    cursor -= 1;
    digits[cursor] = b'.';
    let mut whole = absolute / 10;
    loop {
        cursor -= 1;
        digits[cursor] = b'0' + (whole % 10) as u8;
        whole /= 10;
        if whole == 0 {
            break;
        }
    }
    if negative {
        cursor -= 1;
        digits[cursor] = b'-';
    }
    framebuffer::text(
        x,
        y,
        core::str::from_utf8(&digits[cursor..]).unwrap_or("0.0"),
        ink,
        scale,
    );
}

fn push_bytes(output: &mut [u8], offset: usize, bytes: &[u8]) -> Option<usize> {
    let end = offset.checked_add(bytes.len())?;
    if end > output.len() {
        return None;
    }
    output[offset..end].copy_from_slice(bytes);
    Some(end)
}

fn push_percent_encoded(output: &mut [u8], mut offset: usize, bytes: &[u8]) -> Option<usize> {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in bytes.iter().copied() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            offset = push_bytes(output, offset, &[byte])?;
        } else if byte == b' ' {
            offset = push_bytes(output, offset, b"%20")?;
        } else {
            offset = push_bytes(
                output,
                offset,
                &[b'%', HEX[(byte >> 4) as usize], HEX[(byte & 0xF) as usize]],
            )?;
        }
    }
    Some(offset)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    (!needle.is_empty() && needle.len() <= haystack.len())
        .then(|| {
            haystack
                .windows(needle.len())
                .position(|window| window == needle)
        })
        .flatten()
}

fn json_value_start(body: &[u8], key: &[u8]) -> Option<usize> {
    let key_offset = find_bytes(body, key)?;
    let colon = body[key_offset + key.len()..]
        .iter()
        .position(|byte| *byte == b':')?
        + key_offset
        + key.len();
    let mut start = colon + 1;
    while body.get(start).is_some_and(u8::is_ascii_whitespace) {
        start += 1;
    }
    Some(start)
}

fn json_number<'a>(body: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let start = json_value_start(body, key)?;
    let length = body[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.'))
        .count();
    (length != 0).then_some(&body[start..start + length])
}

fn json_string<'a>(body: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let start = json_value_start(body, key)?;
    if body.get(start) != Some(&b'"') {
        return None;
    }
    let content = start + 1;
    let length = body[content..]
        .iter()
        .position(|byte| *byte == b'"' || *byte == b'\\')?;
    Some(&body[content..content + length])
}

fn json_array_number<'a>(body: &'a [u8], key: &[u8], ordinal: usize) -> Option<&'a [u8]> {
    let mut cursor = json_value_start(body, key)?;
    if body.get(cursor) != Some(&b'[') {
        return None;
    }
    cursor += 1;
    for index in 0..=ordinal {
        while body
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b',')
        {
            cursor += 1;
        }
        let start = cursor;
        while body
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'+' | b'.'))
        {
            cursor += 1;
        }
        if start == cursor {
            return None;
        }
        if index == ordinal {
            return Some(&body[start..cursor]);
        }
        while body
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            cursor += 1;
        }
        if !matches!(body.get(cursor), Some(b',')) {
            return None;
        }
    }
    None
}

fn valid_weather_city(city: &[u8]) -> bool {
    !city.is_empty()
        && city.len() <= WEATHER_CITY_CAPACITY
        && city.iter().any(u8::is_ascii_alphanumeric)
        && city.iter().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'\'' | b',' | b'.' | b'-')
        })
}

fn coordinate_in_range(bytes: &[u8], maximum_tenths: i16) -> bool {
    parse_decimal_tenths(bytes).is_some_and(|value| value.unsigned_abs() <= maximum_tenths as u16)
}

fn parse_decimal_tenths(bytes: &[u8]) -> Option<i16> {
    if bytes.is_empty() {
        return None;
    }
    let negative = bytes[0] == b'-';
    let bytes = if negative || bytes[0] == b'+' {
        &bytes[1..]
    } else {
        bytes
    };
    let mut whole = 0_i32;
    let mut fraction = 0_i32;
    let mut after_decimal = false;
    let mut found = false;
    for byte in bytes.iter().copied() {
        if byte == b'.' && !after_decimal {
            after_decimal = true;
        } else if byte.is_ascii_digit() {
            found = true;
            if after_decimal {
                fraction = (byte - b'0') as i32;
                break;
            }
            whole = whole
                .saturating_mul(10)
                .saturating_add((byte - b'0') as i32);
        } else {
            break;
        }
    }
    if !found {
        return None;
    }
    let value = whole.saturating_mul(10).saturating_add(fraction);
    Some((if negative { -value } else { value }).clamp(i16::MIN as i32, i16::MAX as i32) as i16)
}

fn parse_u8_ascii(bytes: &[u8]) -> Option<u8> {
    let value = parse_u64(bytes);
    (value <= u8::MAX as u64).then_some(value as u8)
}

fn valid_json_structure(bytes: &[u8]) -> bool {
    let Some(start) = bytes.iter().position(|byte| !byte.is_ascii_whitespace()) else {
        return false;
    };
    let Some(end) = bytes.iter().rposition(|byte| !byte.is_ascii_whitespace()) else {
        return false;
    };
    let bytes = &bytes[start..=end];
    if bytes.len() < 2
        || !matches!(
            (bytes[0], bytes[bytes.len() - 1]),
            (b'{', b'}') | (b'[', b']')
        )
    {
        return false;
    }
    let mut stack = [0_u8; 16];
    let mut depth = 0;
    let mut in_string = false;
    let mut escaped = false;
    for byte in bytes.iter().copied() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' | b'[' if depth < stack.len() => {
                stack[depth] = byte;
                depth += 1;
            }
            b'}' | b']' if depth != 0 => {
                let expected = if byte == b'}' { b'{' } else { b'[' };
                if stack[depth - 1] != expected {
                    return false;
                }
                depth -= 1;
            }
            b'}' | b']' => return false,
            _ => {}
        }
    }
    depth == 0 && !in_string
}

fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    }
}

fn valid_date(year: u16, month: u8, day: u8) -> bool {
    (1970..=9999).contains(&year)
        && (1..=12).contains(&month)
        && day != 0
        && day <= days_in_month(year, month)
}

fn days_from_civil(year: u16, month: u8, day: u8) -> i64 {
    let mut year = year as i64;
    let month = month as i64;
    year -= i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_prime + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn weekday_monday_zero(year: u16, month: u8, day: u8) -> u8 {
    (days_from_civil(year, month, day) + 3).rem_euclid(7) as u8
}

fn weekday_name(day: u8) -> &'static str {
    [
        "MONDAY",
        "TUESDAY",
        "WEDNESDAY",
        "THURSDAY",
        "FRIDAY",
        "SATURDAY",
        "SUNDAY",
    ][day.min(6) as usize]
}

fn shift_month(year: u16, month: u8, offset: i16) -> (u16, u8) {
    let absolute = year as i32 * 12 + month as i32 - 1 + offset as i32;
    let shifted_year = absolute.div_euclid(12).clamp(1970, 9999) as u16;
    let shifted_month = absolute.rem_euclid(12) as u8 + 1;
    (shifted_year, shifted_month)
}

fn parse_date(bytes: &[u8]) -> Option<(u16, u8, u8)> {
    let text = core::str::from_utf8(bytes).ok()?;
    let mut parts = text.split('-');
    let year = parts.next()?.parse().ok()?;
    let month = parts.next()?.parse().ok()?;
    let day = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((year, month, day))
}

fn parse_tip_input(bytes: &[u8]) -> (u64, u8, u8) {
    let text = core::str::from_utf8(bytes).unwrap_or("");
    let mut values = text.split_ascii_whitespace();
    let amount = values
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let percent = values
        .next()
        .map(|value| value.trim_end_matches('%'))
        .and_then(|value| value.parse().ok())
        .unwrap_or(15);
    let people = values
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    (amount, percent, people)
}

fn parse_cidr(bytes: &[u8]) -> Option<(u32, u8)> {
    let text = core::str::from_utf8(bytes).ok()?;
    let (address, prefix) = text.split_once('/')?;
    let prefix = prefix.parse::<u8>().ok()?.min(32);
    let mut octets = address.split('.');
    let mut value = 0_u32;
    for _ in 0..4 {
        value = (value << 8) | octets.next()?.parse::<u8>().ok()? as u32;
    }
    octets.next().is_none().then_some((value, prefix))
}

fn draw_ipv4(x: i32, y: i32, address: u32, ink: u32) {
    let mut offset = x;
    for index in 0..4 {
        let octet = (address >> (24 - index * 8)) & 0xFF;
        number(offset, y, octet as u64, ink, 1);
        offset += if octet >= 100 {
            26
        } else if octet >= 10 {
            20
        } else {
            14
        };
        if index != 3 {
            framebuffer::text(offset - 6, y, ".", ink, 1);
        }
    }
}

fn morse(byte: u8) -> &'static str {
    match byte.to_ascii_uppercase() {
        b'A' => ".-",
        b'B' => "-...",
        b'C' => "-.-.",
        b'D' => "-..",
        b'E' => ".",
        b'F' => "..-.",
        b'G' => "--.",
        b'H' => "....",
        b'I' => "..",
        b'J' => ".---",
        b'K' => "-.-",
        b'L' => ".-..",
        b'M' => "--",
        b'N' => "-.",
        b'O' => "---",
        b'P' => ".--.",
        b'Q' => "--.-",
        b'R' => ".-.",
        b'S' => "...",
        b'T' => "-",
        b'U' => "..-",
        b'V' => "...-",
        b'W' => ".--",
        b'X' => "-..-",
        b'Y' => "-.--",
        b'Z' => "--..",
        b'0' => "-----",
        b'1' => ".----",
        b'2' => "..---",
        b'3' => "...--",
        b'4' => "....-",
        b'5' => ".....",
        b'6' => "-....",
        b'7' => "--...",
        b'8' => "---..",
        b'9' => "----.",
        b' ' => "/",
        _ => "?",
    }
}

fn draw_morse(x: i32, y: i32, input: &[u8]) {
    let mut x = x;
    let mut y = y;
    for byte in input.iter().copied().take(24) {
        let code = morse(byte);
        let width = framebuffer::text_advance(1) * (code.len() as i32 + 1);
        if x + width > framebuffer::width() as i32 - 30 {
            x = 28;
            y += 24;
        }
        framebuffer::text(x, y, code, color::CYAN, 1);
        x += width;
    }
}

const fn blend_rgb(first: u32, second: u32, part: u32, total: u32) -> u32 {
    let inverse = total - part;
    let red = (((first >> 16) & 0xFF) * inverse + ((second >> 16) & 0xFF) * part) / total;
    let green = (((first >> 8) & 0xFF) * inverse + ((second >> 8) & 0xFF) * part) / total;
    let blue = ((first & 0xFF) * inverse + (second & 0xFF) * part) / total;
    (red << 16) | (green << 8) | blue
}

const fn weather_condition(code: u8) -> &'static str {
    match code {
        0 => "CLEAR SKY",
        1 | 2 => "PARTLY CLOUDY",
        3 => "OVERCAST",
        45 | 48 => "FOG",
        51 | 53 | 55 | 56 | 57 => "DRIZZLE",
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => "RAIN",
        71 | 73 | 75 | 77 | 85 | 86 => "SNOW",
        95 | 96 | 99 => "THUNDERSTORM",
        _ => "MIXED CONDITIONS",
    }
}

const fn forecast_day_label(day: usize) -> &'static str {
    match day {
        0 => "TODAY",
        1 => "TOMORROW",
        2 => "+2 DAYS",
        3 => "+3 DAYS",
        _ => "+4 DAYS",
    }
}

fn compact_high_low(x: i32, y: i32, high_tenths: i16, low_tenths: i16) {
    framebuffer::text(x, y, "H", color::MUTED, 1);
    number_signed(x + 10, y, rounded_temperature(high_tenths), color::WHITE, 1);
    framebuffer::text(x + 32, y, "L", color::MUTED, 1);
    number_signed(x + 42, y, rounded_temperature(low_tenths), color::WHITE, 1);
}

const fn rounded_temperature(tenths: i16) -> i64 {
    if tenths >= 0 {
        ((tenths as i32 + 5) / 10) as i64
    } else {
        ((tenths as i32 - 5) / 10) as i64
    }
}

fn draw_weather_icon(x: i32, y: i32, code: u8) {
    if matches!(code, 0 | 1) {
        framebuffer::rounded_rect(x + 18, y, 54, 54, 27, 0x00FF_D66B);
        return;
    }
    let cloud = if matches!(code, 95 | 96 | 99) {
        0x0068_7180
    } else {
        0x00E8_F3F8
    };
    framebuffer::rounded_rect(x, y + 22, 54, 34, 17, cloud);
    framebuffer::rounded_rect(x + 24, y + 8, 48, 48, 24, cloud);
    framebuffer::rounded_rect(x + 54, y + 24, 42, 32, 16, cloud);
    if matches!(
        code,
        51 | 53 | 55 | 56 | 57 | 61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 | 95 | 96 | 99
    ) {
        for offset in [12, 38, 64] {
            framebuffer::line(x + offset, y + 68, x + offset - 7, y + 86, 0x0067_C8FF);
        }
    } else if matches!(code, 71 | 73 | 75 | 77 | 85 | 86) {
        for offset in [14, 42, 70] {
            framebuffer::text(x + offset, y + 68, "*", color::WHITE, 1);
        }
    }
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
    fn ecosystem_has_thirty_distinct_native_apps() {
        assert_eq!(APPS.len(), 30);
        assert_eq!(NativeApps::new().installed_count(), 0);
        for (index, app) in APPS.iter().enumerate() {
            assert!(!app.name.is_empty());
            assert_eq!(package_name(index), Some(app.name));
            assert!(!app.summary.is_empty());
            assert!(!APPS[..index].iter().any(|other| other.name == app.name));
        }
        assert_eq!(package_name(PACKAGE_COUNT), None);
    }

    #[test]
    fn calculator_and_hash_are_bounded_and_deterministic() {
        assert_eq!(eval_expression(b"12+3*2"), 18);
        assert_eq!(eval_expression(b"20-8/2"), 16);
        assert_eq!(eval_expression(b"12/0"), 12);
        assert_eq!(fnv1a(b"ExpOS"), fnv1a(b"ExpOS"));
        assert_ne!(fnv1a(b"ExpOS"), fnv1a(b"expos"));
    }

    #[test]
    fn durable_app_state_roundtrips_without_restoring_transient_input() {
        let mut apps = NativeApps::new();
        apps.selected = 6;
        apps.commit_install_selected();
        assert_eq!(apps.installed_count(), 1);
        assert_eq!(apps.installed_at(0), Some(6));
        assert!(apps.activate_installed(6));
        assert_eq!(apps.active_index(), 6);
        apps.active = 6;
        apps.pixels = 0x55AA;
        apps.counter = 42;
        apps.palette = 4;
        apps.weather_fahrenheit = true;
        apps.weather_forecast_count = 2;
        apps.weather_forecast_highs[..2].copy_from_slice(&[120, 145]);
        apps.weather_forecast_lows[..2].copy_from_slice(&[40, 55]);
        apps.input[..4].copy_from_slice(b"temp");
        apps.input_len = 4;
        let mut wire = [0_u8; NativeApps::STATE_CAPACITY];
        let length = apps.encode_state(&mut wire);
        let restored = NativeApps::restore_state(&wire[..length]);
        assert!(restored.is_installed(6));
        assert_eq!(restored.active, 6);
        assert_eq!(restored.pixels, 0x55AA);
        assert_eq!(restored.counter, 42);
        assert_eq!(restored.palette, 4);
        assert!(restored.weather_fahrenheit);
        assert_eq!(restored.weather_forecast_count, 2);
        assert_eq!(&restored.weather_forecast_highs[..2], &[120, 145]);
        assert_eq!(restored.input_len, 0);
    }

    #[test]
    fn weather_uses_bounded_open_meteo_requests_and_parses_live_shapes() {
        let mut apps = NativeApps::new();
        apps.weather_city[..11].copy_from_slice(b"New York,NY");
        apps.weather_city_len = 11;
        let mut url = [0_u8; 512];
        let length = apps.weather_search_url(&mut url).expect("bounded URL");
        let search_url = core::str::from_utf8(&url[..length]).unwrap();
        assert!(search_url.starts_with("https://geocoding-api.open-meteo.com/v1/search?"));
        assert!(search_url.ends_with("New%20York%2CNY"));

        let location = br#"{"results":[{"name":"New York","latitude":40.7128,"longitude":-74.006,"country":"United States"}]}"#;
        assert!(apps.apply_weather_location(location));
        let length = apps
            .weather_forecast_url(&mut url)
            .expect("bounded forecast URL");
        let url = core::str::from_utf8(&url[..length]).unwrap();
        assert!(url.starts_with(
            "https://api.open-meteo.com/v1/forecast?latitude=40.7128&longitude=-74.006"
        ));
        assert!(url.contains("temperature_2m"));
        assert!(url.contains("forecast_days=5"));

        let forecast = br#"{"latitude":40.71,"current_units":{"temperature_2m":"C","relative_humidity_2m":"%","wind_speed_10m":"km/h"},"current":{"time":"2026-10-06T12:00","temperature_2m":12.4,"relative_humidity_2m":71,"apparent_temperature":10.8,"is_day":1,"weather_code":61,"wind_speed_10m":15.2},"daily_units":{"temperature_2m_max":"C"},"daily":{"weather_code":[61,3,0,80,71],"temperature_2m_max":[15.2,16.1,17.0,13.4,8.2],"temperature_2m_min":[8.1,7.2,6.3,5.4,-1.2],"precipitation_probability_max":[80,20,0,65,40]}}"#;
        assert!(apps.apply_weather_forecast(forecast));
        assert_eq!(apps.weather_temperature(), Some(124));
        assert_eq!(apps.weather_condition(), "RAIN");
        assert_eq!(apps.weather_forecast_count, 5);
        assert_eq!(apps.weather_forecast_highs[0], 152);
        assert_eq!(apps.weather_forecast_lows[4], -12);
        assert_eq!(apps.weather_forecast_precipitation[3], 65);
        apps.weather_fahrenheit = true;
        assert_eq!(apps.weather_temperature_display(), Some(543));
        assert!(!valid_weather_city(b"../../etc/passwd"));
        assert!(valid_weather_city(b"Bishkek, KG"));
        assert!(!coordinate_in_range(b"181.0", 1_800));
    }

    #[test]
    fn utility_parsers_cover_dates_json_and_subnets() {
        assert_eq!(weekday_name(weekday_monday_zero(2026, 10, 6)), "TUESDAY");
        assert!(valid_json_structure(br#"{"a":[1,{"b":true}]}"#));
        assert!(!valid_json_structure(br#"{"a":[1,2}"#));
        assert_eq!(parse_cidr(b"192.168.1.42/24"), Some((0xC0A8_012A, 24)));
    }
}
