/*!
The uri of the protocol as a path, and back out of a request.
*/

use std::path::PathBuf;

use serde_json::Value;

pub(super) fn uri_of(params: &Value) -> String {
    params["textDocument"]["uri"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/*
A `file://` uri as a path.

Only the plain form, with percent decoding for the characters that an editor
escapes. A full uri parser would be a dependency for the one scheme that
matters. A path that fails here only means that the server does not find the
project config; no other function breaks.
*/
pub(super) fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;

    // On Windows the path arrives as /C:/thing; remove the leading slash.
    let rest = match rest.get(..3) {
        Some(p) if p.starts_with('/') && p.as_bytes()[2] == b':' => &rest[1..],

        _ => rest,
    };

    let mut out = String::with_capacity(rest.len());
    let mut chars = rest.chars();

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);

            continue;
        }

        let hex: String = chars.by_ref().take(2).collect();

        match u8::from_str_radix(&hex, 16) {
            Ok(byte) => out.push(byte as char),

            Err(_) => {
                out.push('%');
                out.push_str(&hex);
            }
        }
    }

    Some(PathBuf::from(out))
}
