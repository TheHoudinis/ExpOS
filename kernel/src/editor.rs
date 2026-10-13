//! Shared bounded text engine for the optional Cliptick and Writer Forms.
//! Execution always uses the isolated service; UI modes never select CPL0.
use crate::{
    expfs_store, framebuffer as fb,
    input::{KEY_DOWN, KEY_LEFT, KEY_RIGHT, KEY_UP},
};
use alloc::{boxed::Box, vec::Vec};
use core::cell::Cell;
use expos_core::{Authority, CfcFin, Fin, FormKind, Lifecycle, Rect, Text};

const CAPACITY: usize = expfs_store::FORM_CONTENT_CAPACITY;
const BACKGROUND: u32 = 0x001e1e1e;
const PANEL: u32 = 0x00252526;
const INK: u32 = 0x00d4d4d4;
const MUTED: u32 = 0x0085888c;
const ACCENT: u32 = 0x000078d4;

pub struct Editor {
    code: [u8; CAPACITY],
    len: usize,
    cursor: usize,
    scroll: Cell<usize>,
    name: [u8; 32],
    name_len: usize,
    naming: bool,
    dirty: bool,
    files: Vec<Text>,
    output: Vec<u8>,
    status: &'static str,
    settings: bool,
    operator: bool,
    local: bool,
    light: bool,
    line_numbers: bool,
    cfc: CfcFin,
    ide: bool,
}

impl Editor {
    pub fn new(cfc: CfcFin, ide: bool) -> Box<Self> {
        let mut editor = Box::new(Self {
            code: [0; CAPACITY],
            len: 0,
            cursor: 0,
            scroll: Cell::new(0),
            name: [0; 32],
            name_len: 0,
            naming: false,
            dirty: false,
            files: Vec::new(),
            output: Vec::new(),
            status: "Ready",
            settings: false,
            operator: false,
            local: false,
            light: false,
            line_numbers: true,
            cfc,
            ide,
        });
        editor.reset();
        if let Some((bytes, len)) = expfs_store::load_data_form(cfc, "Cliptick.state") {
            if len == 8 && &bytes[..4] == b"CTK1" && bytes[4..8].iter().all(|b| *b <= 1) {
                editor.local = bytes[4] != 0;
                editor.light = bytes[5] != 0;
                editor.line_numbers = bytes[6] != 0;
                // Operator intent is deliberately session-only.
            }
        }
        editor.refresh_files();
        editor
    }

    fn filename(&self) -> &str {
        core::str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }
    fn reset(&mut self) {
        self.code.fill(0);
        self.len = 0;
        self.cursor = 0;
        self.scroll.set(0);
        self.name.fill(0);
        let name: &[u8] = if self.ide {
            b"main.py"
        } else {
            b"Untitled.txt"
        };
        self.name[..name.len()].copy_from_slice(name);
        self.name_len = name.len();
        self.dirty = false;
        self.naming = false;
        self.output.clear();
        self.status = "Ready";
    }
    pub fn refresh_files(&mut self) {
        self.files.clear();
        if let Some(snapshot) = expfs_store::current_snapshot(self.cfc) {
            for stored in snapshot.forms.iter().flatten() {
                if stored.form.kind == FormKind::Data
                    && stored.form.lifecycle == Lifecycle::Active
                    && expfs_store::is_text_name(stored.form.name.as_str())
                {
                    self.files.push(stored.form.name);
                }
            }
        }
    }
    fn open(&mut self, name: &str) {
        if self.dirty {
            self.status = "Save changes before opening another file";
            return;
        }
        if let Some((bytes, len)) = expfs_store::load_data_form(self.cfc, name) {
            if len > CAPACITY || !bytes[..len].is_ascii() || name.len() > self.name.len() {
                self.status = "File is not supported ASCII text";
                return;
            }
            self.code = bytes;
            self.len = len;
            self.cursor = 0;
            self.scroll.set(0);
            self.name.fill(0);
            self.name[..name.len()].copy_from_slice(name.as_bytes());
            self.name_len = name.len();
            self.naming = false;
            self.status = "Opened";
            self.output.clear();
            crate::slog!(
                "EXPOS_EDITOR_OPEN app={} name={} bytes={}\r\n",
                self.app(),
                name,
                len
            );
        } else {
            self.status = "File not found";
        }
    }
    fn app(&self) -> &'static str {
        if self.ide {
            "Cliptick"
        } else {
            "Writer"
        }
    }
    fn save(&mut self, authority: Authority) -> bool {
        if authority == Authority::Guest {
            self.status = "Sign in to save files";
            return false;
        }
        if !expfs_store::is_text_name(self.filename()) {
            self.status = "Use .txt .sys .py .rs .go .c or .h";
            return false;
        }
        match expfs_store::save_text_form(self.cfc, self.filename(), &self.code[..self.len]) {
            Ok((_, revision)) => {
                self.dirty = false;
                self.status = "Saved";
                crate::slog!(
                    "EXPOS_EDITOR_SAVE app={} name={} revision={} bytes={}\r\n",
                    self.app(),
                    self.filename(),
                    revision,
                    self.len
                );
                self.refresh_files();
                true
            }
            Err(error) => {
                self.status = error.message();
                false
            }
        }
    }
    fn persist_settings(&mut self, authority: Authority) {
        if authority == Authority::Guest {
            self.status = "Settings apply to this session";
            return;
        }
        let bytes = [
            b'C',
            b'T',
            b'K',
            b'1',
            self.local as u8,
            self.light as u8,
            self.line_numbers as u8,
            0,
        ];
        if let Err(error) = expfs_store::save_data_form(self.cfc, "Cliptick.state", &bytes) {
            self.status = error.message();
        } else {
            self.status = "Settings saved";
        }
    }
    fn run(&mut self, authority: Authority, execute_allowed: bool) {
        if !self.ide {
            return;
        }
        self.output.clear();
        if !execute_allowed || authority == Authority::Guest {
            self.status = "Execution permission denied";
            return;
        }
        if self.operator && authority != Authority::Operator {
            self.status = "An Operator session is required";
            return;
        }
        if !self.filename().ends_with(".py") {
            self.run_compiled(authority);
            return;
        }
        // Local runs use the saved CFC document. Sandbox runs use the buffer
        // and never write a source Form. Neither grants filesystem/network imports.
        if self.local && !self.save(authority) {
            return;
        }
        let source = if self.local {
            let Some((bytes, len)) = expfs_store::load_data_form(self.cfc, self.filename()) else {
                self.status = "Saved source could not be read";
                return;
            };
            bytes[..len].to_vec()
        } else {
            self.code[..self.len].to_vec()
        };
        crate::slog!(
            "EXPOS_CLIPTICK_RUN language=python location={} authority={} bytes={}\r\n",
            if self.local { "local" } else { "sandbox" },
            if self.operator { "operator" } else { "normal" },
            source.len()
        );
        match crate::python::run(core::str::from_utf8(&source).unwrap_or(""), self.cfc) {
            Ok(report) => {
                self.output
                    .extend_from_slice(&report.output.bytes[..report.output.length]);
                self.status = match report.result {
                    0 => "Finished",
                    1 => "Execution budget exceeded",
                    2 => "Output budget exceeded",
                    _ => "Python exception",
                };
                crate::slog!(
                    "EXPOS_CLIPTICK_EXIT result={} ticks={} bytes={}\r\n",
                    report.result,
                    report.ticks,
                    report.output.length
                );
            }
            Err(error) => {
                self.status = "Isolated worker stopped";
                use core::fmt::Write;
                let mut message = alloc::string::String::new();
                let _ = write!(message, "Worker error: {:?}", error);
                self.output.extend_from_slice(message.as_bytes());
            }
        }
    }
    fn run_compiled(&mut self, authority: Authority) {
        let filename = Text::new(self.filename()).expect("editor file name is bounded");
        let Some((stem, extension)) = filename.as_str().rsplit_once('.') else {
            self.status = "Select a source file";
            return;
        };
        let Some(language) = expos_core::NativeLanguage::from_extension(extension) else {
            self.status = "This file type is not executable";
            return;
        };
        if self.local && !self.save(authority) {
            return;
        }
        let source = core::str::from_utf8(&self.code[..self.len]).unwrap_or("");
        let image = match expos_core::compile_native_source(language, source) {
            Ok(image) => image,
            Err(error) => {
                self.status = error.message();
                self.output.extend_from_slice(b"Compile error: ");
                self.output.extend_from_slice(error.message().as_bytes());
                self.output.push(b'\n');
                return;
            }
        };
        let fin = if self.local {
            match expfs_store::save_executable_form(self.cfc, stem, image.bytes()) {
                Ok((fin, revision)) => {
                    use core::fmt::Write;
                    let mut line = alloc::string::String::new();
                    let _ = writeln!(
                        line,
                        "Compiled {} as EXF1 revision {} on this device.",
                        stem, revision
                    );
                    self.output.extend_from_slice(line.as_bytes());
                    fin
                }
                Err(error) => {
                    self.status = error.message();
                    return;
                }
            }
        } else {
            // An unsaved sandbox build gets a deterministic, session-local FIN;
            // it is never inserted into ExpFS or another CFC.
            let hash = stem.bytes().fold(5381_u64, |hash, byte| {
                hash.wrapping_mul(33).wrapping_add(byte as u64)
            });
            Fin::from_u128(0x434F_4D50_494C_4544_0000_0000_0000_0000 | hash as u128)
        };
        crate::slog!(
            "EXPOS_CLIPTICK_COMPILE language={:?} storage={} source_bytes={} image_bytes={} fin={}\r\n",
            language,
            if self.local { "expfs" } else { "sandbox" },
            self.len,
            image.bytes().len(),
            fin
        );
        self.output.extend_from_slice(
            b"Running the on-device build in an isolated Ring 3 address space.\n",
        );
        let authority = if self.operator {
            Authority::Operator
        } else {
            Authority::Power
        };
        crate::slog!(
            "EXPOS_CLIPTICK_NATIVE name={} authority={:?} bytes={}\r\n",
            stem,
            authority,
            image.bytes().len()
        );
        match crate::service::run_image(image.bytes(), self.cfc, fin, authority) {
            Ok(report) => {
                self.output
                    .extend_from_slice(&report.output.bytes[..report.output.length]);
                use core::fmt::Write;
                let mut result = alloc::string::String::new();
                let _ = write!(result, "\nExit result: {}\n", report.result);
                self.output.extend_from_slice(result.as_bytes());
                self.status = "On-device build exited";
                crate::slog!(
                    "EXPOS_CLIPTICK_NATIVE_EXIT result={} ticks={} bytes={}\r\n",
                    report.result,
                    report.ticks,
                    report.output.length
                );
            }
            Err(error) => {
                self.status = "Native Form stopped";
                crate::slog!("EXPOS_CLIPTICK_NATIVE_ERROR error={:?}\r\n", error);
            }
        }
    }

    fn insert(&mut self, bytes: &[u8]) {
        if self.len + bytes.len() > CAPACITY {
            self.status = "File limit: 4096 bytes";
            return;
        }
        self.code
            .copy_within(self.cursor..self.len, self.cursor + bytes.len());
        self.code[self.cursor..self.cursor + bytes.len()].copy_from_slice(bytes);
        self.cursor += bytes.len();
        self.len += bytes.len();
        self.dirty = true;
        self.status = "Modified";
    }
    pub fn key(&mut self, key: u8, authority: Authority, execute_allowed: bool) -> bool {
        // Desktop shortcuts must still reach the window manager.
        if key >= 0x90 || key == 27 {
            return false;
        }
        if key == 19 {
            self.save(authority);
            return true;
        }
        if key == 18 && self.ide {
            self.run(authority, execute_allowed);
            return true;
        }
        if self.settings {
            return true;
        }
        if self.naming {
            match key {
                b'\r' | b'\n' => self.naming = false,
                8 | 127 if self.name_len > 0 => {
                    self.name_len -= 1;
                    self.dirty = true;
                }
                byte if (byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
                    && self.name_len < self.name.len() =>
                {
                    self.name[self.name_len] = byte;
                    self.name_len += 1;
                    self.dirty = true;
                }
                _ => {}
            }
            return true;
        }
        match key {
            KEY_LEFT => self.cursor = self.cursor.saturating_sub(1),
            KEY_RIGHT => self.cursor = (self.cursor + 1).min(self.len),
            KEY_UP => self.vertical(-1),
            KEY_DOWN => self.vertical(1),
            8 | 127 if self.cursor > 0 => {
                self.code
                    .copy_within(self.cursor..self.len, self.cursor - 1);
                self.cursor -= 1;
                self.len -= 1;
                self.dirty = true;
                self.status = "Modified";
            }
            b'\r' | b'\n' => self.insert(b"\n"),
            b'\t' => self.insert(b"    "),
            byte if byte.is_ascii_graphic() || byte == b' ' => self.insert(&[byte]),
            _ => {}
        }
        true
    }
    fn vertical(&mut self, direction: i32) {
        let start = self.code[..self.cursor]
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |n| n + 1);
        let col = self.cursor - start;
        if direction < 0 && start > 0 {
            let previous = self.code[..start - 1]
                .iter()
                .rposition(|b| *b == b'\n')
                .map_or(0, |n| n + 1);
            self.cursor = (previous + col).min(start - 1);
        } else if direction > 0 {
            if let Some(end) = self.code[self.cursor..self.len]
                .iter()
                .position(|b| *b == b'\n')
            {
                let next = self.cursor + end + 1;
                let length = self.code[next..self.len]
                    .iter()
                    .position(|b| *b == b'\n')
                    .unwrap_or(self.len - next);
                self.cursor = next + col.min(length);
            }
        }
    }
    pub fn click(
        &mut self,
        x: i16,
        y: i16,
        rect: Rect,
        authority: Authority,
        execute_allowed: bool,
    ) -> bool {
        let x = x as i32 - rect.x as i32;
        let y = y as i32 - rect.y as i32;
        if (42..72).contains(&y) {
            match (x - 12) / 72 {
                0 => {
                    if self.dirty {
                        self.status = "Save changes before creating a new file";
                    } else {
                        self.reset();
                    }
                }
                1 => {
                    self.refresh_files();
                    let name = alloc::string::String::from(self.filename());
                    self.open(&name);
                }
                2 => {
                    self.save(authority);
                }
                3 if self.ide => self.run(authority, execute_allowed),
                4 => self.settings = !self.settings,
                _ => {}
            }
            return true;
        }
        if self.settings {
            if x >= 200 && (110..278).contains(&y) {
                match (y - 110) / 42 {
                    0 if y >= 110 && self.ide => {
                        if !self.operator && authority != Authority::Operator {
                            self.status = "Sign in as Operator to select this mode";
                        } else {
                            self.operator = !self.operator;
                        }
                    }
                    1 if self.ide => {
                        self.local = !self.local;
                        self.persist_settings(authority);
                    }
                    2 => {
                        self.light = !self.light;
                        self.persist_settings(authority);
                    }
                    3 => {
                        self.line_numbers = !self.line_numbers;
                        self.persist_settings(authority);
                    }
                    _ => {}
                }
            }
            return true;
        }
        if (78..106).contains(&y) && x > 180 {
            self.naming = true;
            return true;
        }
        if x < 180 && y >= 114 {
            let index = ((y - 114) / 24) as usize;
            if let Some(name) = self.files.get(index).copied() {
                self.open(name.as_str());
            }
            return true;
        }
        self.naming = false;
        if x >= 230 && y >= 114 {
            let line = self.scroll.get() + ((y - 114) / 20) as usize;
            let mut start = 0;
            for _ in 0..line {
                let Some(next) = self.code[start..self.len].iter().position(|b| *b == b'\n') else {
                    break;
                };
                start += next + 1;
            }
            let end = self.code[start..self.len]
                .iter()
                .position(|b| *b == b'\n')
                .map_or(self.len, |n| start + n);
            self.cursor = (start + ((x - 230) / fb::text_advance(1)).max(0) as usize).min(end);
        }
        true
    }

    pub fn render(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        let w = rect.width as i32;
        let h = rect.height as i32;
        let bg = if self.light { 0x00fafafa } else { BACKGROUND };
        let ink = if self.light { 0x00303030 } else { INK };
        fb::rect(x + 1, y + 36, w - 2, h - 37, bg);
        fb::rect(x + 1, y + 36, w - 2, 38, PANEL);
        for (i, label) in [
            "New",
            "Open",
            "Save",
            if self.ide { "Run" } else { "" },
            "Settings",
        ]
        .iter()
        .enumerate()
        {
            fb::text(
                x + 18 + i as i32 * 72,
                y + 51,
                label,
                if i == 3 { 0x0089d185 } else { INK },
                1,
            );
        }
        fb::rect(x + 1, y + 74, 35, h - 96, 0x00333333);
        fb::text(x + 12, y + 90, "F", INK, 1);
        fb::rect(x + 36, y + 74, 144, h - 96, PANEL);
        fb::text(x + 48, y + 90, "FILES", MUTED, 1);
        for (i, file) in self.files.iter().enumerate() {
            let yy = y + 118 + i as i32 * 24;
            if yy + 16 >= y + h - 24 {
                break;
            }
            clipped(
                x + 48,
                yy,
                122,
                file.as_str(),
                if file.as_str() == self.filename() {
                    0x0075beff
                } else {
                    INK
                },
            );
        }
        if self.settings {
            let modes = [
                if self.operator {
                    "Privileges: Operator"
                } else {
                    "Privileges: Normal"
                },
                if self.local {
                    "Run source: Local saved file"
                } else {
                    "Run source: Sandbox buffer"
                },
                if self.light {
                    "Theme: Light"
                } else {
                    "Theme: Dark"
                },
                if self.line_numbers {
                    "Line numbers: On"
                } else {
                    "Line numbers: Off"
                },
            ];
            for (i, label) in modes.iter().enumerate() {
                if !self.ide && i < 2 {
                    continue;
                }
                clipped(x + 206, y + 120 + i as i32 * 42, w - 222, label, ink);
            }
            if self.ide {
                clipped(
                    x + 206,
                    y + 300,
                    w - 220,
                    "Python runs in Ring 3 in both modes.",
                    MUTED,
                );
                clipped(
                    x + 206,
                    y + 322,
                    w - 220,
                    "Local saves first; sandbox runs without saving.",
                    MUTED,
                );
                clipped(
                    x + 206,
                    y + 344,
                    w - 220,
                    "Filesystem and network imports are unavailable.",
                    MUTED,
                );
            }
        } else {
            fb::rect(x + 180, y + 74, w - 181, 32, PANEL);
            clipped(
                x + 200,
                y + 86,
                w - 232,
                self.filename(),
                if self.naming { 0x0075beff } else { INK },
            );
            if self.dirty {
                fb::text(x + w - 24, y + 86, "*", INK, 1);
            }
            let output_height = if self.ide { (h / 4).clamp(80, 140) } else { 0 };
            let bottom = h - 30 - output_height;
            let rows = ((bottom - 114) / 20).max(1) as usize;
            let cursor_line = self.code[..self.cursor]
                .iter()
                .filter(|b| **b == b'\n')
                .count();
            let scroll = self
                .scroll
                .get()
                .min(cursor_line)
                .max(cursor_line.saturating_sub(rows - 1));
            self.scroll.set(scroll);
            let mut offset = 0;
            for (line, bytes) in self.code[..self.len].split(|b| *b == b'\n').enumerate() {
                if line >= scroll && line < scroll + rows {
                    let yy = y + 118 + (line - scroll) as i32 * 20;
                    if self.line_numbers {
                        number(x + 190, yy, line + 1, MUTED);
                    }
                    let line_text = core::str::from_utf8(bytes).unwrap_or("");
                    let shade = if self.ide
                        && (line_text.trim_start().starts_with('#')
                            || line_text.trim_start().starts_with("//"))
                    {
                        0x006a9955
                    } else if self.ide
                        && ["def ", "fn ", "func ", "import ", "use ", "return "]
                            .iter()
                            .any(|word| line_text.trim_start().starts_with(word))
                    {
                        0x00569cd6
                    } else {
                        ink
                    };
                    // Horizontal scrolling follows the insertion point on its line.
                    let columns = ((w - 245) / fb::text_advance(1)).max(1) as usize;
                    let column = self.cursor.saturating_sub(offset);
                    let left = if line == cursor_line {
                        column.saturating_sub(columns - 1)
                    } else {
                        0
                    };
                    clipped(
                        x + 230,
                        yy,
                        w - 245,
                        &line_text[left.min(line_text.len())..],
                        shade,
                    );
                    if line == cursor_line && !self.naming {
                        fb::rect(
                            x + 230 + (column - left) as i32 * fb::text_advance(1),
                            yy - 2,
                            2,
                            16,
                            ACCENT,
                        );
                    }
                }
                offset += bytes.len() + 1;
            }
            if self.ide {
                let top = y + h - 26 - output_height;
                fb::rect(x + 181, top, w - 182, output_height, PANEL);
                fb::text(x + 198, top + 10, "OUTPUT", MUTED, 1);
                let max_lines = ((output_height - 30) / 18).max(1) as usize;
                let lines = self.output.split(|b| *b == b'\n');
                let skip = lines.clone().count().saturating_sub(max_lines);
                for (i, line) in lines.skip(skip).take(max_lines).enumerate() {
                    let text = core::str::from_utf8(line).unwrap_or("[non-UTF8 output]");
                    clipped(x + 198, top + 32 + i as i32 * 18, w - 214, text, INK);
                }
            }
        }
        fb::rect(x + 1, y + h - 24, w - 2, 23, ACCENT);
        clipped(x + 12, y + h - 16, w - 165, self.status, 0x00ffffff);
        number(x + w - 134, y + h - 16, self.len, 0x00ffffff);
        fb::text(x + w - 112, y + h - 16, "/4096 bytes", 0x00ffffff, 1);
    }
}
fn clipped(x: i32, y: i32, width: i32, text: &str, color: u32) {
    if width <= 0 {
        return;
    }
    let mut end = text
        .len()
        .min((width / fb::text_advance(1)).max(0) as usize);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    fb::text(x, y, &text[..end], color, 1);
}
fn number(x: i32, y: i32, value: usize, color: u32) {
    let mut bytes = [b'0'; 20];
    let mut n = value;
    let mut pos = 19;
    bytes[pos] += (n % 10) as u8;
    while n >= 10 && pos > 0 {
        n /= 10;
        pos -= 1;
        bytes[pos] += (n % 10) as u8;
    }
    fb::text(
        x,
        y,
        core::str::from_utf8(&bytes[pos..]).unwrap_or(""),
        color,
        1,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn insert_move_delete_and_bound_source() {
        let mut editor = Editor::new(CfcFin::from_u128(1), true);
        editor.insert(b"ac\nlong");
        editor.cursor = 1;
        editor.insert(b"b");
        assert_eq!(&editor.code[..editor.len], b"abc\nlong");
        editor.vertical(1);
        assert_eq!(editor.cursor, 6);
        editor.key(8, Authority::Power, false);
        assert_eq!(&editor.code[..editor.len], b"abc\nlng");
        editor.insert(&[b'x'; CAPACITY]);
        assert_eq!(editor.len, 7);
    }
    #[test]
    fn privilege_toggle_does_not_elevate_power() {
        let mut editor = Editor::new(CfcFin::from_u128(1), true);
        editor.settings = true;
        let rect = Rect {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        editor.click(220, 120, rect, Authority::Power, true);
        assert!(!editor.operator);
        editor.click(220, 120, rect, Authority::Operator, true);
        assert!(editor.operator);
        editor.run(Authority::Power, true);
        assert_eq!(editor.status, "An Operator session is required");
    }
}
