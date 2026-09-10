//! Verified TLS 1.3 client used by the native HTTPS fetch path.
//!
//! The transport remains synchronous and fixed-capacity, but it does not skip
//! the security properties that distinguish HTTPS from plain TCP: ClientHello
//! entropy comes from RDRAND, the server name is sent with SNI, the certificate
//! chain is validated against an embedded public root, validity dates come
//! from the RTC, and the leaf name/signature are checked before HTTP is sent.

use crate::{network::NetworkError, port, slog};
use core::{
    arch::x86_64::{__cpuid, _rdrand64_step},
    num::NonZeroU32,
};
use embedded_io::Write as _;
use embedded_tls::blocking::{
    Aes128GcmSha256, Certificate, CryptoProvider, TlsConfig, TlsConnection, TlsContext, TlsVerifier,
};
use embedded_tls::{pki::CertVerifier, TlsError};
use rand_core::{CryptoRng, RngCore};

const TLS_RECORD_CAPACITY: usize = 16_640;
const TLS_WRITE_CAPACITY: usize = 4_096;
const TLS_CERTIFICATE_CAPACITY: usize = 12 * 1024;

// GlobalSign Root R1 is the trust anchor for the cross-signed GTS R4 chain
// currently served by Google/YouTube. The source PEM is checked into
// `kernel/trust` and comes from GlobalSign's public certificate repository.
const GLOBAL_SIGN_ROOT_R1: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/global_sign_root_r1.der"));

struct RtcClock;

impl embedded_tls::blocking::TlsClock for RtcClock {
    fn now() -> Option<u64> {
        rtc_unix_time()
    }
}

struct HardwareRng;

impl HardwareRng {
    fn available() -> bool {
        // CPUID.01H:ECX.RDRAND[bit 30].
        __cpuid(1).ecx & (1 << 30) != 0
    }

    fn word() -> Option<u64> {
        if !Self::available() {
            return None;
        }
        for _ in 0..64 {
            let mut value = 0_u64;
            // SAFETY: the CPUID feature bit was checked immediately above.
            if unsafe { _rdrand64_step(&mut value) } == 1 {
                return Some(value);
            }
            core::hint::spin_loop();
        }
        None
    }

    fn failure() -> rand_core::Error {
        rand_core::Error::from(
            NonZeroU32::new(rand_core::Error::CUSTOM_START)
                .expect("rand_core custom error code is non-zero"),
        )
    }
}

impl RngCore for HardwareRng {
    fn next_u32(&mut self) -> u32 {
        self.next_u64() as u32
    }

    fn next_u64(&mut self) -> u64 {
        Self::word().expect("RDRAND failed after availability check")
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        self.try_fill_bytes(destination)
            .expect("RDRAND failed after availability check");
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), rand_core::Error> {
        for chunk in destination.chunks_mut(8) {
            let bytes = Self::word().ok_or_else(Self::failure)?.to_ne_bytes();
            chunk.copy_from_slice(&bytes[..chunk.len()]);
        }
        Ok(())
    }
}

impl CryptoRng for HardwareRng {}

struct VerifiedProvider<'a> {
    rng: HardwareRng,
    verifier: CertVerifier<'a, Aes128GcmSha256, RtcClock, TLS_CERTIFICATE_CAPACITY>,
}

impl CryptoProvider for VerifiedProvider<'_> {
    type CipherSuite = Aes128GcmSha256;
    type Signature = &'static [u8];

    fn rng(&mut self) -> impl embedded_tls::CryptoRngCore {
        &mut self.rng
    }

    fn verifier(&mut self) -> Result<&mut impl TlsVerifier<Aes128GcmSha256>, TlsError> {
        Ok(&mut self.verifier)
    }
}

/// Exchange one HTTP request over an authenticated TLS 1.3 stream.
pub fn https_exchange<Socket>(
    socket: Socket,
    hostname: &str,
    request: &[u8],
    output: &mut [u8],
) -> Result<(usize, bool), NetworkError>
where
    Socket: embedded_io::Read<Error = NetworkError> + embedded_io::Write<Error = NetworkError>,
{
    if !HardwareRng::available() {
        return Err(NetworkError::EntropyUnavailable);
    }
    // Refuse to turn a missing/broken RTC into silently timeless PKI.
    if rtc_unix_time().is_none() {
        return Err(NetworkError::TlsCertificate);
    }

    let mut read_records = [0_u8; TLS_RECORD_CAPACITY];
    let mut write_records = [0_u8; TLS_WRITE_CAPACITY];
    let config = TlsConfig::new()
        .with_server_name(hostname)
        .enable_rsa_signatures();
    let provider = VerifiedProvider {
        rng: HardwareRng,
        verifier: CertVerifier::new(Certificate::X509(GLOBAL_SIGN_ROOT_R1)),
    };
    let mut connection =
        TlsConnection::<_, Aes128GcmSha256>::new(socket, &mut read_records, &mut write_records);
    connection
        .open(TlsContext::new(&config, provider))
        .map_err(map_tls_error)?;
    slog!(
        "HEXA_TLS_VERIFIED host={} version=1.3 suite=AES_128_GCM_SHA256 trust=GlobalSign_R1\r\n",
        hostname
    );

    connection.write_all(request).map_err(map_tls_error)?;
    connection.flush().map_err(map_tls_error)?;
    let mut length = 0;
    let mut truncated = false;
    loop {
        if length == output.len() {
            truncated = true;
            break;
        }
        match connection.read(&mut output[length..]) {
            Ok(0) | Err(TlsError::ConnectionClosed) => break,
            Ok(count) => length += count,
            Err(error) => return Err(map_tls_error(error)),
        }
    }
    // `close_notify` and the TCP FIN are best effort after a complete HTTP
    // response. Verification/read failures above are never hidden.
    let _ = connection.close();
    Ok((length, truncated))
}

fn map_tls_error(error: TlsError) -> NetworkError {
    match error {
        TlsError::InvalidCertificate
        | TlsError::InvalidCertificateEntry
        | TlsError::InvalidCertificateRequest
        | TlsError::InvalidSignature
        | TlsError::InvalidSignatureScheme => NetworkError::TlsCertificate,
        TlsError::Io(embedded_io::ErrorKind::TimedOut) => NetworkError::TcpTimeout,
        TlsError::Io(embedded_io::ErrorKind::ConnectionReset) | TlsError::ConnectionClosed => {
            NetworkError::TcpReset
        }
        TlsError::InvalidRecord
        | TlsError::UnknownContentType
        | TlsError::InvalidApplicationData
        | TlsError::DecodeError
        | TlsError::ParseError(_) => NetworkError::TlsProtocol,
        _ => NetworkError::TlsHandshake,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RtcDateTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

fn rtc_unix_time() -> Option<u64> {
    let first = read_rtc()?;
    let second = read_rtc()?;
    if first != second {
        return None;
    }
    unix_time(first)
}

fn read_rtc() -> Option<RtcDateTime> {
    for _ in 0..100_000 {
        if read_cmos(0x0A) & 0x80 == 0 {
            break;
        }
        core::hint::spin_loop();
    }
    if read_cmos(0x0A) & 0x80 != 0 {
        return None;
    }
    let status_b = read_cmos(0x0B);
    let binary = status_b & 0x04 != 0;
    let twenty_four_hour = status_b & 0x02 != 0;
    let convert = |value: u8| if binary { value } else { bcd(value) };
    let raw_hour = read_cmos(0x04);
    let pm = raw_hour & 0x80 != 0;
    let mut hour = convert(raw_hour & 0x7F);
    if !twenty_four_hour {
        hour %= 12;
        if pm {
            hour += 12;
        }
    }
    let year_low = convert(read_cmos(0x09)) as u16;
    let century = convert(read_cmos(0x32));
    let year = if (19..=99).contains(&century) {
        century as u16 * 100 + year_low
    } else if year_low >= 70 {
        1900 + year_low
    } else {
        2000 + year_low
    };
    Some(RtcDateTime {
        year,
        month: convert(read_cmos(0x08)),
        day: convert(read_cmos(0x07)),
        hour,
        minute: convert(read_cmos(0x02)),
        second: convert(read_cmos(0x00)),
    })
}

fn unix_time(value: RtcDateTime) -> Option<u64> {
    if value.year < 1970
        || !(1..=12).contains(&value.month)
        || value.day == 0
        || value.day > days_in_month(value.year, value.month)
        || value.hour > 23
        || value.minute > 59
        || value.second > 59
    {
        return None;
    }
    let mut days = 0_u64;
    for year in 1970..value.year {
        days += if leap_year(year) { 366 } else { 365 };
    }
    for month in 1..value.month {
        days += days_in_month(value.year, month) as u64;
    }
    days += (value.day - 1) as u64;
    Some(days * 86_400 + value.hour as u64 * 3_600 + value.minute as u64 * 60 + value.second as u64)
}

const fn leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

const fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn read_cmos(register: u8) -> u8 {
    unsafe {
        port::outb(0x70, register | 0x80);
        port::inb(0x71)
    }
}

const fn bcd(value: u8) -> u8 {
    (value & 0x0F) + (value >> 4) * 10
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_epoch_and_leap_dates() {
        assert_eq!(
            unix_time(RtcDateTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            }),
            Some(0)
        );
        assert_eq!(
            unix_time(RtcDateTime {
                year: 2000,
                month: 2,
                day: 29,
                hour: 12,
                minute: 34,
                second: 56,
            }),
            Some(951_827_696)
        );
    }

    #[test]
    fn rejects_invalid_rtc_values() {
        assert_eq!(
            unix_time(RtcDateTime {
                year: 2026,
                month: 2,
                day: 29,
                hour: 0,
                minute: 0,
                second: 0,
            }),
            None
        );
    }
}
