//! Allocation-free PCI configuration and capability discovery.
//!
//! Drivers use this module instead of open-coding CF8/CFC cycles.  It is the
//! first concrete ASL resource boundary: a driver receives a [`Function`] and
//! may only decode BARs or capabilities belonging to that function.

use crate::port;

const CONFIG_ADDRESS: u16 = 0xCF8;
const CONFIG_DATA: u16 = 0xCFC;
const COMMAND_MEMORY: u16 = 1 << 1;
const COMMAND_BUS_MASTER: u16 = 1 << 2;
const STATUS_CAPABILITIES: u16 = 1 << 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Location {
    pub bus: u8,
    pub slot: u8,
    pub function: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Function {
    pub location: Location,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub programming_interface: u8,
    pub revision: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MemoryBar {
    pub address: u64,
    pub prefetchable: bool,
    pub is_64_bit: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capability {
    pub id: u8,
    pub offset: u8,
}

impl Function {
    pub fn read(self, offset: u8) -> u32 {
        read(self.location, offset)
    }

    pub fn write(self, offset: u8, value: u32) {
        write(self.location, offset, value);
    }

    /// Enable MMIO decoding and DMA for a claimed device while preserving all
    /// firmware-selected command bits.
    pub fn enable_memory_bus_master(self) {
        let value = self.read(0x04);
        let command = value as u16 | COMMAND_MEMORY | COMMAND_BUS_MASTER;
        self.write(0x04, (value & 0xFFFF_0000) | command as u32);
    }

    pub fn memory_bar(self, index: u8) -> Option<MemoryBar> {
        if index >= 6 {
            return None;
        }
        let offset = 0x10 + index * 4;
        let low = self.read(offset);
        if low == 0 || low == u32::MAX || low & 1 != 0 {
            return None;
        }
        let kind = (low >> 1) & 0x3;
        let (address, is_64_bit) = match kind {
            0 => ((low & !0xF) as u64, false),
            2 if index < 5 => {
                let high = self.read(offset + 4);
                (((high as u64) << 32) | (low & !0xF) as u64, true)
            }
            _ => return None,
        };
        (address != 0).then_some(MemoryBar {
            address,
            prefetchable: low & (1 << 3) != 0,
            is_64_bit,
        })
    }

    /// Locate a conventional PCI capability. Traversal is bounded and rejects
    /// malformed, unaligned, cyclic or header-overlapping lists.
    pub fn capability(self, requested: u8) -> Option<Capability> {
        if (self.read(0x04) >> 16) as u16 & STATUS_CAPABILITIES == 0 {
            return None;
        }
        let mut offset = (self.read(0x34) as u8) & !0x3;
        let mut visited = [false; 64];
        for _ in 0..48 {
            if !(0x40..=0xFC).contains(&offset) {
                return None;
            }
            let slot = (offset / 4) as usize;
            if visited[slot] {
                return None;
            }
            visited[slot] = true;
            let header = self.read(offset);
            let id = header as u8;
            if id == requested {
                return Some(Capability { id, offset });
            }
            offset = ((header >> 8) as u8) & !0x3;
            if offset == 0 {
                return None;
            }
        }
        None
    }
}

pub fn find_class(class_code: u8, subclass: u8, interface: Option<u8>) -> Option<Function> {
    find(|function| {
        function.class_code == class_code
            && function.subclass == subclass
            && interface.is_none_or(|value| function.programming_interface == value)
    })
}

pub fn find(mut matches: impl FnMut(Function) -> bool) -> Option<Function> {
    visit(|function| {
        if matches(function) {
            Some(function)
        } else {
            None
        }
    })
}

pub fn visit<T>(mut visitor: impl FnMut(Function) -> Option<T>) -> Option<T> {
    for bus in 0_u16..=255 {
        for slot in 0_u8..32 {
            let location = Location {
                bus: bus as u8,
                slot,
                function: 0,
            };
            let Some(primary) = function(location) else {
                continue;
            };
            if let Some(result) = visitor(primary) {
                return Some(result);
            }
            let functions = if read(location, 0x0C) & (1 << 23) != 0 {
                8
            } else {
                1
            };
            for number in 1..functions {
                let location = Location {
                    function: number,
                    ..location
                };
                if let Some(candidate) = function(location) {
                    if let Some(result) = visitor(candidate) {
                        return Some(result);
                    }
                }
            }
        }
    }
    None
}

pub fn function(location: Location) -> Option<Function> {
    let identity = read(location, 0);
    let vendor_id = identity as u16;
    if vendor_id == u16::MAX {
        return None;
    }
    let class = read(location, 0x08);
    Some(Function {
        location,
        vendor_id,
        device_id: (identity >> 16) as u16,
        class_code: (class >> 24) as u8,
        subclass: (class >> 16) as u8,
        programming_interface: (class >> 8) as u8,
        revision: class as u8,
    })
}

pub fn read(location: Location, offset: u8) -> u32 {
    let address = config_address(location, offset);
    unsafe {
        port::outl(CONFIG_ADDRESS, address);
        port::inl(CONFIG_DATA)
    }
}

pub fn write(location: Location, offset: u8, value: u32) {
    let address = config_address(location, offset);
    unsafe {
        port::outl(CONFIG_ADDRESS, address);
        port::outl(CONFIG_DATA, value);
    }
}

const fn config_address(location: Location, offset: u8) -> u32 {
    0x8000_0000
        | ((location.bus as u32) << 16)
        | ((location.slot as u32) << 11)
        | ((location.function as u32) << 8)
        | (offset as u32 & 0xFC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_address_encodes_bdf_and_aligned_offset() {
        assert_eq!(
            config_address(
                Location {
                    bus: 0xAB,
                    slot: 0x1D,
                    function: 6,
                },
                0x13,
            ),
            0x80AB_EE10
        );
    }
}
