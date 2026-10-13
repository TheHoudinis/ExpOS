//! ExpPython runs the real MicroPython VM in an owned, restartable CPL3 service.
use expos_core::{CfcFin, Fin};

const PYTHON_FIN: Fin = Fin::from_u128(0x505954484f4e00000000000000000001);
#[cfg(not(test))]
const IMAGE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/python-service.elf"));

pub fn run(source: &str, cfc: CfcFin) -> Result<crate::service::Report, crate::service::Error> {
    if !crate::security::python_allowed() {
        return Err(crate::service::Error::Unsupported);
    }
    if source.len() > 4096 {
        return Err(crate::service::Error::InputTooLarge);
    }
    #[cfg(not(test))]
    {
        crate::service::run(IMAGE, source.as_bytes(), cfc, PYTHON_FIN, 2000)
    }
    #[cfg(test)]
    {
        let _ = (cfc, PYTHON_FIN);
        Err(crate::service::Error::Unsupported)
    }
}

pub fn execute(source: &str, cfc: CfcFin) -> bool {
    let report = match run(source, cfc) {
        Ok(report) => report,
        Err(error) => {
            crate::println!("ExpPython stopped: {:?}.", error);
            crate::slog!("EXPOS_PYTHON_SERVICE_ERROR error={:?}\r\n", error);
            return false;
        }
    };
    for byte in &report.output.bytes[..report.output.length] {
        crate::print!("{}", if byte.is_ascii() { *byte as char } else { '?' });
    }
    match report.result {
        0 => crate::slog!("EXPOS_PYTHON_OK ring=3 ticks={}\r\n", report.ticks),
        1 => crate::println!("ExpPython: execution budget exhausted; script stopped."),
        2 => crate::println!("ExpPython: output budget exhausted; script stopped."),
        3 => crate::println!("ExpPython: script raised an exception."),
        _ => crate::println!("ExpPython: script exceeds 4096 bytes."),
    }
    report.result == 0
}
