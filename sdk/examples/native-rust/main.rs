#![no_std]
#![no_main]
include!(concat!(env!("EXPOS_SDK"), "/rust/native.rs"));

#[no_mangle]
#[link_section = ".text.entry"]
pub unsafe extern "C" fn _start(
    request: *mut NativeRequest,
    response: *mut NativeResponse,
    execute: u32,
    log: u32,
) -> ! {
    let mut context = NativeContext::from_entry(request, response, execute, log);
    context.log(b"Hello from compiled Rust in Ring 3\n");
    context.exit(43)
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    unsafe { core::arch::asm!("ud2", options(noreturn)) }
}
