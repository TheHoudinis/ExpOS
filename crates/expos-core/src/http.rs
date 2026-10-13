//! Bounded HTTP response decoding shared by the isolated network worker and host tests.
pub const HTTP_BODY_CAPACITY: usize = 14 * 1024;
const HTTP_LOCATION_CAPACITY: usize = 512;
const HTTP_CONTENT_TYPE_CAPACITY: usize = 96;
const HTTP_COOKIE_CAPACITY: usize = 384;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpError {
    MalformedHttp,
}
pub struct HttpResponse {
    pub status: u16,
    pub body: [u8; HTTP_BODY_CAPACITY],
    pub body_len: usize,
    pub truncated: bool,
    pub peer: [u8; 4],
    location: [u8; HTTP_LOCATION_CAPACITY],
    location_len: usize,
    content_type: [u8; HTTP_CONTENT_TYPE_CAPACITY],
    content_type_len: usize,
    set_cookie: [u8; HTTP_COOKIE_CAPACITY],
    set_cookie_len: usize,
}

impl HttpResponse {
    pub fn body(&self) -> &[u8] {
        &self.body[..self.body_len]
    }

    pub fn location(&self) -> Option<&str> {
        (self.location_len != 0).then(|| {
            // Header parsing accepts visible ASCII only.
            unsafe { core::str::from_utf8_unchecked(&self.location[..self.location_len]) }
        })
    }

    pub fn content_type(&self) -> Option<&str> {
        (self.content_type_len != 0).then(|| {
            // Header parsing accepts visible ASCII only.
            unsafe { core::str::from_utf8_unchecked(&self.content_type[..self.content_type_len]) }
        })
    }

    pub fn set_cookie(&self) -> Option<&str> {
        (self.set_cookie_len != 0).then(|| {
            // Header parsing accepts visible ASCII only.
            unsafe { core::str::from_utf8_unchecked(&self.set_cookie[..self.set_cookie_len]) }
        })
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn parse_ascii_usize(bytes: &[u8]) -> Option<usize> {
    let mut value = 0_usize;
    let mut found = false;
    for byte in bytes.iter().copied() {
        if byte == b' ' || byte == b'\t' {
            continue;
        }
        if !byte.is_ascii_digit() {
            return None;
        }
        found = true;
        value = value.checked_mul(10)?.checked_add((byte - b'0') as usize)?;
    }
    found.then_some(value)
}

fn parse_chunk_size(line: &[u8]) -> Option<usize> {
    let value = line.split(|byte| *byte == b';').next()?;
    let mut result = 0_usize;
    let mut found = false;
    for byte in value.iter().copied() {
        if byte == b' ' || byte == b'\t' {
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => byte - b'0',
            b'a'..=b'f' => byte - b'a' + 10,
            b'A'..=b'F' => byte - b'A' + 10,
            _ => return None,
        };
        found = true;
        result = result.checked_mul(16)?.checked_add(digit as usize)?;
    }
    found.then_some(result)
}

pub fn decode_chunked(input: &[u8], output: &mut [u8]) -> Result<(usize, bool), HttpError> {
    let mut input_offset = 0;
    let mut output_length = 0;
    loop {
        let line_end = find_bytes(&input[input_offset..], b"\r\n")
            .map(|relative| input_offset + relative)
            .ok_or(HttpError::MalformedHttp)?;
        let chunk_length =
            parse_chunk_size(&input[input_offset..line_end]).ok_or(HttpError::MalformedHttp)?;
        input_offset = line_end + 2;
        if chunk_length == 0 {
            return Ok((output_length, false));
        }
        let chunk_end = input_offset
            .checked_add(chunk_length)
            .filter(|end| end.checked_add(2).is_some_and(|tail| tail <= input.len()))
            .ok_or(HttpError::MalformedHttp)?;
        if input[chunk_end..chunk_end + 2] != *b"\r\n" {
            return Err(HttpError::MalformedHttp);
        }
        let remaining = output.len().saturating_sub(output_length);
        let copied = chunk_length.min(remaining);
        output[output_length..output_length + copied]
            .copy_from_slice(&input[input_offset..input_offset + copied]);
        output_length += copied;
        if copied != chunk_length {
            return Ok((output_length, true));
        }
        input_offset = chunk_end + 2;
    }
}

pub fn parse_http_response(
    peer: [u8; 4],
    wire: &[u8],
    wire_truncated: bool,
) -> Result<HttpResponse, HttpError> {
    let header_end = find_bytes(wire, b"\r\n\r\n").ok_or(HttpError::MalformedHttp)?;
    let first_line_end = find_bytes(&wire[..header_end], b"\r\n").unwrap_or(header_end);
    let status_line = &wire[..first_line_end];
    if !status_line.starts_with(b"HTTP/1.") {
        return Err(HttpError::MalformedHttp);
    }
    let first_space = status_line
        .iter()
        .position(|byte| *byte == b' ')
        .ok_or(HttpError::MalformedHttp)?;
    let status_bytes = status_line
        .get(first_space + 1..first_space + 4)
        .ok_or(HttpError::MalformedHttp)?;
    if !status_bytes.iter().all(u8::is_ascii_digit) {
        return Err(HttpError::MalformedHttp);
    }
    let status = ((status_bytes[0] - b'0') as u16) * 100
        + ((status_bytes[1] - b'0') as u16) * 10
        + (status_bytes[2] - b'0') as u16;

    let mut content_length = None;
    let mut chunked = false;
    let mut location = [0_u8; HTTP_LOCATION_CAPACITY];
    let mut location_len = 0;
    let mut content_type = [0_u8; HTTP_CONTENT_TYPE_CAPACITY];
    let mut content_type_len = 0;
    let mut set_cookie = [0_u8; HTTP_COOKIE_CAPACITY];
    let mut set_cookie_len = 0;
    let mut cursor = first_line_end.saturating_add(2);
    while cursor < header_end {
        let line_length =
            find_bytes(&wire[cursor..header_end], b"\r\n").unwrap_or(header_end - cursor);
        let line = &wire[cursor..cursor + line_length];
        if let Some(colon) = line.iter().position(|byte| *byte == b':') {
            let name = &line[..colon];
            let value = &line[colon + 1..];
            if name.eq_ignore_ascii_case(b"content-length") {
                content_length = Some(parse_ascii_usize(value).ok_or(HttpError::MalformedHttp)?);
            } else if name.eq_ignore_ascii_case(b"location") {
                let value = value.trim_ascii();
                if value.is_empty()
                    || value.len() > location.len()
                    || !value.iter().all(|byte| (0x21..=0x7E).contains(byte))
                {
                    return Err(HttpError::MalformedHttp);
                }
                location[..value.len()].copy_from_slice(value);
                location_len = value.len();
            } else if name.eq_ignore_ascii_case(b"content-type") {
                let value = value.trim_ascii();
                if value.is_empty()
                    || value.len() > content_type.len()
                    || !value.iter().all(|byte| (0x20..=0x7E).contains(byte))
                {
                    return Err(HttpError::MalformedHttp);
                }
                content_type[..value.len()].copy_from_slice(value);
                content_type_len = value.len();
            } else if name.eq_ignore_ascii_case(b"set-cookie") && set_cookie_len == 0 {
                let value = value.trim_ascii();
                if value.is_empty()
                    || value.len() > set_cookie.len()
                    || !value.iter().all(|byte| (0x20..=0x7E).contains(byte))
                {
                    return Err(HttpError::MalformedHttp);
                }
                set_cookie[..value.len()].copy_from_slice(value);
                set_cookie_len = value.len();
            } else if name.eq_ignore_ascii_case(b"transfer-encoding")
                && value
                    .windows(7)
                    .any(|part| part.eq_ignore_ascii_case(b"chunked"))
            {
                chunked = true;
            }
        }
        cursor += line_length + 2;
    }

    let encoded_body = &wire[header_end + 4..];
    let mut response = HttpResponse {
        status,
        body: [0; HTTP_BODY_CAPACITY],
        body_len: 0,
        truncated: false,
        peer,
        location,
        location_len,
        content_type,
        content_type_len,
        set_cookie,
        set_cookie_len,
    };
    if chunked {
        let (length, truncated) = decode_chunked(encoded_body, &mut response.body)?;
        response.body_len = length;
        response.truncated = truncated || wire_truncated;
    } else {
        let expected = content_length.unwrap_or(encoded_body.len());
        if encoded_body.len() < expected && !wire_truncated {
            return Err(HttpError::MalformedHttp);
        }
        let available = expected.min(encoded_body.len());
        response.body_len = available.min(response.body.len());
        response.body[..response.body_len].copy_from_slice(&encoded_body[..response.body_len]);
        response.truncated = expected > response.body.len()
            || available < expected
            || (content_length.is_none() && wire_truncated);
    }
    Ok(response)
}

impl HttpResponse {
    pub fn encode(&self, out: &mut [u8]) -> Result<usize, HttpError> {
        let length =
            16 + self.body_len + self.location_len + self.content_type_len + self.set_cookie_len;
        if out.len() < length {
            return Err(HttpError::MalformedHttp);
        }
        out[..2].copy_from_slice(&self.status.to_le_bytes());
        out[2] = self.truncated as u8;
        out[3] = 0;
        for (at, value) in [
            (4, self.body_len),
            (6, self.location_len),
            (8, self.content_type_len),
            (10, self.set_cookie_len),
        ] {
            out[at..at + 2].copy_from_slice(&(value as u16).to_le_bytes());
        }
        out[12..16].copy_from_slice(&self.peer);
        let mut cursor = 16;
        for data in [
            self.body(),
            &self.location[..self.location_len],
            &self.content_type[..self.content_type_len],
            &self.set_cookie[..self.set_cookie_len],
        ] {
            out[cursor..cursor + data.len()].copy_from_slice(data);
            cursor += data.len();
        }
        Ok(length)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, HttpError> {
        if bytes.len() < 16 || bytes[2] > 1 || bytes[3] != 0 {
            return Err(HttpError::MalformedHttp);
        }
        let word = |at| u16::from_le_bytes([bytes[at], bytes[at + 1]]) as usize;
        let (body, location, content_type, cookie) = (word(4), word(6), word(8), word(10));
        if body > HTTP_BODY_CAPACITY
            || location > HTTP_LOCATION_CAPACITY
            || content_type > HTTP_CONTENT_TYPE_CAPACITY
            || cookie > HTTP_COOKIE_CAPACITY
            || bytes.len() != 16 + body + location + content_type + cookie
            || !(100..=599).contains(&word(0))
        {
            return Err(HttpError::MalformedHttp);
        }
        let mut response = Self {
            status: word(0) as u16,
            body: [0; HTTP_BODY_CAPACITY],
            body_len: body,
            truncated: bytes[2] != 0,
            peer: bytes[12..16].try_into().unwrap(),
            location: [0; HTTP_LOCATION_CAPACITY],
            location_len: location,
            content_type: [0; HTTP_CONTENT_TYPE_CAPACITY],
            content_type_len: content_type,
            set_cookie: [0; HTTP_COOKIE_CAPACITY],
            set_cookie_len: cookie,
        };
        let mut cursor = 16;
        response.body[..body].copy_from_slice(&bytes[cursor..cursor + body]);
        cursor += body;
        for (out, length, minimum) in [
            (&mut response.location[..], location, 0x21),
            (&mut response.content_type[..], content_type, 0x20),
            (&mut response.set_cookie[..], cookie, 0x20),
        ] {
            let data = &bytes[cursor..cursor + length];
            if !data.iter().all(|byte| (minimum..=0x7e).contains(byte)) {
                return Err(HttpError::MalformedHttp);
            }
            out[..length].copy_from_slice(data);
            cursor += length;
        }
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_reply_roundtrip_and_corruption() {
        let response = parse_http_response(
            [1, 2, 3, 4],
            b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\nLocation: /next\r\n\r\nhi",
            false,
        )
        .unwrap();
        let mut wire = [0; 16384];
        let length = response.encode(&mut wire).unwrap();
        let decoded = HttpResponse::decode(&wire[..length]).unwrap();
        assert_eq!(decoded.body(), b"hi");
        assert_eq!(decoded.location(), Some("/next"));
        for end in 0..length {
            assert!(HttpResponse::decode(&wire[..end]).is_err());
        }
        wire[2] = 2;
        assert!(HttpResponse::decode(&wire[..length]).is_err());
        wire[2] = 0;
        wire[length - 1] = 0xff;
        assert!(HttpResponse::decode(&wire[..length]).is_err());
    }
}
