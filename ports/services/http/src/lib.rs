#![no_std]
use core::arch::asm;
use expos_core::{parse_http_response, AbiCall, AbiRequest, AbiResponse};

unsafe fn call(request: *mut AbiRequest, response: *mut AbiResponse) {
    let mut magic = 0x4558504f53414249_u64;
    asm!("int 0x80", inout("rax") magic, in("rdi") request, in("rsi") response);
    let _ = magic;
}

/// Built-in service entry point.
///
/// # Safety
/// The loader must supply its mapped ABI request/response page, valid Handle
/// IDs, and an immutable input allocation covering `length` bytes. The service
/// image and private stack must remain mapped until it exits.
#[no_mangle]
#[link_section = ".text.entry"]
pub unsafe extern "C" fn _start(
    request: *mut AbiRequest,
    response: *mut AbiResponse,
    execute: u32,
    reply: u32,
    input: *const u8,
    length: usize,
) -> ! {
    let input = core::slice::from_raw_parts(input, length);
    let mut result = 1;
    if input.len() >= 5 && input.len() <= 16389 && input[4] <= 1 {
        if let Ok(parsed) =
            parse_http_response(input[..4].try_into().unwrap(), &input[5..], input[4] != 0)
        {
            let mut encoded = [0_u8; 16384];
            if let Ok(length) = parsed.encode(&mut encoded) {
                result = 0;
                for bytes in encoded[..length].chunks(512) {
                    core::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        (request as *mut u8).add(256),
                        bytes.len(),
                    );
                    (*request).call = AbiCall::IpcSend as u16;
                    (*request).handle_id = reply;
                    (*request).arguments = [256, bytes.len() as u64, 0, 0, 0, 0];
                    call(request, response);
                    if (*response).status != 0 {
                        result = 2;
                        break;
                    }
                }
            }
        }
    }
    (*request).call = AbiCall::ExecutionExit as u16;
    (*request).handle_id = execute;
    (*request).arguments = [result, 0, 0, 0, 0, 0];
    call(request, response);
    asm!("ud2", options(noreturn));
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        asm!("ud2", options(noreturn));
    }
}
