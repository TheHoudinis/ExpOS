//! Kernel-side HTTPS admission metadata.
//!
//! TLS records, cryptography, certificate parsing and streaming TCP state run
//! in the sealed Ring 3 network worker. The kernel supplies a trusted RTC time
//! and revalidates the worker's host-scoped trust-anchor identifier.

use crate::port;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TrustAnchor {
    GlobalSignRootR1,
    DigiCertGlobalRootG2,
    IsrgRootX1,
}

impl TrustAnchor {
    const fn id(self) -> u8 {
        match self {
            Self::GlobalSignRootR1 => 1,
            Self::DigiCertGlobalRootG2 => 2,
            Self::IsrgRootX1 => 3,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::GlobalSignRootR1 => "GlobalSign_R1",
            Self::DigiCertGlobalRootG2 => "DigiCert_Global_Root_G2",
            Self::IsrgRootX1 => "ISRG_Root_X1",
        }
    }
}

pub(crate) fn expected_trust_anchor(hostname: &str) -> (u8, &'static str) {
    let anchor = trust_anchor_for(hostname);
    (anchor.id(), anchor.label())
}

fn trust_anchor_for(hostname: &str) -> TrustAnchor {
    const DUCKDUCKGO: &[u8] = b"duckduckgo.com";
    const WIKIPEDIA: &[u8] = b"wikipedia.org";
    const OPEN_METEO: &[u8] = b"open-meteo.com";

    let mut host = hostname.as_bytes();
    if host.last() == Some(&b'.') {
        host = &host[..host.len() - 1];
    }
    let in_domain = |domain: &[u8]| {
        host.eq_ignore_ascii_case(domain)
            || (host.len() > domain.len()
                && host[host.len() - domain.len() - 1] == b'.'
                && host[host.len() - domain.len()..].eq_ignore_ascii_case(domain))
    };
    let is_duckduckgo = in_domain(DUCKDUCKGO);
    if is_duckduckgo {
        TrustAnchor::DigiCertGlobalRootG2
    } else if in_domain(WIKIPEDIA) || in_domain(OPEN_METEO) {
        TrustAnchor::IsrgRootX1
    } else {
        TrustAnchor::GlobalSignRootR1
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

pub(crate) fn rtc_unix_time() -> Option<u64> {
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

    #[test]
    fn selects_duckduckgo_trust_anchor_for_the_domain_and_subdomains() {
        for hostname in [
            "duckduckgo.com",
            "www.duckduckgo.com",
            "html.duckduckgo.com",
            "LITE.DUCKDUCKGO.COM",
            "duckduckgo.com.",
        ] {
            assert_eq!(
                trust_anchor_for(hostname),
                TrustAnchor::DigiCertGlobalRootG2,
                "unexpected trust anchor for {hostname}"
            );
        }
    }

    #[test]
    fn selects_isrg_for_wikipedia_without_trusting_deceptive_suffixes() {
        for hostname in [
            "wikipedia.org",
            "en.wikipedia.org",
            "EN.WIKIPEDIA.ORG.",
            "api.open-meteo.com",
            "geocoding-api.open-meteo.com",
        ] {
            assert_eq!(trust_anchor_for(hostname), TrustAnchor::IsrgRootX1);
        }
        assert_eq!(
            trust_anchor_for("wikipedia.org.example.net"),
            TrustAnchor::GlobalSignRootR1
        );
        assert_eq!(
            trust_anchor_for("open-meteo.com.example.net"),
            TrustAnchor::GlobalSignRootR1
        );
    }

    #[test]
    fn keeps_the_default_anchor_for_unrelated_or_deceptive_hostnames() {
        for hostname in [
            "google.com",
            "youtube.com",
            "notduckduckgo.com",
            "duckduckgo.com.example.net",
            "duckduckgo.invalid",
        ] {
            assert_eq!(
                trust_anchor_for(hostname),
                TrustAnchor::GlobalSignRootR1,
                "unexpected trust anchor for {hostname}"
            );
        }
    }
}
