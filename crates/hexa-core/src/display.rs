use crate::{Fin, Text};

const MAX_SURFACES: usize = 16;
const MAX_DISPLAY_EVENTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub const fn new(x: i16, y: i16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(self, x: i16, y: i16) -> bool {
        x >= self.x
            && y >= self.y
            && x < self.x.saturating_add_unsigned(self.width)
            && y < self.y.saturating_add_unsigned(self.height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferHandle {
    pub id: u32,
    pub owner: Fin,
    pub width: u16,
    pub height: u16,
    pub format: BufferFormat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferFormat {
    Xrgb8888,
    Argb8888,
    Rgb565,
    TextCells,
}

impl BufferFormat {
    pub const fn bits_per_pixel(self) -> u8 {
        match self {
            Self::Xrgb8888 | Self::Argb8888 => 32,
            Self::Rgb565 | Self::TextCells => 16,
        }
    }

    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::Argb8888)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRole {
    Background,
    Window,
    Panel,
    Popup,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SurfaceState {
    pub rect: Rect,
    pub buffer: Option<BufferHandle>,
    pub damage: Option<Rect>,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Surface {
    pub id: u32,
    pub owner: Fin,
    pub title: Text,
    pub role: SurfaceRole,
    pub current: SurfaceState,
    pub pending: SurfaceState,
    pub commit_sequence: u64,
    pub z_index: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayError {
    Invalid,
    Denied,
    NotFound,
    Full,
    BufferSizeMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayEventKind {
    Configure,
    FocusIn,
    FocusOut,
    FrameDone,
    Key,
    PointerMotion,
    PointerButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DisplayEvent {
    pub serial: u64,
    pub owner: Fin,
    pub surface_id: u32,
    pub kind: DisplayEventKind,
    pub value: u64,
}

pub struct DisplayServer {
    surfaces: [Option<Surface>; MAX_SURFACES],
    next_surface_id: u32,
    next_z: u16,
    commit_sequence: u64,
    focused: Option<u32>,
    events: [Option<DisplayEvent>; MAX_DISPLAY_EVENTS],
    next_event_serial: u64,
}

impl DisplayServer {
    pub const fn new() -> Self {
        Self {
            surfaces: [None; MAX_SURFACES],
            next_surface_id: 1,
            next_z: 1,
            commit_sequence: 0,
            focused: None,
            events: [None; MAX_DISPLAY_EVENTS],
            next_event_serial: 1,
        }
    }

    pub fn create_surface(
        &mut self,
        owner: Fin,
        title: &str,
        role: SurfaceRole,
        rect: Rect,
    ) -> Result<u32, DisplayError> {
        if owner.is_zero() || rect.width == 0 || rect.height == 0 {
            return Err(DisplayError::Invalid);
        }
        let title = Text::new(title).map_err(|_| DisplayError::Invalid)?;
        let slot = self
            .surfaces
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(DisplayError::Full)?;
        let id = self.next_surface_id;
        self.next_surface_id = self.next_surface_id.wrapping_add(1).max(1);
        let state = SurfaceState {
            rect,
            buffer: None,
            damage: Some(rect),
            visible: true,
        };
        *slot = Some(Surface {
            id,
            owner,
            title,
            role,
            current: state,
            pending: state,
            commit_sequence: 0,
            z_index: self.next_z,
        });
        self.next_z = self.next_z.saturating_add(1);
        self.push_event(owner, id, DisplayEventKind::Configure, pack_size(rect));
        Ok(id)
    }

    pub fn attach(
        &mut self,
        owner: Fin,
        surface_id: u32,
        buffer: BufferHandle,
    ) -> Result<(), DisplayError> {
        let surface = self.owned_mut(owner, surface_id)?;
        if buffer.owner != owner {
            return Err(DisplayError::Denied);
        }
        if buffer.width < surface.pending.rect.width || buffer.height < surface.pending.rect.height
        {
            return Err(DisplayError::BufferSizeMismatch);
        }
        surface.pending.buffer = Some(buffer);
        Ok(())
    }

    pub fn damage(
        &mut self,
        owner: Fin,
        surface_id: u32,
        damage: Rect,
    ) -> Result<(), DisplayError> {
        let surface = self.owned_mut(owner, surface_id)?;
        if damage.width == 0 || damage.height == 0 {
            return Err(DisplayError::Invalid);
        }
        surface.pending.damage = Some(damage);
        Ok(())
    }

    pub fn set_position(
        &mut self,
        owner: Fin,
        surface_id: u32,
        x: i16,
        y: i16,
    ) -> Result<(), DisplayError> {
        let surface = self.owned_mut(owner, surface_id)?;
        surface.pending.rect.x = x;
        surface.pending.rect.y = y;
        Ok(())
    }

    pub fn set_geometry(
        &mut self,
        owner: Fin,
        surface_id: u32,
        rect: Rect,
    ) -> Result<(), DisplayError> {
        if rect.width == 0 || rect.height == 0 {
            return Err(DisplayError::Invalid);
        }
        let surface = self.owned_mut(owner, surface_id)?;
        if surface
            .pending
            .buffer
            .is_some_and(|buffer| buffer.width < rect.width || buffer.height < rect.height)
        {
            return Err(DisplayError::BufferSizeMismatch);
        }
        surface.pending.rect = rect;
        surface.pending.damage = Some(Rect::new(0, 0, rect.width, rect.height));
        Ok(())
    }

    pub fn set_visible(
        &mut self,
        owner: Fin,
        surface_id: u32,
        visible: bool,
    ) -> Result<(), DisplayError> {
        self.owned_mut(owner, surface_id)?.pending.visible = visible;
        Ok(())
    }

    pub fn commit(&mut self, owner: Fin, surface_id: u32) -> Result<u64, DisplayError> {
        self.commit_sequence = self.commit_sequence.wrapping_add(1).max(1);
        let sequence = self.commit_sequence;
        let surface = self.owned_mut(owner, surface_id)?;
        surface.current = surface.pending;
        surface.current.damage = None;
        surface.pending.damage = None;
        surface.commit_sequence = sequence;
        self.push_event(owner, surface_id, DisplayEventKind::FrameDone, sequence);
        Ok(sequence)
    }

    pub fn focus(&mut self, surface_id: u32) -> Result<(), DisplayError> {
        let previous = self.focused;
        let previous_owner = previous
            .filter(|previous_id| *previous_id != surface_id)
            .and_then(|previous_id| {
                self.surface(previous_id)
                    .map(|surface| (previous_id, surface.owner))
            });
        let owner = {
            let surface = self
                .surfaces
                .iter_mut()
                .flatten()
                .find(|surface| surface.id == surface_id)
                .ok_or(DisplayError::NotFound)?;
            if !surface.current.visible || matches!(surface.role, SurfaceRole::Background) {
                return Err(DisplayError::Denied);
            }
            surface.z_index = self.next_z;
            surface.owner
        };
        self.next_z = self.next_z.saturating_add(1);
        self.focused = Some(surface_id);
        if let Some((previous_id, previous_owner)) = previous_owner {
            self.push_event(previous_owner, previous_id, DisplayEventKind::FocusOut, 0);
        }
        self.push_event(owner, surface_id, DisplayEventKind::FocusIn, 0);
        Ok(())
    }

    pub fn hit_test(&self, x: i16, y: i16) -> Option<u32> {
        self.surfaces
            .iter()
            .flatten()
            .filter(|surface| surface.current.visible && surface.current.rect.contains(x, y))
            .max_by_key(|surface| surface.z_index)
            .map(|surface| surface.id)
    }

    pub fn destroy(&mut self, owner: Fin, surface_id: u32) -> Result<(), DisplayError> {
        let slot = self
            .surfaces
            .iter_mut()
            .find(|slot| slot.is_some_and(|surface| surface.id == surface_id))
            .ok_or(DisplayError::NotFound)?;
        if slot.as_ref().is_none_or(|surface| surface.owner != owner) {
            return Err(DisplayError::Denied);
        }
        *slot = None;
        if self.focused == Some(surface_id) {
            self.focused = None;
        }
        Ok(())
    }

    pub fn route_key(&mut self, key: u8) -> Result<(), DisplayError> {
        let surface_id = self.focused.ok_or(DisplayError::NotFound)?;
        let owner = self
            .surface(surface_id)
            .ok_or(DisplayError::NotFound)?
            .owner;
        self.push_event(owner, surface_id, DisplayEventKind::Key, key as u64);
        Ok(())
    }

    pub fn route_pointer(
        &mut self,
        x: i16,
        y: i16,
        buttons: u8,
        changed: u8,
    ) -> Result<u32, DisplayError> {
        let surface_id = self.hit_test(x, y).ok_or(DisplayError::NotFound)?;
        let owner = self
            .surface(surface_id)
            .ok_or(DisplayError::NotFound)?
            .owner;
        let value = x as u16 as u64
            | ((y as u16 as u64) << 16)
            | ((buttons as u64) << 32)
            | ((changed as u64) << 40);
        self.push_event(owner, surface_id, DisplayEventKind::PointerMotion, value);
        if changed != 0 {
            self.push_event(owner, surface_id, DisplayEventKind::PointerButton, value);
        }
        Ok(surface_id)
    }

    pub fn poll_event(&mut self, owner: Fin) -> Option<DisplayEvent> {
        let index = self
            .events
            .iter()
            .enumerate()
            .filter_map(|(index, event)| {
                event
                    .filter(|event| event.owner == owner)
                    .map(|event| (index, event.serial))
            })
            .min_by_key(|(_, serial)| *serial)
            .map(|(index, _)| index)?;
        self.events[index].take()
    }

    pub fn surface(&self, surface_id: u32) -> Option<&Surface> {
        self.surfaces
            .iter()
            .flatten()
            .find(|surface| surface.id == surface_id)
    }

    pub fn surfaces(&self) -> impl Iterator<Item = &Surface> {
        self.surfaces.iter().flatten()
    }

    pub const fn focused(&self) -> Option<u32> {
        self.focused
    }

    pub const fn commit_sequence(&self) -> u64 {
        self.commit_sequence
    }

    fn owned_mut(&mut self, owner: Fin, surface_id: u32) -> Result<&mut Surface, DisplayError> {
        let surface = self
            .surfaces
            .iter_mut()
            .flatten()
            .find(|surface| surface.id == surface_id)
            .ok_or(DisplayError::NotFound)?;
        if surface.owner != owner {
            return Err(DisplayError::Denied);
        }
        Ok(surface)
    }

    fn push_event(&mut self, owner: Fin, surface_id: u32, kind: DisplayEventKind, value: u64) {
        let event = DisplayEvent {
            serial: self.next_event_serial,
            owner,
            surface_id,
            kind,
            value,
        };
        self.next_event_serial = self.next_event_serial.wrapping_add(1).max(1);
        if let Some(slot) = self.events.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(event);
        }
    }
}

const fn pack_size(rect: Rect) -> u64 {
    ((rect.width as u64) << 32) | rect.height as u64
}

impl Default for DisplayServer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_surface_state_is_atomic_and_owner_scoped() {
        let owner = Fin::from_u128(1);
        let intruder = Fin::from_u128(2);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(
                owner,
                "Browser",
                SurfaceRole::Window,
                Rect::new(20, 30, 640, 480),
            )
            .unwrap();
        display.set_position(owner, surface, 50, 60).unwrap();
        assert_eq!(display.surface(surface).unwrap().current.rect.x, 20);
        assert_eq!(
            display.set_position(intruder, surface, 0, 0),
            Err(DisplayError::Denied)
        );
        assert_eq!(display.commit(owner, surface), Ok(1));
        assert_eq!(display.surface(surface).unwrap().current.rect.x, 50);
    }

    #[test]
    fn focus_and_hit_testing_follow_z_order() {
        let owner = Fin::from_u128(1);
        let mut display = DisplayServer::new();
        let back = display
            .create_surface(
                owner,
                "Back",
                SurfaceRole::Window,
                Rect::new(0, 0, 100, 100),
            )
            .unwrap();
        let front = display
            .create_surface(
                owner,
                "Front",
                SurfaceRole::Window,
                Rect::new(20, 20, 100, 100),
            )
            .unwrap();
        assert_eq!(display.hit_test(30, 30), Some(front));
        display.focus(back).unwrap();
        assert_eq!(display.hit_test(30, 30), Some(back));
        assert_eq!(display.focused(), Some(back));
        display.route_key(b'x').unwrap();
        assert_eq!(display.route_pointer(30, 30, 1, 1), Ok(back));
        let events: std::vec::Vec<_> = core::iter::from_fn(|| display.poll_event(owner)).collect();
        assert!(events
            .iter()
            .any(|event| event.kind == DisplayEventKind::FocusIn));
        assert!(events
            .iter()
            .any(|event| event.kind == DisplayEventKind::Key && event.value == b'x' as u64));
        assert!(events
            .iter()
            .any(|event| event.kind == DisplayEventKind::PointerButton));
    }

    #[test]
    fn geometry_is_atomic_and_cannot_outgrow_its_buffer() {
        let owner = Fin::from_u128(8);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(
                owner,
                "Terminal",
                SurfaceRole::Window,
                Rect::new(0, 0, 640, 480),
            )
            .unwrap();
        display
            .attach(
                owner,
                surface,
                BufferHandle {
                    id: 1,
                    owner,
                    width: 704,
                    height: 500,
                    format: BufferFormat::Xrgb8888,
                },
            )
            .unwrap();
        display
            .set_geometry(owner, surface, Rect::new(20, 30, 500, 400))
            .unwrap();
        assert_eq!(display.surface(surface).unwrap().current.rect.width, 640);
        display.commit(owner, surface).unwrap();
        assert_eq!(display.surface(surface).unwrap().current.rect.width, 500);
        assert_eq!(
            display.set_geometry(owner, surface, Rect::new(0, 0, 705, 400)),
            Err(DisplayError::BufferSizeMismatch)
        );
        assert_eq!(BufferFormat::Xrgb8888.bits_per_pixel(), 32);
        assert_eq!(BufferFormat::Argb8888.bits_per_pixel(), 32);
        assert!(BufferFormat::Argb8888.has_alpha());
        assert_eq!(BufferFormat::Rgb565.bits_per_pixel(), 16);
        assert!(!BufferFormat::Rgb565.has_alpha());
    }
}
