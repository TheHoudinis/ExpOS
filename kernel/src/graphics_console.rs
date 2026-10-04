//! Small graphical console used after a native UEFI handoff.
//!
//! UEFI leaves QEMU in GOP graphics mode, so writes to the legacy 0xb8000 VGA
//! text buffer is not visible. Genesis, the unlock path, login and shell mirror
//! the existing bounded text interface into the firmware framebuffer; serial
//! and VGA remain diagnostic fallbacks.

use crate::{framebuffer, sync::SpinMutex};
use core::fmt;

const COLS: usize = 74;
const ROWS: usize = 45;
const CELL_WIDTH: i32 = 8;
const CELL_HEIGHT: i32 = 9;
const ORIGIN_X: i32 = 24;
const ORIGIN_Y: i32 = 66;
const PANEL: u32 = 0x000D_1118;
const INK: u32 = 0x00E5_EAE7;
const BACKGROUND: u32 = 0x0008_0B10;
const HEADER: u32 = 0x0012_1820;
const MUTED: u32 = 0x0088_9690;
const ACCENT: u32 = 0x006F_BF9A;

struct Console {
    enabled: bool,
    col: usize,
    row: usize,
    cells: [[char; COLS]; ROWS],
}

impl Console {
    const fn new() -> Self {
        Self {
            enabled: false,
            col: 0,
            row: 0,
            cells: [[' '; COLS]; ROWS],
        }
    }

    fn reset(&mut self) {
        self.col = 0;
        self.row = 0;
        self.cells = [[' '; COLS]; ROWS];
    }

    fn draw_chrome(&self, title: &str, subtitle: &str) {
        let width = framebuffer::width() as i32;
        let height = framebuffer::height() as i32;
        framebuffer::clear(BACKGROUND);
        framebuffer::rect(0, 0, width, 54, HEADER);
        framebuffer::rect(0, 52, width, 2, ACCENT);
        framebuffer::text(24, 14, title, framebuffer::color::WHITE, 2);
        framebuffer::text(236, 20, subtitle, MUTED, 1);
        framebuffer::rect(14, 58, width - 28, height - 68, PANEL);
        framebuffer::outline(14, 58, width - 28, height - 68, framebuffer::color::BORDER);
    }

    fn draw_cell(&self, col: usize, row: usize) {
        let x = ORIGIN_X + col as i32 * CELL_WIDTH;
        let y = ORIGIN_Y + row as i32 * CELL_HEIGHT;
        framebuffer::rect(x, y, CELL_WIDTH, CELL_HEIGHT, PANEL);
        let character = self.cells[row][col];
        if character != ' ' {
            framebuffer::glyph_char(x, y, character, INK, 1);
        }
    }

    fn cell_damage(col: usize, row: usize) -> framebuffer::DamageRegion {
        framebuffer::DamageRegion::new(
            ORIGIN_X + col as i32 * CELL_WIDTH,
            ORIGIN_Y + row as i32 * CELL_HEIGHT,
            CELL_WIDTH,
            CELL_HEIGHT,
        )
    }

    fn body_damage() -> framebuffer::DamageRegion {
        framebuffer::DamageRegion::new(
            ORIGIN_X,
            ORIGIN_Y,
            COLS as i32 * CELL_WIDTH,
            ROWS as i32 * CELL_HEIGHT,
        )
    }

    fn redraw_body(&self) {
        framebuffer::rect(
            ORIGIN_X,
            ORIGIN_Y,
            COLS as i32 * CELL_WIDTH,
            ROWS as i32 * CELL_HEIGHT,
            PANEL,
        );
        for row in 0..ROWS {
            for col in 0..COLS {
                if self.cells[row][col] != ' ' {
                    self.draw_cell(col, row);
                }
            }
        }
    }

    fn new_line(&mut self) -> bool {
        self.col = 0;
        if self.row + 1 < ROWS {
            self.row += 1;
            return false;
        }
        for row in 1..ROWS {
            self.cells[row - 1] = self.cells[row];
        }
        self.cells[ROWS - 1] = [' '; COLS];
        self.redraw_body();
        true
    }

    fn write_character(&mut self, character: char) -> Option<framebuffer::DamageRegion> {
        match character {
            '\n' => self.new_line().then(Self::body_damage),
            '\u{8}' if self.col != 0 => {
                self.col -= 1;
                self.cells[self.row][self.col] = ' ';
                self.draw_cell(self.col, self.row);
                Some(Self::cell_damage(self.col, self.row))
            }
            '\u{8}' => None,
            character if !character.is_control() => {
                let scrolled = self.col >= COLS && self.new_line();
                self.cells[self.row][self.col] = character;
                self.draw_cell(self.col, self.row);
                let cell = Self::cell_damage(self.col, self.row);
                self.col += 1;
                Some(if scrolled { Self::body_damage() } else { cell })
            }
            _ => None,
        }
    }
}

impl fmt::Write for Console {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if !self.enabled {
            return Ok(());
        }
        let mut damage: Option<framebuffer::DamageRegion> = None;
        let rtl = value
            .chars()
            .any(|character| matches!(character as u32, 0x0590..=0x05FF));
        if rtl {
            for character in value.chars().rev() {
                if let Some(changed) = self.write_character(character) {
                    damage = Some(damage.map_or(changed, |current| current.union(changed)));
                }
            }
        } else {
            for character in value.chars() {
                if let Some(changed) = self.write_character(character) {
                    damage = Some(damage.map_or(changed, |current| current.union(changed)));
                }
            }
        }
        if let Some(damage) = damage {
            let _ = framebuffer::present_damage(false, &[damage]);
        }
        Ok(())
    }
}

static CONSOLE: SpinMutex<Console> = SpinMutex::new(Console::new());

fn enable(title: &str, subtitle: &str) -> bool {
    let _ = framebuffer::request_mode(framebuffer::DisplayMode::P480);
    if !framebuffer::enter() {
        return false;
    }
    let mut console = CONSOLE.lock();
    console.reset();
    console.enabled = true;
    console.draw_chrome(title, subtitle);
    framebuffer::present(false)
}

/// Activate the visible installer console. Serial and VGA remain diagnostic
/// mirrors if the safe QEMU framebuffer is unavailable.
#[cfg(feature = "genesis-installer")]
pub fn enable_genesis() -> bool {
    enable("ExpOS Genesis Engine", "VERIFIED CFC CONSTRUCTION")
}

/// Activate the pre-session console used for encrypted CFC unlock and
/// fail-closed manifest diagnostics on installed UEFI systems.
pub fn enable_boot() -> bool {
    enable("ExpOS Secure Boot", "CFC UNLOCK + VERIFIED STARTUP")
}

/// Keep the command environment visible when UEFI GOP remains the active
/// scanout and no legacy VGA text mode exists.
pub fn enable_console() -> bool {
    enable("ExpOS Console", "FORM COMMAND ENVIRONMENT")
}

/// Stop mirroring text and return QEMU to VGA text before the normal session
/// chooser selects its graphical or console environment.
pub fn disable() {
    let was_enabled = {
        let mut console = CONSOLE.lock();
        let enabled = console.enabled;
        console.enabled = false;
        enabled
    };
    if was_enabled {
        framebuffer::exit();
    }
}

pub fn write(args: fmt::Arguments<'_>) {
    let _ = fmt::Write::write_fmt(&mut *CONSOLE.lock(), args);
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write as _;

    #[test]
    fn console_wraps_backspaces_and_scrolls_inside_its_fixed_grid() {
        let mut console = Console::new();
        console.enabled = true;
        for _ in 0..ROWS + 2 {
            console.write_str("line\n").unwrap();
        }
        assert_eq!(&console.cells[ROWS - 2][..4], &['l', 'i', 'n', 'e']);
        console.write_str("abc\x08 ").unwrap();
        assert_eq!(console.cells[ROWS - 1][0], 'a');
        assert_eq!(console.cells[ROWS - 1][1], 'b');
        assert_eq!(console.cells[ROWS - 1][2], ' ');
    }
}
