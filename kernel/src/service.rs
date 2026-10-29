//! Owned, restartable CPL3 workers. No service receives device or ambient rights.
use crate::form_runtime::{self, RunAuthorization, SliceReason};
use alloc::{
    alloc::{alloc_zeroed, Layout},
    boxed::Box,
};
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use expos_core::{Authority, CapabilityBroker, CfcFin, Fin, Operations};

pub const BASE: u64 = 0x400000200000;
pub const MEMORY_BYTES: usize = 2 * 1024 * 1024;
pub const INPUT_OFFSET: usize = 0x180000;
pub const INPUT_BYTES: usize = 64 * 1024;
// The network worker can hold a 60 KiB bounded TCP reply plus TLS/parser
// frames. Keep a guard gap after the 64 KiB input and give that isolated
// worker enough private stack without enlarging its fixed 2 MiB arena.
pub const STACK_OFFSET: usize = 0x1a0000;
pub const STACK_BYTES: usize = 256 * 1024;
pub const OUTPUT_BYTES: usize = 64 * 1024;
const DIMENSION: Fin = Fin::from_u128(0x53455256494345000000000000000001);
const REPLY: Fin = Fin::from_u128(0x53455256494345000000000000000002);
static ACTIVE: AtomicBool = AtomicBool::new(false);

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static FAULTS: AtomicU64 = AtomicU64::new(0);
static BUDGETS: AtomicU64 = AtomicU64::new(0);
static REJECTED: AtomicU64 = AtomicU64::new(0);
static REPLY_BYTES: AtomicU64 = AtomicU64::new(0);

pub fn statistics(mut output: impl FnMut(core::fmt::Arguments<'_>)) {
    output(format_args!(
        "Ring 3 workers: active={} requests={} completed={}",
        ACTIVE.load(Ordering::Acquire),
        REQUESTS.load(Ordering::Relaxed),
        COMPLETED.load(Ordering::Relaxed)
    ));
    output(format_args!(
        "faults={} budget-stops={} rejected={} reply-bytes={}",
        FAULTS.load(Ordering::Relaxed),
        BUDGETS.load(Ordering::Relaxed),
        REJECTED.load(Ordering::Relaxed),
        REPLY_BYTES.load(Ordering::Relaxed)
    ));
    output(format_args!(
        "ELF arena={} KiB; stack={} KiB; replies<={} KiB; W^X required",
        MEMORY_BYTES / 1024,
        STACK_BYTES / 1024,
        OUTPUT_BYTES / 1024
    ));
}

#[repr(C, align(4096))]
pub struct Memory {
    pub table: [u64; 512],
    pub bytes: [u8; MEMORY_BYTES],
}

pub struct Output {
    pub bytes: [u8; OUTPUT_BYTES],
    pub length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Busy,
    Unsupported,
    InvalidImage,
    InputTooLarge,
    NoMemory,
    Admission,
    Fault(u8),
    Budget,
}

pub struct Report {
    pub output: Box<Output>,
    pub result: u64,
    pub ticks: u64,
}

impl Memory {
    pub fn load(elf: &[u8], input: &[u8]) -> Result<(Box<Self>, u64), Error> {
        if input.len() > INPUT_BYTES {
            return Err(Error::InputTooLarge);
        }
        let image = Image::parse(elf)?;
        let pointer = unsafe { alloc_zeroed(Layout::new::<Self>()) }.cast::<Self>();
        if pointer.is_null() {
            return Err(Error::NoMemory);
        }
        let mut memory = unsafe { Box::from_raw(pointer) };
        for segment in image.segments.iter().flatten() {
            let start = (segment.address - BASE) as usize;
            memory.bytes[start..start + segment.file_size]
                .copy_from_slice(&elf[segment.offset..segment.offset + segment.file_size]);
            let flags = 1
                | 4
                | if segment.flags & 2 != 0 { 2 } else { 0 }
                | if segment.flags & 1 == 0 { 1 << 63 } else { 0 };
            memory.map(start, segment.memory_size, flags);
        }
        memory.bytes[INPUT_OFFSET..INPUT_OFFSET + input.len()].copy_from_slice(input);
        memory.map(INPUT_OFFSET, INPUT_BYTES, 1 | 4 | (1 << 63));
        memory.map(STACK_OFFSET, STACK_BYTES, 1 | 2 | 4 | (1 << 63));
        Ok((memory, image.entry))
    }

    fn map(&mut self, offset: usize, length: usize, flags: u64) {
        for page in offset / 4096..(offset + length).div_ceil(4096) {
            self.table[page] = (self.bytes.as_ptr() as u64 + (page * 4096) as u64) | flags;
        }
    }

    /// Translate a writable user range into the service arena's contiguous
    /// supervisor alias. This is used only to pin a short-lived DMA aperture
    /// for the admitted network driver worker; executable/read-only/input
    /// pages and addresses outside the private arena are rejected.
    pub(crate) fn writable_physical_range(
        &self,
        user_address: u64,
        length: usize,
    ) -> Option<usize> {
        let offset = usize::try_from(user_address.checked_sub(BASE)?).ok()?;
        let end = offset.checked_add(length)?;
        if length == 0 || end > INPUT_OFFSET {
            return None;
        }
        let required = 1 | 2 | 4 | (1_u64 << 63);
        for page in offset / 4096..end.div_ceil(4096) {
            if self.table[page] & required != required {
                return None;
            }
        }
        Some(self.bytes.as_ptr() as usize + offset)
    }

    /// Translate a short-lived writable service range for a kernel copy. This
    /// includes the private stack but never grants DMA ownership or retains the
    /// alias after the ABI call returns.
    pub(crate) fn writable_alias_range(&self, user_address: u64, length: usize) -> Option<usize> {
        let offset = usize::try_from(user_address.checked_sub(BASE)?).ok()?;
        let end = offset.checked_add(length)?;
        if length == 0 || end > MEMORY_BYTES {
            return None;
        }
        let required = 1 | 2 | 4 | (1_u64 << 63);
        for page in offset / 4096..end.div_ceil(4096) {
            if self.table[page] & required != required {
                return None;
            }
        }
        Some(self.bytes.as_ptr() as usize + offset)
    }
}

#[derive(Clone, Copy)]
struct Segment {
    address: u64,
    offset: usize,
    file_size: usize,
    memory_size: usize,
    flags: u32,
}
struct Image {
    entry: u64,
    segments: [Option<Segment>; 8],
}

impl Image {
    fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() < 64
            || &bytes[..7] != b"\x7fELF\x02\x01\x01"
            || bytes[16..20] != [2, 0, 62, 0]
        {
            return Err(Error::InvalidImage);
        }
        let u16_at = |at| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap()) as usize;
        let u64_at = |at| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let ph = usize::try_from(u64_at(32)).map_err(|_| Error::InvalidImage)?;
        let count = u16_at(56);
        if u16_at(54) != 56
            || count == 0
            || count > 8
            || ph
                .checked_add(count * 56)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(Error::InvalidImage);
        }
        let mut image = Self {
            entry: u64_at(24),
            segments: [None; 8],
        };
        let mut executable_entry = false;
        for i in 0..count {
            let at = ph + i * 56;
            if bytes[at..at + 4] != [1, 0, 0, 0] {
                continue;
            }
            let flags = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
            let offset = usize::try_from(u64_at(at + 8)).map_err(|_| Error::InvalidImage)?;
            let address = u64_at(at + 16);
            let file_size = usize::try_from(u64_at(at + 32)).map_err(|_| Error::InvalidImage)?;
            let memory_size = usize::try_from(u64_at(at + 40)).map_err(|_| Error::InvalidImage)?;
            if memory_size == 0 {
                continue;
            }
            let end = address
                .checked_add(memory_size as u64)
                .ok_or(Error::InvalidImage)?;
            if ![4, 5, 6].contains(&flags)
                || address < BASE
                || address % 4096 != 0
                || end > BASE + INPUT_OFFSET as u64
                || file_size > memory_size
                || offset
                    .checked_add(file_size)
                    .is_none_or(|end| end > bytes.len())
            {
                return Err(Error::InvalidImage);
            }
            for previous in image.segments.iter().flatten() {
                let previous_end =
                    previous.address + previous.memory_size.div_ceil(4096) as u64 * 4096;
                if address < previous_end && previous.address < end.div_ceil(4096) * 4096 {
                    return Err(Error::InvalidImage);
                }
            }
            executable_entry |=
                flags == 5 && (address..address + file_size as u64).contains(&image.entry);
            image.segments[i] = Some(Segment {
                address,
                offset,
                file_size,
                memory_size,
                flags,
            });
        }
        if !executable_entry {
            return Err(Error::InvalidImage);
        }
        Ok(image)
    }
}

pub fn run(
    elf: &[u8],
    input: &[u8],
    cfc: CfcFin,
    form: Fin,
    tick_limit: u64,
) -> Result<Report, Error> {
    run_counted(elf, input, cfc, form, tick_limit, None, false)
}

/// Run the built-in network worker with the sole additional authority to lease
/// its private DMA arena and use the confined RTL8139 register portal.
/// Applications never receive this Handle directly.
pub fn run_network(
    elf: &[u8],
    input: &[u8],
    cfc: CfcFin,
    form: Fin,
    tick_limit: u64,
) -> Result<Report, Error> {
    run_counted(elf, input, cfc, form, tick_limit, None, true)
}

#[cfg(test)]
pub fn run_image(_: &[u8], _: CfcFin, _: Fin, _: Authority) -> Result<Report, Error> {
    Err(Error::Unsupported)
}

/// Execute an explicitly selected EXF1 Form with captured LOG output.
#[cfg(not(test))]
pub fn run_image(
    image: &[u8],
    cfc: CfcFin,
    form: Fin,
    authority: Authority,
) -> Result<Report, Error> {
    expos_core::NativeImage::parse(image).map_err(|_| Error::InvalidImage)?;
    if authority == Authority::Guest {
        return Err(Error::Admission);
    }
    run_counted(image, &[], cfc, form, 256, Some(authority), false)
}

fn run_counted(
    elf: &[u8],
    input: &[u8],
    cfc: CfcFin,
    form: Fin,
    tick_limit: u64,
    native_authority: Option<Authority>,
    network_service: bool,
) -> Result<Report, Error> {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    let result = run_inner(
        elf,
        input,
        cfc,
        form,
        tick_limit,
        native_authority,
        network_service,
    );
    match &result {
        Ok(report) => {
            COMPLETED.fetch_add(1, Ordering::Relaxed);
            REPLY_BYTES.fetch_add(report.output.length as u64, Ordering::Relaxed);
        }
        Err(Error::Fault(_)) => {
            FAULTS.fetch_add(1, Ordering::Relaxed);
        }
        Err(Error::Budget) => {
            BUDGETS.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            REJECTED.fetch_add(1, Ordering::Relaxed);
        }
    }
    if let Err(error) = result {
        crate::slog!("EXPOS_SERVICE_REJECT form={} error={:?}\r\n", form, error);
    }
    result
}

fn run_inner(
    elf: &[u8],
    input: &[u8],
    cfc: CfcFin,
    form: Fin,
    tick_limit: u64,
    native_authority: Option<Authority>,
    network_service: bool,
) -> Result<Report, Error> {
    if !crate::security::nx_active() {
        return Err(Error::Unsupported);
    }
    if ACTIVE.swap(true, Ordering::AcqRel) {
        return Err(Error::Busy);
    }
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            ACTIVE.store(false, Ordering::Release);
        }
    }
    let _release = Release;
    let mut broker = CapabilityBroker::new(cfc);
    let execute = broker
        .issue_for(
            form,
            native_authority.unwrap_or(Authority::Power),
            form,
            DIMENSION,
            Operations::EXECUTE,
            u64::MAX,
        )
        .map_err(|_| Error::Admission)?;
    let reply = broker
        .issue_for(
            form,
            native_authority.unwrap_or(Authority::Power),
            REPLY,
            DIMENSION,
            if native_authority.is_some() {
                Operations::READ
            } else {
                Operations::EXECUTE
            },
            u64::MAX,
        )
        .map_err(|_| Error::Admission)?;
    let network = if network_service {
        Some(
            broker
                .issue_for(
                    form,
                    Authority::Power,
                    crate::network::NETWORK_FIN,
                    DIMENSION,
                    Operations::NETWORK,
                    u64::MAX,
                )
                .map_err(|_| Error::Admission)?,
        )
    } else {
        None
    };
    broker
        .seal_context(form, DIMENSION)
        .map_err(|_| Error::Admission)?;
    let mut prepared = if native_authority.is_some() {
        let prepared =
            form_runtime::prepare(form, execute, reply, elf).map_err(|_| Error::Admission)?;
        form_runtime::capture_output(prepared);
        prepared
    } else {
        form_runtime::prepare_service(form, execute, reply, elf, input, network)?
    };
    crate::slog!(
        "EXPOS_SERVICE_ENTER fin={} ring=3 input={} limit={}\r\n",
        form,
        input.len(),
        tick_limit
    );
    let mut authorization = RunAuthorization::new(&broker, cfc, DIMENSION, form, execute, reply);
    if let Some(network) = network {
        authorization = authorization.with_network(network);
    }
    let mut ticks = 0;
    let result = loop {
        let slice = form_runtime::run_slice(
            &mut prepared,
            authorization,
            tick_limit.saturating_sub(ticks).max(1),
        );
        ticks += slice.timer_ticks;
        match slice.reason {
            SliceReason::Exited => break Ok(slice.result),
            SliceReason::Fault(vector) => break Err(Error::Fault(vector)),
            SliceReason::BudgetExhausted => break Err(Error::Budget),
            SliceReason::Preempted if ticks >= tick_limit => break Err(Error::Budget),
            SliceReason::Preempted => {}
        }
    };
    if network.is_some() {
        crate::network::service_driver_release();
    }
    let output = form_runtime::take_service_output(prepared);
    form_runtime::finish(prepared);
    crate::slog!(
        "EXPOS_SERVICE_EXIT fin={} ticks={} result={:?}\r\n",
        form,
        ticks,
        result
    );
    Ok(Report {
        output: output.ok_or(Error::Admission)?,
        result: result?,
        ticks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn elf() -> [u8; 128] {
        let mut bytes = [0; 128];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16..20].copy_from_slice(&[2, 0, 62, 0]);
        bytes[24..32].copy_from_slice(&BASE.to_le_bytes());
        bytes[32..40].copy_from_slice(&64_u64.to_le_bytes());
        bytes[54] = 56;
        bytes[56] = 1;
        bytes[64] = 1;
        bytes[68] = 5;
        bytes[72] = 120;
        bytes[80..88].copy_from_slice(&BASE.to_le_bytes());
        bytes[96] = 8;
        bytes[104] = 8;
        bytes
    }
    #[test]
    #[ignore = "bounded parser mutation campaign; run make parser-fuzz"]
    fn service_image_parser_fuzz() {
        crate::parser_fuzz::campaign("service-elf", &[&elf()], |bytes| {
            if let Ok(image) = Image::parse(bytes) {
                assert!(image.entry >= BASE && image.entry < BASE + INPUT_OFFSET as u64);
                for segment in image.segments.iter().flatten() {
                    assert!([4, 5, 6].contains(&segment.flags));
                    assert!(segment.address >= BASE);
                    assert!(
                        segment.address + segment.memory_size as u64 <= BASE + INPUT_OFFSET as u64
                    );
                }
            }
        });
    }

    #[test]
    fn rejects_elf_truncation_permissions_and_ranges() {
        let good = elf();
        assert!(Image::parse(&good).is_ok());
        for end in 0..good.len() {
            assert!(Image::parse(&good[..end]).is_err());
        }
        for (offset, byte) in [(68, 7), (80, 1), (104, 1), (56, 9), (54, 1), (24, 9)] {
            let mut bad = good;
            bad[offset] = byte;
            assert!(Image::parse(&bad).is_err());
        }
    }
}
