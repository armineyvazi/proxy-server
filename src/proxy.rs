//! Pure HTTP/1.1 parsing and rewriting. No sockets or process state — byte
//! slices in, parsed structs or rewritten bytes out.

/// A parsed request. Borrows from the input buffer (httparse is zero-copy).
pub struct ParsedRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub version: u8, // HTTP minor version: 0 for 1.0, 1 for 1.1
    pub headers: Vec<httparse::Header<'a>>,
    pub body_offset: usize,
}

/// Outcome of parsing a request out of a partially filled buffer.
pub enum ParseStatus<'a> {
    Complete(ParsedRequest<'a>),
    Partial,
    /// Malformed beyond recovery; the u16 is the status code to return.
    Invalid(u16),
}

/// How a request body is delimited.
#[derive(Debug, PartialEq, Eq)]
pub enum BodyFraming {
    None,
    ContentLength(u64),
    Chunked,
}

/// Parse a request from `buf`. `Partial` means the header section is incomplete;
/// `Invalid` carries the status code (431 for too many headers, 400 otherwise).
pub fn parse_request<'a>(
    buf: &'a [u8],
    header_storage: &'a mut [httparse::Header<'a>],
) -> ParseStatus<'a> {
    let mut req = httparse::Request::new(header_storage);
    match req.parse(buf) {
        Ok(httparse::Status::Complete(body_start)) => {
            let (method, path) = match (req.method, req.path) {
                (Some(m), Some(p)) => (m, p),
                _ => return ParseStatus::Invalid(400),
            };
            ParseStatus::Complete(ParsedRequest {
                method,
                path,
                version: req.version.unwrap_or(1),
                headers: req
                    .headers
                    .iter()
                    .filter(|h| !h.name.is_empty())
                    .cloned()
                    .collect(),
                body_offset: body_start,
            })
        }
        Ok(httparse::Status::Partial) => ParseStatus::Partial,
        Err(httparse::Error::TooManyHeaders) => ParseStatus::Invalid(431),
        Err(_) => ParseStatus::Invalid(400),
    }
}

/// Case-insensitive `Host` match against `expected_host`, ignoring any port.
pub fn validate_host(headers: &[httparse::Header<'_>], expected_host: &str) -> bool {
    headers.iter().any(|h| {
        h.name.eq_ignore_ascii_case("host")
            && std::str::from_utf8(h.value)
                .map(|v| {
                    let host_part = v.split(':').next().unwrap_or(v).trim();
                    host_part.eq_ignore_ascii_case(expected_host)
                })
                .unwrap_or(false)
    })
}

/// Determine request body framing, rejecting request-smuggling vectors:
/// conflicting `Content-Length` values, `Content-Length` with
/// `Transfer-Encoding`, and any `Transfer-Encoding` whose final coding is not
/// `chunked`. On rejection the `Err` carries the status code (always 400).
pub fn request_body_framing(headers: &[httparse::Header<'_>]) -> Result<BodyFraming, u16> {
    let mut content_length: Option<u64> = None;
    let mut te_present = false;
    let mut te_chunked = false;

    for h in headers {
        if h.name.eq_ignore_ascii_case("content-length") {
            let n: u64 = std::str::from_utf8(h.value)
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .ok_or(400u16)?;
            match content_length {
                Some(prev) if prev != n => return Err(400),
                _ => content_length = Some(n),
            }
        } else if h.name.eq_ignore_ascii_case("transfer-encoding") {
            te_present = true;
            let v = std::str::from_utf8(h.value)
                .unwrap_or("")
                .to_ascii_lowercase();
            te_chunked = v.split(',').map(str::trim).next_back() == Some("chunked");
        }
    }

    // CL + TE is a classic smuggling desync; reject (RFC 9112 §6.1).
    if te_present && content_length.is_some() {
        return Err(400);
    }
    if te_present {
        return if te_chunked {
            Ok(BodyFraming::Chunked)
        } else {
            Err(400)
        };
    }
    match content_length {
        Some(n) => Ok(BodyFraming::ContentLength(n)),
        None => Ok(BodyFraming::None),
    }
}

/// Whether the client wants keep-alive (HTTP/1.1 default unless
/// `Connection: close`; reverse for HTTP/1.0).
pub fn request_wants_keep_alive(headers: &[httparse::Header<'_>], version: u8) -> bool {
    let conn = header_value_lower(headers, "connection");
    match conn {
        Some(v) if v.contains("close") => false,
        Some(v) if v.contains("keep-alive") => true,
        _ => version >= 1,
    }
}

/// Rebuild the request for the upstream: request line, headers minus hop-by-hop
/// ones (RFC 9110 §7.6.1), `Connection: keep-alive`, then `body` verbatim.
/// `Transfer-Encoding` is preserved so a chunked body stays correctly framed.
pub fn build_upstream_request(req: &ParsedRequest<'_>, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 256);

    out.extend_from_slice(req.method.as_bytes());
    out.push(b' ');
    out.extend_from_slice(req.path.as_bytes());
    out.extend_from_slice(b" HTTP/1.1\r\n");

    for h in &req.headers {
        if should_forward_header(h.name) {
            out.extend_from_slice(h.name.as_bytes());
            out.extend_from_slice(b": ");
            out.extend_from_slice(h.value);
            out.extend_from_slice(b"\r\n");
        }
    }

    out.extend_from_slice(b"Connection: keep-alive\r\n\r\n");
    out.extend_from_slice(body);
    out
}

/// Offset just past the `\r\n\r\n` that ends the response headers, or `None`.
pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// Parse `Content-Length` from a response header section.
pub fn parse_response_content_length(header_section: &[u8]) -> Option<u64> {
    std::str::from_utf8(header_section)
        .ok()?
        .lines()
        .find_map(|line| {
            let lower = line.to_ascii_lowercase();
            lower
                .strip_prefix("content-length:")
                .map(|v| v.trim().to_string())
        })?
        .parse()
        .ok()
}

/// Whether the response permits connection reuse. HTTP/1.1 defaults to
/// keep-alive; only an explicit `Connection: close` disables it.
pub fn response_is_keep_alive(header_section: &[u8]) -> bool {
    match std::str::from_utf8(header_section) {
        Ok(s) => !s.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("connection:") && l.contains("close")
        }),
        Err(_) => false,
    }
}

/// Whether the response uses `Transfer-Encoding: chunked`.
pub fn response_is_chunked(header_section: &[u8]) -> bool {
    match std::str::from_utf8(header_section) {
        Ok(s) => s.lines().any(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("transfer-encoding:") && l.contains("chunked")
        }),
        Err(_) => false,
    }
}

/// Scan for the chunked terminal marker `"0\r\n\r\n"`. This is a byte scan, not
/// a full decoder — a chunk payload containing those bytes is a false positive
/// and simply prevents pooling that connection.
pub fn has_terminal_chunk(buf: &[u8]) -> bool {
    buf.windows(5).any(|w| w == b"0\r\n\r\n")
}

fn header_value_lower(headers: &[httparse::Header<'_>], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case(name))
        .and_then(|h| std::str::from_utf8(h.value).ok())
        .map(|v| v.to_ascii_lowercase())
}

/// Hop-by-hop headers that must not be forwarded. `Transfer-Encoding` is
/// intentionally absent: the body is streamed verbatim, so its framing header
/// travels with it.
fn should_forward_header(name: &str) -> bool {
    const HOP_BY_HOP: &[&str] = &[
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailers",
        "upgrade",
    ];
    !HOP_BY_HOP.iter().any(|h| h.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framing_of(req: &'static str) -> Result<BodyFraming, u16> {
        let mut store = [httparse::EMPTY_HEADER; 32];
        match parse_request(req.as_bytes(), &mut store) {
            ParseStatus::Complete(p) => request_body_framing(&p.headers),
            _ => panic!("request did not parse"),
        }
    }

    #[test]
    fn content_length_framing() {
        assert_eq!(
            framing_of("POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello"),
            Ok(BodyFraming::ContentLength(5))
        );
    }

    #[test]
    fn chunked_framing() {
        assert_eq!(
            framing_of("POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n"),
            Ok(BodyFraming::Chunked)
        );
    }

    #[test]
    fn no_body_framing() {
        assert_eq!(
            framing_of("GET / HTTP/1.1\r\nHost: x\r\n\r\n"),
            Ok(BodyFraming::None)
        );
    }

    #[test]
    fn rejects_cl_plus_te() {
        assert_eq!(
            framing_of(
                "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\n"
            ),
            Err(400)
        );
    }

    #[test]
    fn rejects_conflicting_content_length() {
        assert_eq!(
            framing_of(
                "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\n"
            ),
            Err(400)
        );
    }

    #[test]
    fn rejects_non_chunked_transfer_encoding() {
        assert_eq!(
            framing_of("POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: gzip\r\n\r\n"),
            Err(400)
        );
    }

    #[test]
    fn keep_alive_defaults_by_version() {
        let mut store = [httparse::EMPTY_HEADER; 16];
        if let ParseStatus::Complete(p) =
            parse_request(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n", &mut store)
        {
            assert!(request_wants_keep_alive(&p.headers, p.version));
        } else {
            panic!("parse failed");
        }

        let mut store = [httparse::EMPTY_HEADER; 16];
        if let ParseStatus::Complete(p) =
            parse_request(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n", &mut store)
        {
            assert!(!request_wants_keep_alive(&p.headers, p.version));
        } else {
            panic!("parse failed");
        }
    }

    #[test]
    fn connection_close_overrides_keep_alive() {
        let mut store = [httparse::EMPTY_HEADER; 16];
        let req = b"GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
        if let ParseStatus::Complete(p) = parse_request(req, &mut store) {
            assert!(!request_wants_keep_alive(&p.headers, p.version));
        } else {
            panic!("parse failed");
        }
    }

    #[test]
    fn too_many_headers_is_431() {
        let mut req = String::from("GET / HTTP/1.1\r\n");
        for i in 0..100 {
            req.push_str(&format!("X-H{i}: v\r\n"));
        }
        req.push_str("\r\n");
        let mut store = [httparse::EMPTY_HEADER; 64];
        assert!(matches!(
            parse_request(req.as_bytes(), &mut store),
            ParseStatus::Invalid(431)
        ));
    }

    #[test]
    fn transfer_encoding_is_forwarded() {
        let mut store = [httparse::EMPTY_HEADER; 16];
        let req = b"POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n";
        if let ParseStatus::Complete(p) = parse_request(req, &mut store) {
            let out = build_upstream_request(&p, b"");
            let text = String::from_utf8(out).unwrap();
            assert!(
                text.to_ascii_lowercase()
                    .contains("transfer-encoding: chunked")
            );
        } else {
            panic!("parse failed");
        }
    }

    #[test]
    fn partial_request_is_partial() {
        let mut store = [httparse::EMPTY_HEADER; 16];
        assert!(matches!(
            parse_request(b"GET / HTTP/1.1\r\nHost: x", &mut store),
            ParseStatus::Partial
        ));
    }
}
