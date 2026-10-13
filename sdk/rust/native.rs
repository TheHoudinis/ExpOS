// Standalone, no-std EXF1 transport. Include this file in a freestanding Form.
// The loader initializes caller/version; all buffers stay in its ABI page.
use core::arch::asm;

#[repr(C)]
pub struct NativeRequest {
    pub version: u16,
    pub call: u16,
    pub caller: [u8; 16],
    pub handle: u32,
    pub arguments: [u64; 6],
}
#[repr(C)]
pub struct NativeResponse {
    pub status: u16,
    pub reserved: [u8; 6],
    pub values: [u64; 4],
}
const _: [(); 72] = [(); core::mem::size_of::<NativeRequest>()];
const _: [(); 40] = [(); core::mem::size_of::<NativeResponse>()];

pub struct NativeContext {
    request: *mut NativeRequest,
    response: *mut NativeResponse,
    execute: u32,
    log: u32,
}
impl NativeContext {
    /// # Safety
    /// Use only the exact ABI pointers and Handle IDs supplied to EXF1 `_start`.
    pub unsafe fn from_entry(
        request: *mut NativeRequest,
        response: *mut NativeResponse,
        execute: u32,
        log: u32,
    ) -> Self {
        Self {
            request,
            response,
            execute,
            log,
        }
    }
    #[inline(always)]
    unsafe fn call(&mut self) {
        asm!("int 0x80", inout("rax") 0x4558504f53414249_u64 => _, in("rdi") self.request, in("rsi") self.response);
    }
    /// Copy a bounded message into the admitted ABI page and invoke LOG.
    #[inline(always)]
    pub fn log(&mut self, bytes: &[u8]) -> bool {
        if bytes.len() > 512 {
            return false;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.request.cast::<u8>().add(256),
                bytes.len(),
            );
            (*self.request).call = 1;
            (*self.request).handle = self.log;
            (*self.request).arguments[0] = 256;
            (*self.request).arguments[1] = bytes.len() as u64;
            self.call();
            (*self.response).status == 0
        }
    }
    #[inline(always)]
    pub fn exit(&mut self, result: u64) -> ! {
        unsafe {
            (*self.request).call = 8;
            (*self.request).handle = self.execute;
            (*self.request).arguments[0] = result;
            self.call();
            asm!("ud2", options(noreturn));
        }
    }
}
