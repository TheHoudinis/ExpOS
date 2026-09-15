//! Allocation-free HexaDisplay protocol state.
//!
//! HexaDisplay is Form-native rather than a Unix or Wayland compatibility ABI:
//! every surface and buffer remains bound to its owning FIN. Surface changes are
//! committed atomically, while damage and frame completion are synchronized at
//! an explicit compositor presentation boundary.

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
    /// Surface-local damage. On `current` this is the bounding region accumulated
    /// since the last completed frame; on `pending` it belongs to the next commit.
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
    /// Event payload. For `FrameDone`, this is the newest commit sequence that
    /// became visible; other event kinds retain their existing payload format.
    pub value: u64,
}

pub struct DisplayServer {
    surfaces: [Option<Surface>; MAX_SURFACES],
    /// Output-local damage retained until the compositor confirms presentation.
    ///
    /// Damage is tracked per surface slot so the server stays allocation-free.
    /// A slot's damage deliberately survives surface destruction (and is merged
    /// if that slot is reused) until `complete_frame` clears the presented frame.
    pending_output_damage: [Option<Rect>; MAX_SURFACES],
    next_surface_id: u32,
    next_z: u16,
    commit_sequence: u64,
    frame_sequence: u64,
    pending_frame_commits: [u64; MAX_SURFACES],
    focused: Option<u32>,
    events: [Option<DisplayEvent>; MAX_DISPLAY_EVENTS],
    next_event_serial: u64,
}

impl DisplayServer {
    pub const fn new() -> Self {
        Self {
            surfaces: [None; MAX_SURFACES],
            pending_output_damage: [None; MAX_SURFACES],
            next_surface_id: 1,
            next_z: 1,
            commit_sequence: 0,
            frame_sequence: 0,
            pending_frame_commits: [0; MAX_SURFACES],
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
            damage: Some(surface_bounds(rect)),
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
        add_pending_damage(surface, surface_bounds(surface.pending.rect));
        Ok(())
    }

    /// Adds a surface-local damaged region to the next atomic commit.
    ///
    /// Damage is clipped to the pending surface bounds and repeated calls are
    /// coalesced into one bounding rectangle without allocating. A region that
    /// cannot affect the surface is rejected.
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
        let clipped = intersect_damage(damage, surface_bounds(surface.pending.rect))
            .ok_or(DisplayError::Invalid)?;
        add_pending_damage(surface, clipped);
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
        if surface.pending.rect.x != x || surface.pending.rect.y != y {
            surface.pending.rect.x = x;
            surface.pending.rect.y = y;
            add_pending_damage(surface, surface_bounds(surface.pending.rect));
        }
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
        surface.pending.damage = Some(surface_bounds(rect));
        Ok(())
    }

    pub fn set_visible(
        &mut self,
        owner: Fin,
        surface_id: u32,
        visible: bool,
    ) -> Result<(), DisplayError> {
        let surface = self.owned_mut(owner, surface_id)?;
        if surface.pending.visible != visible {
            surface.pending.visible = visible;
            add_pending_damage(surface, surface_bounds(surface.pending.rect));
        }
        Ok(())
    }

    /// Atomically applies pending state and returns its globally ordered commit
    /// sequence. Frame completion is deliberately deferred until
    /// [`Self::complete_frame`] is called after scanout presentation.
    pub fn commit(&mut self, owner: Fin, surface_id: u32) -> Result<u64, DisplayError> {
        let index = self.owned_index(owner, surface_id)?;
        self.commit_sequence = self.commit_sequence.wrapping_add(1).max(1);
        let sequence = self.commit_sequence;

        let (old_state, pending_state) = {
            let surface = self.surfaces[index]
                .as_ref()
                .expect("owned surface index remains populated");
            (surface.current, surface.pending)
        };
        let output_damage = committed_output_damage(old_state, pending_state);
        self.pending_output_damage[index] =
            merge_damage(self.pending_output_damage[index], output_damage);

        let unpresented_damage = (self.pending_frame_commits[index] != 0)
            .then_some(old_state.damage)
            .flatten()
            .and_then(|damage| intersect_damage(damage, surface_bounds(pending_state.rect)));
        let surface = self.surfaces[index]
            .as_mut()
            .expect("owned surface index remains populated");
        let mut committed = surface.pending;
        committed.damage = merge_damage(unpresented_damage, committed.damage);
        surface.current = committed;
        surface.pending.damage = None;
        surface.commit_sequence = sequence;
        self.pending_frame_commits[index] = sequence;
        Ok(sequence)
    }

    /// Completes one compositor frame after its pixels have reached scanout.
    ///
    /// Multiple commits to the same surface before this boundary produce one
    /// `FrameDone` event carrying the newest commit sequence. If an older
    /// unconsumed `FrameDone` already exists, it is updated in place. A full
    /// event queue defers rather than drops the callback.
    ///
    /// Returns the number of callbacks newly queued or coalesced in this frame.
    pub fn complete_frame(&mut self) -> usize {
        self.frame_sequence = self.frame_sequence.wrapping_add(1).max(1);
        let mut completed = 0;

        for index in 0..MAX_SURFACES {
            let sequence = self.pending_frame_commits[index];
            if sequence == 0 {
                continue;
            }
            let Some((owner, surface_id)) = self.surfaces[index]
                .as_ref()
                .map(|surface| (surface.owner, surface.id))
            else {
                self.pending_frame_commits[index] = 0;
                continue;
            };

            // Damage was consumed by this presentation even if callback queue
            // pressure requires the acknowledgement to be retried next frame.
            if let Some(surface) = self.surfaces[index].as_mut() {
                surface.current.damage = None;
            }
            if self.push_frame_done(owner, surface_id, sequence) {
                self.pending_frame_commits[index] = 0;
                completed += 1;
            }
        }

        // These regions describe pixels consumed by this presentation, not
        // callback delivery. Queue pressure may defer FrameDone, but it must not
        // make the compositor repaint a frame that has already reached scanout.
        self.pending_output_damage.fill(None);

        completed
    }

    /// Returns whether committed state is waiting for a presentation boundary.
    pub fn needs_frame(&self) -> bool {
        self.pending_frame_commits
            .iter()
            .any(|sequence| *sequence != 0)
            || self
                .pending_output_damage
                .iter()
                .any(|damage| damage.is_some())
    }

    /// Iterates output-local regions that must be repainted in the next frame.
    ///
    /// Surface-local committed damage is translated here, while moves, resizes,
    /// visibility changes, and destruction retain the affected global
    /// footprints. Regions accumulate across commits and remain available until
    /// `complete_frame` confirms that the frame reached scanout.
    pub fn pending_output_damage(&self) -> impl Iterator<Item = Rect> + '_ {
        self.pending_output_damage.iter().flatten().copied()
    }

    /// Monotonic compositor frame sequence, advanced by `complete_frame`.
    pub const fn frame_sequence(&self) -> u64 {
        self.frame_sequence
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
        let index = self.owned_index(owner, surface_id)?;
        if let Some(surface) = self.surfaces[index] {
            if surface.current.visible {
                self.pending_output_damage[index] = merge_damage(
                    self.pending_output_damage[index],
                    Some(surface.current.rect),
                );
            }
        }
        self.surfaces[index] = None;
        self.pending_frame_commits[index] = 0;
        for event in &mut self.events {
            if event.is_some_and(|event| event.owner == owner && event.surface_id == surface_id) {
                *event = None;
            }
        }
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

    fn owned_index(&self, owner: Fin, surface_id: u32) -> Result<usize, DisplayError> {
        let index = self
            .surfaces
            .iter()
            .position(|slot| slot.is_some_and(|surface| surface.id == surface_id))
            .ok_or(DisplayError::NotFound)?;
        if self.surfaces[index].is_none_or(|surface| surface.owner != owner) {
            return Err(DisplayError::Denied);
        }
        Ok(index)
    }

    fn push_frame_done(&mut self, owner: Fin, surface_id: u32, sequence: u64) -> bool {
        if let Some(index) = self.events.iter().position(|event| {
            event.is_some_and(|event| {
                event.owner == owner
                    && event.surface_id == surface_id
                    && event.kind == DisplayEventKind::FrameDone
            })
        }) {
            let serial = self.next_event_serial;
            self.next_event_serial = self.next_event_serial.wrapping_add(1).max(1);
            let event = self.events[index]
                .as_mut()
                .expect("matching frame event remains populated");
            event.serial = serial;
            event.value = sequence;
            return true;
        }

        let Some(slot) = self.events.iter_mut().find(|slot| slot.is_none()) else {
            return false;
        };
        *slot = Some(DisplayEvent {
            serial: self.next_event_serial,
            owner,
            surface_id,
            kind: DisplayEventKind::FrameDone,
            value: sequence,
        });
        self.next_event_serial = self.next_event_serial.wrapping_add(1).max(1);
        true
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

const fn surface_bounds(rect: Rect) -> Rect {
    Rect::new(0, 0, rect.width, rect.height)
}

fn add_pending_damage(surface: &mut Surface, damage: Rect) {
    surface.pending.damage = merge_damage(surface.pending.damage, Some(damage));
}

fn committed_output_damage(old: SurfaceState, new: SurfaceState) -> Option<Rect> {
    let geometry_changed = old.rect != new.rect || old.visible != new.visible;
    let mut output = None;

    if geometry_changed && old.visible {
        output = merge_damage(output, Some(old.rect));
    }
    if geometry_changed && new.visible {
        output = merge_damage(output, Some(new.rect));
    }
    if new.visible {
        output = merge_damage(
            output,
            new.damage.map(|damage| translate_damage(new.rect, damage)),
        );
    }

    output
}

fn translate_damage(surface: Rect, damage: Rect) -> Rect {
    Rect::new(
        surface.x.saturating_add(damage.x),
        surface.y.saturating_add(damage.y),
        damage.width,
        damage.height,
    )
}

fn intersect_damage(left: Rect, right: Rect) -> Option<Rect> {
    let x1 = (left.x as i32).max(right.x as i32);
    let y1 = (left.y as i32).max(right.y as i32);
    let x2 = (left.x as i32 + left.width as i32).min(right.x as i32 + right.width as i32);
    let y2 = (left.y as i32 + left.height as i32).min(right.y as i32 + right.height as i32);
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    Some(Rect::new(
        x1 as i16,
        y1 as i16,
        (x2 - x1) as u16,
        (y2 - y1) as u16,
    ))
}

fn merge_damage(left: Option<Rect>, right: Option<Rect>) -> Option<Rect> {
    let (Some(left), Some(right)) = (left, right) else {
        return left.or(right);
    };
    let x1 = (left.x as i32).min(right.x as i32);
    let y1 = (left.y as i32).min(right.y as i32);
    let x2 = (left.x as i32 + left.width as i32).max(right.x as i32 + right.width as i32);
    let y2 = (left.y as i32 + left.height as i32).max(right.y as i32 + right.height as i32);
    Some(Rect::new(
        x1 as i16,
        y1 as i16,
        (x2 - x1) as u16,
        (y2 - y1) as u16,
    ))
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
        assert_eq!(display.commit(intruder, surface), Err(DisplayError::Denied));
        assert_eq!(display.commit_sequence(), 0);
        assert_eq!(display.commit(owner, surface), Ok(1));
        assert_eq!(display.surface(surface).unwrap().current.rect.x, 50);
    }

    #[test]
    fn damage_is_clipped_and_coalesced_until_the_frame_completes() {
        let owner = Fin::from_u128(11);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(
                owner,
                "Canvas",
                SurfaceRole::Window,
                Rect::new(40, 50, 100, 80),
            )
            .unwrap();

        display.commit(owner, surface).unwrap();
        assert_eq!(display.complete_frame(), 1);
        while display.poll_event(owner).is_some() {}

        display
            .damage(owner, surface, Rect::new(-5, 10, 20, 20))
            .unwrap();
        display
            .damage(owner, surface, Rect::new(10, 5, 30, 15))
            .unwrap();
        assert_eq!(
            display.damage(owner, surface, Rect::new(120, 0, 5, 5)),
            Err(DisplayError::Invalid)
        );
        let sequence = display.commit(owner, surface).unwrap();

        assert_eq!(sequence, 2);
        assert_eq!(
            display.surface(surface).unwrap().current.damage,
            Some(Rect::new(0, 5, 40, 25))
        );
        assert_eq!(
            display
                .pending_output_damage()
                .collect::<std::vec::Vec<_>>(),
            std::vec![Rect::new(40, 55, 40, 25)]
        );
        assert!(display.needs_frame());
        assert_eq!(display.complete_frame(), 1);
        assert_eq!(display.surface(surface).unwrap().current.damage, None);
        assert!(!display.needs_frame());
    }

    #[test]
    fn frame_done_waits_for_scanout_and_coalesces_to_the_latest_commit() {
        let owner = Fin::from_u128(12);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(
                owner,
                "Animation",
                SurfaceRole::Window,
                Rect::new(0, 0, 64, 64),
            )
            .unwrap();
        // Ignore the initial Configure event so only frame completion remains.
        assert_eq!(
            display.poll_event(owner).map(|event| event.kind),
            Some(DisplayEventKind::Configure)
        );

        assert_eq!(display.commit(owner, surface), Ok(1));
        display.set_position(owner, surface, 4, 5).unwrap();
        assert_eq!(display.commit(owner, surface), Ok(2));
        display.set_position(owner, surface, 8, 9).unwrap();
        assert_eq!(display.commit(owner, surface), Ok(3));
        assert!(display.poll_event(owner).is_none());

        assert_eq!(display.complete_frame(), 1);
        assert_eq!(display.frame_sequence(), 1);
        let done = display.poll_event(owner).unwrap();
        assert_eq!(done.kind, DisplayEventKind::FrameDone);
        assert_eq!(done.value, 3);
        assert!(display.poll_event(owner).is_none());

        // A second frame without a commit produces no spurious callback.
        assert_eq!(display.complete_frame(), 0);
        assert_eq!(display.frame_sequence(), 2);
        assert!(display.poll_event(owner).is_none());

        // If the client has not consumed an earlier callback, a later frame
        // updates it in place instead of growing the fixed event queue.
        assert_eq!(display.commit(owner, surface), Ok(4));
        assert_eq!(display.complete_frame(), 1);
        assert_eq!(display.commit(owner, surface), Ok(5));
        assert_eq!(display.complete_frame(), 1);
        let done = display.poll_event(owner).unwrap();
        assert_eq!(done.kind, DisplayEventKind::FrameDone);
        assert_eq!(done.value, 5);
        assert!(display.poll_event(owner).is_none());
    }

    #[test]
    fn a_replaced_frame_callback_keeps_newer_input_ahead_of_it() {
        let owner = Fin::from_u128(14);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(
                owner,
                "Ordered",
                SurfaceRole::Window,
                Rect::new(0, 0, 32, 32),
            )
            .unwrap();
        display.focus(surface).unwrap();
        while display.poll_event(owner).is_some() {}

        assert_eq!(display.commit(owner, surface), Ok(1));
        assert_eq!(display.complete_frame(), 1);
        display.route_key(b'x').unwrap();
        assert_eq!(display.commit(owner, surface), Ok(2));
        assert_eq!(display.complete_frame(), 1);

        let key = display.poll_event(owner).unwrap();
        assert_eq!(key.kind, DisplayEventKind::Key);
        assert_eq!(key.value, b'x' as u64);
        let frame = display.poll_event(owner).unwrap();
        assert_eq!(frame.kind, DisplayEventKind::FrameDone);
        assert_eq!(frame.value, 2);
        assert!(display.poll_event(owner).is_none());
    }

    #[test]
    fn output_damage_keeps_old_footprints_for_move_hide_and_destroy() {
        let owner = Fin::from_u128(15);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(
                owner,
                "Moving",
                SurfaceRole::Window,
                Rect::new(10, 20, 30, 40),
            )
            .unwrap();
        display.commit(owner, surface).unwrap();
        display.complete_frame();
        while display.poll_event(owner).is_some() {}

        display.set_position(owner, surface, 100, 120).unwrap();
        display.commit(owner, surface).unwrap();
        assert_eq!(
            display
                .pending_output_damage()
                .collect::<std::vec::Vec<_>>(),
            std::vec![Rect::new(10, 20, 120, 140)]
        );

        display.set_visible(owner, surface, false).unwrap();
        display.commit(owner, surface).unwrap();
        assert_eq!(
            display
                .pending_output_damage()
                .collect::<std::vec::Vec<_>>(),
            std::vec![Rect::new(10, 20, 120, 140)]
        );
        display.complete_frame();
        assert_eq!(display.pending_output_damage().count(), 0);

        display.set_visible(owner, surface, true).unwrap();
        display.commit(owner, surface).unwrap();
        display.complete_frame();
        while display.poll_event(owner).is_some() {}
        display.destroy(owner, surface).unwrap();
        assert!(display.needs_frame());
        assert_eq!(
            display
                .pending_output_damage()
                .collect::<std::vec::Vec<_>>(),
            std::vec![Rect::new(100, 120, 30, 40)]
        );
        assert_eq!(display.complete_frame(), 0);
        assert!(!display.needs_frame());
        assert_eq!(display.pending_output_damage().count(), 0);
    }

    #[test]
    fn a_full_event_queue_defers_instead_of_dropping_frame_done() {
        let owner = Fin::from_u128(13);
        let mut display = DisplayServer::new();
        let surface = display
            .create_surface(owner, "Busy", SurfaceRole::Window, Rect::new(0, 0, 32, 32))
            .unwrap();
        display.focus(surface).unwrap();
        for _ in 0..(MAX_DISPLAY_EVENTS - 2) {
            display.route_key(b'x').unwrap();
        }
        display.commit(owner, surface).unwrap();

        assert_eq!(display.complete_frame(), 0);
        assert!(display.needs_frame());
        assert!(display.poll_event(owner).is_some());
        assert_eq!(display.complete_frame(), 1);
        assert!(!display.needs_frame());

        let mut newest_frame = None;
        while let Some(event) = display.poll_event(owner) {
            if event.kind == DisplayEventKind::FrameDone {
                newest_frame = Some(event.value);
            }
        }
        assert_eq!(newest_frame, Some(1));
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
