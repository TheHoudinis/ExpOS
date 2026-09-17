use crate::{framebuffer, input};
use expos_core::Rect;
use framebuffer::color;

const SNAKE_CAPACITY: usize = 96;
const SNAKE_WIDTH: i16 = 32;
const SNAKE_HEIGHT: i16 = 20;
const MILLIS_PER_SECOND: u64 = 1_000;
const FALLBACK_TSC_HZ: u64 = 1_000_000_000;
const SNAKE_TICK_MILLIS: u64 = 1_200;
const PONG_TICK_MILLIS: u64 = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GameTiming {
    snake_interval: u64,
    pong_interval: u64,
}

impl GameTiming {
    const fn calibrated(tsc_hz: u64) -> Self {
        let tsc_hz = if tsc_hz == 0 { FALLBACK_TSC_HZ } else { tsc_hz };
        Self {
            snake_interval: ticks_from_millis(tsc_hz, SNAKE_TICK_MILLIS),
            pong_interval: ticks_from_millis(tsc_hz, PONG_TICK_MILLIS),
        }
    }

    const fn interval(self, mode: GameMode) -> Option<u64> {
        match mode {
            GameMode::Menu => None,
            GameMode::Snake => Some(self.snake_interval),
            GameMode::Pong => Some(self.pong_interval),
        }
    }
}

const fn ticks_from_millis(tsc_hz: u64, milliseconds: u64) -> u64 {
    let numerator = tsc_hz as u128 * milliseconds as u128;
    let ticks = (numerator + (MILLIS_PER_SECOND - 1) as u128) / MILLIS_PER_SECOND as u128;
    if ticks == 0 {
        1
    } else if ticks > u64::MAX as u128 {
        u64::MAX
    } else {
        ticks as u64
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameMode {
    Menu,
    Snake,
    Pong,
}

pub struct GameHub {
    mode: GameMode,
    paused: bool,
    last_tick: u64,
    timing: GameTiming,
    snake: Snake,
    pong: Pong,
}

impl GameHub {
    pub fn new() -> Self {
        Self::with_clock_hz(crate::hardware::clock_info().tsc_hz)
    }

    const fn with_clock_hz(tsc_hz: u64) -> Self {
        Self {
            mode: GameMode::Menu,
            paused: false,
            last_tick: 0,
            timing: GameTiming::calibrated(tsc_hz),
            snake: Snake::new(),
            pong: Pong::new(),
        }
    }

    pub fn handle_key(&mut self, key: u8) -> bool {
        match key.to_ascii_lowercase() {
            b'1' => {
                self.mode = GameMode::Snake;
                self.snake.reset();
                self.paused = false;
                true
            }
            b'2' => {
                self.mode = GameMode::Pong;
                self.pong.reset();
                self.paused = false;
                true
            }
            b'm' => {
                self.mode = GameMode::Menu;
                self.paused = false;
                true
            }
            b'r' => {
                match self.mode {
                    GameMode::Snake => self.snake.reset(),
                    GameMode::Pong => self.pong.reset(),
                    GameMode::Menu => {}
                }
                self.paused = false;
                true
            }
            b' ' if self.mode != GameMode::Menu => {
                self.paused = !self.paused;
                true
            }
            _ => match (self.mode, key) {
                (GameMode::Snake, input::KEY_UP) => {
                    self.snake.turn(0, -1);
                    true
                }
                (GameMode::Snake, input::KEY_DOWN) => {
                    self.snake.turn(0, 1);
                    true
                }
                (GameMode::Snake, input::KEY_LEFT) => {
                    self.snake.turn(-1, 0);
                    true
                }
                (GameMode::Snake, input::KEY_RIGHT) => {
                    self.snake.turn(1, 0);
                    true
                }
                (GameMode::Pong, input::KEY_UP) => {
                    self.pong.player = (self.pong.player - 70).max(0);
                    true
                }
                (GameMode::Pong, input::KEY_DOWN) => {
                    self.pong.player = (self.pong.player + 70).min(460);
                    true
                }
                _ => false,
            },
        }
    }

    pub fn handle_click(&mut self, x: i16, y: i16, rect: Rect) -> bool {
        if self.mode != GameMode::Menu {
            return false;
        }
        let local_x = x - rect.x;
        let local_y = y - rect.y;
        if !(122..302).contains(&local_y) {
            return false;
        }
        let card_width = (rect.width as i16 - 82) / 2;
        if (34..34 + card_width).contains(&local_x) {
            self.mode = GameMode::Snake;
            self.snake.reset();
            self.paused = false;
            true
        } else if (48 + card_width..48 + card_width * 2).contains(&local_x) {
            self.mode = GameMode::Pong;
            self.pong.reset();
            self.paused = false;
            true
        } else {
            false
        }
    }

    pub fn tick(&mut self, now: u64) -> bool {
        if self.mode == GameMode::Menu || self.paused {
            self.last_tick = now;
            return false;
        }
        if self.mode == GameMode::Snake && !self.snake.started {
            self.last_tick = now;
            return false;
        }
        let Some(interval) = self.timing.interval(self.mode) else {
            return false;
        };
        if now.wrapping_sub(self.last_tick) < interval {
            return false;
        }
        self.last_tick = now;
        match self.mode {
            GameMode::Snake => self.snake.tick(),
            GameMode::Pong => self.pong.tick(),
            GameMode::Menu => {}
        }
        true
    }

    pub fn render(&self, rect: Rect) {
        match self.mode {
            GameMode::Menu => self.render_menu(rect),
            GameMode::Snake => self.render_snake(rect),
            GameMode::Pong => self.render_pong(rect),
        }
    }

    fn render_menu(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        let width = rect.width as i32;
        framebuffer::text(x + 34, y + 58, "PRISM ARCADE", 0x00FF_5CC8, 2);
        framebuffer::text(
            x + 34,
            y + 88,
            "NATIVE GAME FORMS // NO LEGACY FALLBACK",
            color::MUTED,
            1,
        );
        game_card(
            x + 34,
            y + 122,
            (width - 82) / 2,
            "1  SNAKE",
            "EAT SIGNALS. AVOID WALLS AND YOUR TAIL.",
            color::GREEN,
        );
        game_card(
            x + 48 + (width - 82) / 2,
            y + 122,
            (width - 82) / 2,
            "2  PONG",
            "DEFEND THE LEFT EDGE AGAINST THE CPU.",
            color::CYAN,
        );
        framebuffer::text(
            x + 44,
            y + 338,
            "SELECT 1 OR 2   ARROWS MOVE   SPACE PAUSE   R RESET   M MENU",
            color::INK,
            1,
        );
    }

    fn render_snake(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        framebuffer::text(x + 28, y + 54, "SNAKE // SIGNAL GARDEN", color::GREEN, 2);
        framebuffer::text(x + 490, y + 58, "SCORE", color::MUTED, 1);
        draw_number(x + 550, y + 58, self.snake.score as u64, color::GREEN);
        let board_x = x + 50;
        let board_y = y + 94;
        let cell = 11;
        framebuffer::rect(
            board_x - 8,
            board_y - 8,
            SNAKE_WIDTH as i32 * cell + 16,
            SNAKE_HEIGHT as i32 * cell + 16,
            0x0008_0B12,
        );
        framebuffer::outline(
            board_x - 8,
            board_y - 8,
            SNAKE_WIDTH as i32 * cell + 16,
            SNAKE_HEIGHT as i32 * cell + 16,
            color::PURPLE,
        );
        framebuffer::rect(
            board_x + self.snake.food_x as i32 * cell + 2,
            board_y + self.snake.food_y as i32 * cell + 2,
            cell - 4,
            cell - 4,
            0x00FF_5CC8,
        );
        for index in (0..self.snake.len).rev() {
            framebuffer::rect(
                board_x + self.snake.x[index] as i32 * cell + 1,
                board_y + self.snake.y[index] as i32 * cell + 1,
                cell - 2,
                cell - 2,
                if index == 0 {
                    color::WHITE
                } else {
                    color::GREEN
                },
            );
        }
        framebuffer::text(x + 450, y + 120, "ARROWS", color::PURPLE, 2);
        framebuffer::text(x + 450, y + 152, "STEER", color::INK, 1);
        framebuffer::text(x + 450, y + 185, "SPACE", color::CYAN, 2);
        framebuffer::text(x + 450, y + 217, "PAUSE", color::INK, 1);
        framebuffer::text(x + 450, y + 250, "R RESET", color::RED, 2);
        framebuffer::text(x + 450, y + 292, "M MENU", color::MUTED, 1);
        if !self.snake.alive {
            framebuffer::rect(board_x + 58, board_y + 82, 238, 56, color::PANEL);
            framebuffer::outline(board_x + 58, board_y + 82, 238, 56, color::RED);
            framebuffer::text(board_x + 79, board_y + 94, "SIGNAL LOST", color::RED, 2);
            framebuffer::text(board_x + 98, board_y + 122, "PRESS R", color::WHITE, 1);
        } else if self.paused {
            framebuffer::text(board_x + 135, board_y + 105, "PAUSED", color::WHITE, 2);
        } else if !self.snake.started {
            framebuffer::text(
                board_x + 87,
                board_y + 105,
                "PRESS ARROW TO START",
                color::WHITE,
                1,
            );
        }
    }

    fn render_pong(&self, rect: Rect) {
        let x = rect.x as i32;
        let y = rect.y as i32;
        framebuffer::text(x + 28, y + 54, "PONG // FORM DUEL", color::CYAN, 2);
        draw_number(x + 310, y + 58, self.pong.player_score as u64, color::GREEN);
        framebuffer::text(x + 335, y + 58, ":", color::MUTED, 1);
        draw_number(x + 351, y + 58, self.pong.cpu_score as u64, color::RED);
        let field_x = x + 48;
        let field_y = y + 92;
        let field_w = 650;
        let field_h = 330;
        framebuffer::rect(field_x, field_y, field_w, field_h, 0x0008_0B12);
        framebuffer::outline(field_x, field_y, field_w, field_h, color::PURPLE);
        for segment in 0..11 {
            framebuffer::rect(
                field_x + field_w / 2 - 2,
                field_y + 10 + segment * 29,
                4,
                15,
                0x0030_374C,
            );
        }
        let player_y = field_y + self.pong.player as i32 * (field_h - 70) / 600;
        let cpu_y = field_y + self.pong.cpu as i32 * (field_h - 70) / 600;
        framebuffer::rect(field_x + 18, player_y, 10, 70, color::GREEN);
        framebuffer::rect(field_x + field_w - 28, cpu_y, 10, 70, color::RED);
        framebuffer::rect(
            field_x + self.pong.ball_x as i32 * (field_w - 20) / 1000 + 5,
            field_y + self.pong.ball_y as i32 * (field_h - 20) / 600 + 5,
            14,
            14,
            color::WHITE,
        );
        framebuffer::text(
            x + 48,
            y + 440,
            "UP DOWN MOVE   SPACE PAUSE   R RESET   M ARCADE",
            color::MUTED,
            1,
        );
        if self.paused {
            framebuffer::text(field_x + 280, field_y + 155, "PAUSED", color::WHITE, 2);
        }
    }
}

struct Snake {
    x: [u8; SNAKE_CAPACITY],
    y: [u8; SNAKE_CAPACITY],
    len: usize,
    dx: i8,
    dy: i8,
    next_dx: i8,
    next_dy: i8,
    food_x: u8,
    food_y: u8,
    score: u16,
    alive: bool,
    started: bool,
}

impl Snake {
    const fn new() -> Self {
        let mut x = [0; SNAKE_CAPACITY];
        let mut y = [0; SNAKE_CAPACITY];
        x[0] = 16;
        x[1] = 15;
        x[2] = 14;
        y[0] = 10;
        y[1] = 10;
        y[2] = 10;
        Self {
            x,
            y,
            len: 3,
            dx: 1,
            dy: 0,
            next_dx: 1,
            next_dy: 0,
            food_x: 23,
            food_y: 10,
            score: 0,
            alive: true,
            started: false,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn turn(&mut self, dx: i8, dy: i8) {
        self.started = true;
        if dx != -self.dx || dy != -self.dy {
            self.next_dx = dx;
            self.next_dy = dy;
        }
    }

    fn tick(&mut self) {
        if !self.alive {
            return;
        }
        self.dx = self.next_dx;
        self.dy = self.next_dy;
        let next_x = self.x[0] as i16 + self.dx as i16;
        let next_y = self.y[0] as i16 + self.dy as i16;
        if !(0..SNAKE_WIDTH).contains(&next_x)
            || !(0..SNAKE_HEIGHT).contains(&next_y)
            || (0..self.len.saturating_sub(1))
                .any(|index| self.x[index] == next_x as u8 && self.y[index] == next_y as u8)
        {
            self.alive = false;
            return;
        }
        let ate = next_x as u8 == self.food_x && next_y as u8 == self.food_y;
        if ate && self.len < SNAKE_CAPACITY {
            self.len += 1;
            self.score = self.score.saturating_add(10);
        }
        for index in (1..self.len).rev() {
            self.x[index] = self.x[index - 1];
            self.y[index] = self.y[index - 1];
        }
        self.x[0] = next_x as u8;
        self.y[0] = next_y as u8;
        if ate {
            let seed = self.score as usize + self.len * 11;
            for attempt in 0..(SNAKE_WIDTH as usize * SNAKE_HEIGHT as usize) {
                let candidate_x = ((seed * 7 + attempt * 13) % SNAKE_WIDTH as usize) as u8;
                let candidate_y = ((seed * 5 + attempt * 17) % SNAKE_HEIGHT as usize) as u8;
                if !(0..self.len)
                    .any(|index| self.x[index] == candidate_x && self.y[index] == candidate_y)
                {
                    self.food_x = candidate_x;
                    self.food_y = candidate_y;
                    break;
                }
            }
        }
    }
}

struct Pong {
    ball_x: i16,
    ball_y: i16,
    velocity_x: i16,
    velocity_y: i16,
    player: i16,
    cpu: i16,
    player_score: u8,
    cpu_score: u8,
}

impl Pong {
    const fn new() -> Self {
        Self {
            ball_x: 500,
            ball_y: 300,
            velocity_x: -18,
            velocity_y: 13,
            player: 230,
            cpu: 230,
            player_score: 0,
            cpu_score: 0,
        }
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    fn tick(&mut self) {
        self.ball_x += self.velocity_x;
        self.ball_y += self.velocity_y;
        if self.ball_y <= 0 || self.ball_y >= 590 {
            self.velocity_y = -self.velocity_y;
            self.ball_y = self.ball_y.clamp(0, 590);
        }
        if self.ball_y > self.cpu + 45 {
            self.cpu = (self.cpu + 12).min(460);
        } else if self.ball_y < self.cpu + 25 {
            self.cpu = (self.cpu - 12).max(0);
        }
        if self.ball_x <= 45
            && self.velocity_x < 0
            && (self.player..=self.player + 140).contains(&self.ball_y)
        {
            self.velocity_x = -self.velocity_x;
            self.ball_x = 46;
        }
        if self.ball_x >= 935
            && self.velocity_x > 0
            && (self.cpu..=self.cpu + 140).contains(&self.ball_y)
        {
            self.velocity_x = -self.velocity_x;
            self.ball_x = 934;
        }
        if self.ball_x < 0 {
            self.cpu_score = self.cpu_score.saturating_add(1);
            self.serve(1);
        } else if self.ball_x > 1000 {
            self.player_score = self.player_score.saturating_add(1);
            self.serve(-1);
        }
    }

    fn serve(&mut self, direction: i16) {
        self.ball_x = 500;
        self.ball_y = 300;
        self.velocity_x = 18 * direction;
        self.velocity_y = if (self.player_score + self.cpu_score).is_multiple_of(2) {
            13
        } else {
            -13
        };
    }
}

fn game_card(x: i32, y: i32, width: i32, title: &str, detail: &str, accent: u32) {
    framebuffer::rect(x, y, width, 180, 0x0015_1922);
    framebuffer::outline(x, y, width, 180, color::BORDER);
    framebuffer::rect(x, y, 7, 180, accent);
    framebuffer::text(x + 25, y + 28, title, accent, 2);
    framebuffer::text(x + 25, y + 72, detail, color::INK, 1);
    framebuffer::text(x + 25, y + 135, "PLAY NOW", color::PURPLE, 1);
}

fn draw_number(x: i32, y: i32, mut value: u64, color: u32) {
    let mut bytes = [b'0'; 20];
    let mut start = bytes.len() - 1;
    while value >= 10 {
        bytes[start] = b'0' + (value % 10) as u8;
        value /= 10;
        start -= 1;
    }
    bytes[start] = b'0' + value as u8;
    let text = core::str::from_utf8(&bytes[start..]).unwrap_or("?");
    framebuffer::text(x, y, text, color, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_scales_with_the_reported_tsc_frequency() {
        let one_ghz = GameTiming::calibrated(1_000_000_000);
        let two_and_a_half_ghz = GameTiming::calibrated(2_500_000_000);

        assert_eq!(one_ghz.snake_interval, 1_200_000_000);
        assert_eq!(one_ghz.pong_interval, 120_000_000);
        assert_eq!(two_and_a_half_ghz.snake_interval, 3_000_000_000);
        assert_eq!(two_and_a_half_ghz.pong_interval, 300_000_000);
    }

    #[test]
    fn zero_frequency_uses_the_conservative_fallback() {
        assert_eq!(
            GameTiming::calibrated(0),
            GameTiming::calibrated(FALLBACK_TSC_HZ)
        );
    }

    #[test]
    fn interval_conversion_rounds_up_and_saturates() {
        assert_eq!(ticks_from_millis(3, 120), 1);
        assert_eq!(ticks_from_millis(u64::MAX, 1_200), u64::MAX);
    }

    #[test]
    fn pong_ticks_at_the_same_wall_time_on_different_clocks() {
        let mut slow_clock = GameHub::with_clock_hz(10_000);
        let mut fast_clock = GameHub::with_clock_hz(25_000);
        assert!(slow_clock.handle_key(b'2'));
        assert!(fast_clock.handle_key(b'2'));

        assert!(!slow_clock.tick(1_199));
        assert!(!fast_clock.tick(2_999));
        assert!(slow_clock.tick(1_200));
        assert!(fast_clock.tick(3_000));
    }

    #[test]
    fn snake_waits_for_input_then_uses_its_calibrated_interval() {
        let mut hub = GameHub::with_clock_hz(10_000);
        assert!(hub.handle_key(b'1'));
        assert!(!hub.tick(50_000));
        assert!(hub.handle_key(input::KEY_RIGHT));
        assert!(!hub.tick(61_999));
        assert!(hub.tick(62_000));
    }

    #[test]
    fn paused_game_anchors_its_next_deadline_to_resume_time() {
        let mut hub = GameHub::with_clock_hz(10_000);
        assert!(hub.handle_key(b'2'));
        assert!(hub.handle_key(b' '));
        assert!(!hub.tick(50_000));
        assert!(hub.handle_key(b' '));
        assert!(!hub.tick(51_199));
        assert!(hub.tick(51_200));
    }
}
