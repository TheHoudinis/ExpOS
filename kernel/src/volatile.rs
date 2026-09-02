use core::cell::UnsafeCell;

/// Minimal volatile wrapper for MMIO registers.
///
/// The compiler is free to reorder/elide ordinary memory accesses; reads
/// and writes that go through this wrapper are performed with
/// `read_volatile`/`write_volatile`, which the compiler will neither
/// elide nor reorder against each other.
pub struct Volatile<T> {
    value: UnsafeCell<T>,
}

impl<T: Copy> Volatile<T> {
    pub fn read(&self) -> T {
        unsafe { core::ptr::read_volatile(self.value.get()) }
    }

    pub fn write(&self, value: T) {
        unsafe { core::ptr::write_volatile(self.value.get(), value) }
    }
}
