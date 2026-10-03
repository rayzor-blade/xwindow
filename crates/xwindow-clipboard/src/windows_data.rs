//! The bytes of the Windows clipboard formats that need converting: CF_HTML's
//! header, CF_HDROP's paths as `file://` URIs, and DIBs as PNG. Nothing here
//! calls Windows, so it builds and tests on every platform.

const START_MARK: &[u8] = b"<!--StartFragment-->";
const END_MARK: &[u8] = b"<!--EndFragment-->";

/// `html` as CF_HTML: a header of byte offsets, then the document with the
/// fragment between its markers. A document's body is the fragment; anything
/// else is wrapped in a bare document. Ends with a NUL for readers that
/// treat it as a C string.
pub fn cf_html(html: &[u8]) -> Vec<u8> {
    let (before, fragment, after): (&[u8], &[u8], &[u8]) = match body(html) {
        Some((start, end)) => (&html[..start], &html[start..end], &html[end..]),
        None => (b"<html>\r\n<body>\r\n", html, b"\r\n</body>\r\n</html>"),
    };
    let header = |start_html: usize, end_html: usize, start: usize, end: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\n\
             StartFragment:{start:010}\r\nEndFragment:{end:010}\r\n"
        )
    };
    // The offsets are fixed-width, so the header's length does not depend on
    // them.
    let start_html = header(0, 0, 0, 0).len();
    let start = start_html + before.len() + START_MARK.len();
    let end = start + fragment.len();
    let end_html = end + END_MARK.len() + after.len();
    let mut out = header(start_html, end_html, start, end).into_bytes();
    for part in [before, START_MARK, fragment, END_MARK, after, b"\0"] {
        out.extend_from_slice(part);
    }
    out
}

/// Where the content of `html`'s body starts and ends, if it has one.
fn body(html: &[u8]) -> Option<(usize, usize)> {
    let lower = html.to_ascii_lowercase();
    let open = find(&lower, b"<body", 0)
        .filter(|&i| matches!(lower.get(i + 5), Some(b'>' | b' ' | b'\t' | b'\r' | b'\n')))?;
    let start = open + lower[open..].iter().position(|&b| b == b'>')? + 1;
    let end = (start..=lower.len())
        .rev()
        .find(|&i| lower[i..].starts_with(b"</body"))?;
    Some((start, end))
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| i + from)
}

/// The HTML in CF_HTML `data`: the document between StartHTML and EndHTML,
/// less the fragment markers, so that what [`cf_html`] wrapped reads back
/// unchanged. When the document is only a bare wrapper around the fragment,
/// or there is no document, the fragment alone.
pub fn html_from_cf(data: &[u8]) -> Option<Vec<u8>> {
    let offsets = header(data);
    let range = |start: &str, end: &str| {
        let (start, end) = (offsets(start)?, offsets(end)?);
        (start <= end && end <= data.len()).then_some(start..end)
    };
    let document = range("StartHTML", "EndHTML");
    let fragment = range("StartFragment", "EndFragment");
    match (document, fragment) {
        (Some(d), Some(f)) if d.start <= f.start && f.end <= d.end => {
            let before = &data[d.start..f.start];
            let before = before.strip_suffix(START_MARK).unwrap_or(before);
            let after = &data[f.end..d.end];
            let after = after.strip_prefix(END_MARK).unwrap_or(after);
            let fragment = &data[f];
            Some(if bare(before) && bare(after) {
                fragment.to_vec()
            } else {
                [before, fragment, after].concat()
            })
        }
        (Some(d), _) => Some(data[d].to_vec()),
        (None, Some(f)) => Some(data[f].to_vec()),
        (None, None) => None,
    }
}

/// The header's offsets by key: its `Key:value` lines, up to the first that
/// is not one. A negative offset, which CF_HTML allows for an absent
/// document, is none.
fn header(data: &[u8]) -> impl Fn(&str) -> Option<usize> + '_ {
    move |key| {
        data.split(|&b| b == b'\n')
            .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
            .map_while(|line| {
                let colon = line.iter().position(|&b| b == b':')?;
                let name = &line[..colon];
                (!name.is_empty() && name.iter().all(u8::is_ascii_alphanumeric))
                    .then_some((name, &line[colon + 1..]))
            })
            .find(|(name, _)| *name == key.as_bytes())
            .and_then(|(_, value)| std::str::from_utf8(value).ok()?.trim().parse().ok())
    }
}

/// Whether `html` is nothing but whitespace and the bare tags that wrap a
/// fragment.
fn bare(mut html: &[u8]) -> bool {
    loop {
        html = html.trim_ascii_start();
        if html.is_empty() {
            return true;
        }
        let Some(end) = html.iter().position(|&b| b == b'>') else {
            return false;
        };
        let tag = html[..=end].to_ascii_lowercase();
        if !matches!(
            &tag[..],
            b"<html>" | b"</html>" | b"<body>" | b"</body>" | b"<!doctype html>"
        ) {
            return false;
        }
        html = &html[end + 1..];
    }
}

/// A Windows path as a `file://` URI: a drive path as `file:///C:/...`, a UNC
/// path as `file://server/share/...`, every byte but the unreserved ones and
/// separators percent-encoded.
pub fn uri_from_path(path: &str) -> String {
    let path = path
        .strip_prefix(r"\\?\UNC\")
        .map(|unc| format!(r"\\{unc}"))
        .or_else(|| path.strip_prefix(r"\\?\").map(str::to_owned))
        .unwrap_or_else(|| path.to_owned());
    let (mut uri, rest) = match path.strip_prefix(r"\\") {
        Some(unc) => ("file://".to_owned(), unc),
        None => ("file:///".to_owned(), path.as_str()),
    };
    for byte in rest.bytes() {
        match byte {
            b'\\' => uri.push('/'),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                uri.push(byte as char)
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The Windows path a `file://` URI names, or none for another scheme or a
/// path that is not UTF-8. A host other than `localhost` makes a UNC path.
pub fn path_from_uri(uri: &str) -> Option<String> {
    let scheme = uri.get(..5)?;
    if !scheme.eq_ignore_ascii_case("file:") {
        return None;
    }
    let rest = &uri[5..];
    let rest = &rest[..rest.find(['?', '#']).unwrap_or(rest.len())];
    let (host, path) = match rest.strip_prefix("//") {
        Some(rest) => rest.split_at(rest.find('/').unwrap_or(rest.len())),
        None if rest.starts_with('/') => ("", rest),
        None => return None,
    };
    let path = String::from_utf8(decode(path)).ok()?;
    let local = host.is_empty() || host.eq_ignore_ascii_case("localhost");
    let path = match path.as_bytes() {
        // `/C:/x` and the older `/C|/x` are drive paths.
        [b'/', drive, b':' | b'|', ..] if local && drive.is_ascii_alphabetic() => {
            format!("{}:{}", *drive as char, &path[3..])
        }
        _ if local => path,
        _ => format!(r"\\{}{path}", String::from_utf8(decode(host)).ok()?),
    };
    Some(path.replace('/', r"\"))
}

/// `%XX` escapes as their bytes; a malformed escape stays as it is.
fn decode(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    out
}

/// Paths as a `text/uri-list`: one URI per line, each ended by CRLF.
pub fn uri_list<S: AsRef<str>>(paths: &[S]) -> Vec<u8> {
    paths
        .iter()
        .map(|path| uri_from_path(path.as_ref()) + "\r\n")
        .collect::<String>()
        .into_bytes()
}

/// The paths in a `text/uri-list`, skipping comments and URIs that are not
/// files.
pub fn paths(list: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(list)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(path_from_uri)
        .collect()
}

/// The length of the PNG that starts `data`, through its IEND chunk: clipboard
/// memory may run past the image. All of `data` when it does not parse.
pub fn png_len(data: &[u8]) -> usize {
    if !data.starts_with(PNG_SIGNATURE) {
        return data.len();
    }
    let mut at = PNG_SIGNATURE.len();
    while let Some(header) = data.get(at..at + 8) {
        let len = u32::from_be_bytes(header[..4].try_into().unwrap()) as usize;
        let Some(end) = (at + 12).checked_add(len).filter(|&end| end <= data.len()) else {
            break;
        };
        if &header[4..] == b"IEND" {
            return end;
        }
        at = end;
    }
    data.len()
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;

/// A packed DIB's layout, read from its header.
struct Dib {
    width: usize,
    height: usize,
    top_down: bool,
    bits: u16,
    /// Red, green, blue and alpha, for 32 bits a pixel.
    masks: [u32; 4],
    /// Where the pixels start: after the header, the masks that follow a
    /// plain BITMAPINFOHEADER, and any color table.
    offset: usize,
    stride: usize,
}

impl Dib {
    /// The layout when [`dib_to_png`] can convert it: uncompressed 24 bits a
    /// pixel, or 32 with or without bitfields. `data` needs only the header
    /// and the masks after it.
    fn parse(data: &[u8]) -> Option<Self> {
        let u32_at = |at: usize| Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?));
        let size = u32_at(0)? as usize;
        let width = u32_at(4)? as i32;
        let height = u32_at(8)? as i32;
        let bits = u16::from_le_bytes(data.get(14..16)?.try_into().ok()?);
        let compression = u32_at(16)?;
        let colors = u32_at(32)? as usize;
        let masks = match (bits, compression) {
            (24, BI_RGB) => [0; 4],
            (32, BI_RGB) => [0xff_0000, 0xff00, 0xff, 0xff00_0000],
            // A plain header is followed by three masks; a V4 or V5 header
            // holds them at the same place, and alpha's after them.
            (32, BI_BITFIELDS) => [
                u32_at(40)?,
                u32_at(44)?,
                u32_at(48)?,
                if size >= 56 { u32_at(52)? } else { 0 },
            ],
            _ => return None,
        };
        if size < 40 || width <= 0 || height == 0 || height == i32::MIN {
            return None;
        }
        let width = width as usize;
        let masks_after = if compression == BI_BITFIELDS && size == 40 {
            12
        } else {
            0
        };
        Some(Self {
            width,
            height: height.unsigned_abs() as usize,
            top_down: height < 0,
            bits,
            masks,
            offset: size
                .checked_add(masks_after)?
                .checked_add(colors.checked_mul(4)?)?,
            stride: width.checked_mul(bits as usize)?.checked_add(31)? / 32 * 4,
        })
    }
}

/// Whether [`dib_to_png`] converts the DIB this header starts.
pub fn dib_supported(header: &[u8]) -> bool {
    Dib::parse(header).is_some()
}

/// A packed DIB (CF_DIB or CF_DIBV5) as an uncompressed RGBA PNG. A 32-bit
/// DIB whose alpha is zero throughout has none, and is opaque.
pub fn dib_to_png(data: &[u8]) -> Option<Vec<u8>> {
    let dib = Dib::parse(data)?;
    let end = dib
        .stride
        .checked_mul(dib.height)?
        .checked_add(dib.offset)?;
    let pixels = data.get(dib.offset..end)?;
    let mut rows = Vec::with_capacity(dib.height * (1 + dib.width * 4));
    for y in 0..dib.height {
        let row = if dib.top_down { y } else { dib.height - 1 - y };
        let row = &pixels[row * dib.stride..][..dib.stride];
        rows.push(0); // no filter
        for x in 0..dib.width {
            if dib.bits == 24 {
                let [b, g, r] = row[x * 3..x * 3 + 3].try_into().unwrap();
                rows.extend([r, g, b, 0xff]);
            } else {
                let pixel = u32::from_le_bytes(row[x * 4..x * 4 + 4].try_into().unwrap());
                rows.extend(dib.masks.map(|mask| channel(pixel, mask)));
            }
        }
    }
    let (width, height, line) = (dib.width, dib.height, 1 + dib.width * 4);
    let alphas = || (0..height).flat_map(move |y| (0..width).map(move |x| y * line + 4 + x * 4));
    if alphas().all(|i| rows[i] == 0) {
        for i in alphas() {
            rows[i] = 0xff;
        }
    }
    Some(png(dib.width as u32, dib.height as u32, &rows))
}

/// The channel `mask` selects from `pixel`, scaled to eight bits.
fn channel(pixel: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let max = mask >> mask.trailing_zeros();
    let value = (pixel & mask) >> mask.trailing_zeros();
    (value as u64 * 255 / max as u64) as u8
}

/// An RGBA PNG of `rows`, each a filter byte then the pixels, stored without
/// compression.
fn png(width: u32, height: u32, rows: &[u8]) -> Vec<u8> {
    let mut out = PNG_SIGNATURE.to_vec();
    let mut ihdr = [0; 13];
    ihdr[..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8..10].copy_from_slice(&[8, 6]); // eight bits a channel, RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib_stored(rows));
    chunk(&mut out, b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A zlib stream of deflate's stored blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut rest = data;
    loop {
        let len = rest.len().min(0xffff);
        let last = len == rest.len();
        out.push(last as u8);
        out.extend_from_slice(&(len as u16).to_le_bytes());
        out.extend_from_slice(&(!(len as u16)).to_le_bytes());
        out.extend_from_slice(&rest[..len]);
        rest = &rest[len..];
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // The longest run whose sums cannot overflow before the modulo.
    for run in data.chunks(5552) {
        for &byte in run {
            a += byte as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

fn crc32(data: &[u8]) -> u32 {
    const TABLE: [u32; 256] = {
        let mut table = [0; 256];
        let mut n = 0;
        while n < 256 {
            let mut c = n as u32;
            let mut k = 0;
            while k < 8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
                k += 1;
            }
            table[n] = c;
            n += 1;
        }
        table
    };
    !data.iter().fold(!0, |c, &b| {
        TABLE[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_html_offsets() {
        let data = cf_html(b"<b>hi</b>");
        let text = std::str::from_utf8(&data).unwrap();
        let expected = "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000186\r\n\
                        StartFragment:0000000141\r\nEndFragment:0000000150\r\n\
                        <html>\r\n<body>\r\n<!--StartFragment--><b>hi</b><!--EndFragment-->\
                        \r\n</body>\r\n</html>\0";
        assert_eq!(text, expected);
        assert_eq!(&data[105..113], b"<html>\r\n");
        assert_eq!(&data[141..150], b"<b>hi</b>");
        assert_eq!(&data[150..168], END_MARK);
        assert_eq!(data.len(), 187);
    }

    #[test]
    fn cf_html_round_trips() {
        for html in [
            &b"<b>xwindow</b> typed"[..],
            "caf\u{e9} \u{2713}".as_bytes(),
            b"",
            b"<!DOCTYPE html><html><head><style>b{}</style></head><BODY class=x>\n<b>x</b>\n</BODY></html>",
        ] {
            assert_eq!(html_from_cf(&cf_html(html)).unwrap(), html);
        }
    }

    #[test]
    fn cf_html_body_is_the_fragment() {
        let data = cf_html(b"<html><body><p>x</p></body></html>");
        let header = header(&data);
        let (start, end) = (
            header("StartFragment").unwrap(),
            header("EndFragment").unwrap(),
        );
        assert_eq!(&data[start..end], b"<p>x</p>");
        assert_eq!(&data[start - START_MARK.len()..start], START_MARK);
    }

    #[test]
    fn cf_html_reads_other_writers() {
        // No document, as CF_HTML allows: the fragment.
        let fragment_only = b"Version:1.0\nStartHTML:-1\nEndHTML:-1\nStartFragment:0000000084\nEndFragment:0000000092\n<i>x</i>";
        assert_eq!(html_from_cf(fragment_only).unwrap(), b"<i>x</i>");
        // A document with context around the fragment: all of it, less the
        // markers.
        let body = "<html><head><meta charset=utf-8></head><body><!--StartFragment--><u>y</u><!--EndFragment--></body></html>";
        let head = "Version:0.9\r\nStartHTML:0000000105\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n";
        assert_eq!(head.len(), 105);
        let start = 105 + body.find("<u>").unwrap();
        let end = start + 8;
        let head = head
            .replace(
                "EndHTML:0000000000",
                &format!("EndHTML:{:010}", 105 + body.len()),
            )
            .replace(
                "StartFragment:0000000000",
                &format!("StartFragment:{start:010}"),
            )
            .replace("EndFragment:0000000000", &format!("EndFragment:{end:010}"));
        let data = format!("{head}{body}\0\0");
        assert_eq!(
            html_from_cf(data.as_bytes()).unwrap(),
            b"<html><head><meta charset=utf-8></head><body><u>y</u></body></html>"
        );
        assert_eq!(html_from_cf(b"<b>no header</b>"), None);
        assert_eq!(html_from_cf(b"StartHTML:10\r\nEndHTML:999\r\n"), None);
    }

    #[test]
    fn paths_as_uris() {
        assert_eq!(
            uri_from_path("C:\\Users\\a b\\caf\u{e9}#1.txt"),
            "file:///C:/Users/a%20b/caf%C3%A9%231.txt"
        );
        assert_eq!(
            uri_from_path(r"\\server\share\x%y"),
            "file://server/share/x%25y"
        );
        assert_eq!(uri_from_path(r"\\?\C:\long"), "file:///C:/long");
        assert_eq!(
            uri_from_path(r"\\?\UNC\server\share"),
            "file://server/share"
        );
    }

    #[test]
    fn uris_as_paths() {
        let path = |uri| path_from_uri(uri);
        assert_eq!(
            path("file:///C:/Users/a%20b/caf%C3%A9%231.txt").unwrap(),
            "C:\\Users\\a b\\caf\u{e9}#1.txt"
        );
        assert_eq!(path("FILE://localhost/c|/x").unwrap(), r"c:\x");
        assert_eq!(path("file:/D:/y").unwrap(), r"D:\y");
        assert_eq!(
            path("file://server/share/x%25y").unwrap(),
            r"\\server\share\x%y"
        );
        assert_eq!(path("file:///C:/q?x#y").unwrap(), r"C:\q");
        assert_eq!(path("file:///tmp/%zz").unwrap(), r"\tmp\%zz");
        assert_eq!(path("file:///C:/%FF"), None);
        assert_eq!(path("https://example.com/x"), None);
        assert_eq!(path("file:x"), None);
    }

    #[test]
    fn uri_lists() {
        let list = uri_list(&[r"C:\a b", r"\\s\t\u"]);
        assert_eq!(list, b"file:///C:/a%20b\r\nfile://s/t/u\r\n");
        assert_eq!(paths(&list), [r"C:\a b", r"\\s\t\u"]);
        assert_eq!(paths(b"# comment\nhttp://x/y\n\nfile:///C:/z\n"), [r"C:\z"]);
    }

    #[test]
    fn checksums() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
        assert_eq!(adler32(&vec![0xff; 100_000]), {
            let (mut a, mut b) = (1u64, 0u64);
            for _ in 0..100_000 {
                a = (a + 0xff) % 65521;
                b = (b + a) % 65521;
            }
            (b << 16 | a) as u32
        });
    }

    #[test]
    fn stored_blocks() {
        let data = vec![7; 70_000];
        let z = zlib_stored(&data);
        assert_eq!(&z[..2], [0x78, 0x01]);
        assert_eq!(&z[2..7], [0, 0xff, 0xff, 0, 0]);
        let second = 7 + 0xffff;
        assert_eq!(&z[second..second + 5], [1, 0x71, 0x11, 0x8e, 0xee]);
        assert_eq!(z.len(), 2 + 5 + 0xffff + 5 + (70_000 - 0xffff) + 4);
    }

    /// A 2x2 DIB, bottom-up: its first row in memory is the image's last.
    fn dib(bits: u16, compression: u32, size: u32, masks: &[u32], pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(size.to_le_bytes());
        out.extend(2i32.to_le_bytes());
        out.extend(2i32.to_le_bytes());
        out.extend(1u16.to_le_bytes());
        out.extend(bits.to_le_bytes());
        out.extend(compression.to_le_bytes());
        out.resize(40, 0);
        for mask in masks {
            out.extend(mask.to_le_bytes());
        }
        out.resize(
            size.max(40) as usize + if size == 40 { masks.len() * 4 } else { 0 },
            0,
        );
        out.extend(pixels);
        out
    }

    /// The RGBA rows of a PNG [`png`] wrote, filter bytes and all.
    fn rows(png: &[u8]) -> (u32, u32, Vec<u8>) {
        assert_eq!(png_len(png), png.len());
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        let idat_len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        assert_eq!(&png[37..41], b"IDAT");
        let z = &png[41..41 + idat_len];
        (width, height, z[7..z.len() - 4].to_vec())
    }

    #[test]
    fn dib_24_bits() {
        // Rows padded to four bytes; the bottom row first.
        let pixels = [
            0, 0, 0xff, 0, 0xff, 0, 0, 0, // red, green
            0xff, 0, 0, 0xff, 0xff, 0xff, 0, 0, // blue, white
        ];
        let png = dib_to_png(&dib(24, BI_RGB, 40, &[], &pixels)).unwrap();
        let (w, h, rows) = rows(&png);
        assert_eq!((w, h), (2, 2));
        assert_eq!(
            rows,
            [
                0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0, 0xff, 0, 0, 0xff, 0, 0xff, 0, 0xff
            ]
        );
    }

    #[test]
    fn dib_32_bits() {
        // Zero alpha throughout: opaque.
        let pixels = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0];
        let (_, _, opaque) = rows(&dib_to_png(&dib(32, BI_RGB, 40, &[], &pixels)).unwrap());
        assert_eq!(
            opaque,
            [
                0, 9, 8, 7, 255, 12, 11, 10, 255, 0, 3, 2, 1, 255, 6, 5, 4, 255
            ]
        );
        // A V5 header's bitfields, with alpha.
        let masks = [0xff, 0xff00, 0xff_0000, 0xff00_0000];
        let pixels = [1, 2, 3, 0x80, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0xff];
        let data = dib(32, BI_BITFIELDS, 124, &masks, &pixels);
        assert!(dib_supported(&data[..124]));
        let (_, _, alpha) = rows(&dib_to_png(&data).unwrap());
        assert_eq!(
            alpha,
            [
                0, 7, 8, 9, 0, 10, 11, 12, 0xff, 0, 1, 2, 3, 0x80, 4, 5, 6, 0
            ]
        );
        // A plain header's three masks follow it.
        let data = dib(32, BI_BITFIELDS, 40, &masks[..3], &pixels);
        let (_, _, plain) = rows(&dib_to_png(&data).unwrap());
        assert_eq!(
            plain,
            [
                0, 7, 8, 9, 255, 10, 11, 12, 255, 0, 1, 2, 3, 255, 4, 5, 6, 255
            ]
        );
    }

    #[test]
    fn dib_refusals() {
        assert!(!dib_supported(&dib(8, BI_RGB, 40, &[], &[])));
        assert!(!dib_supported(&dib(
            16,
            BI_BITFIELDS,
            40,
            &[0xf800, 0x7e0, 0x1f],
            &[]
        )));
        assert!(!dib_supported(&[0; 20]));
        // Too few pixels for the header.
        assert_eq!(dib_to_png(&dib(24, BI_RGB, 40, &[], &[0; 15])), None);
    }

    #[test]
    fn png_length() {
        let png = png(1, 1, &[0, 1, 2, 3, 4]);
        let mut padded = png.clone();
        padded.extend([0; 16]);
        assert_eq!(png_len(&padded), png.len());
        assert_eq!(png_len(b"not a png"), 9);
    }
}
