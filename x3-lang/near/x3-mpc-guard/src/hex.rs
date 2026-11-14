//! Strict lowercase hex. One spelling per byte string, so a canonical record has one encoding.

const DIGITS: &[u8; 16] = b"0123456789abcdef";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

fn nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    }
}

/// Decode lowercase hex without a prefix. Uppercase, odd length or any other character is refused.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let raw = text.as_bytes();
    if !raw.len().is_multiple_of(2) {
        return None;
    }
    raw.chunks(2)
        .map(|pair| Some((nibble(pair[0])? << 4) | nibble(pair[1])?))
        .collect()
}

pub fn decode_array<const N: usize>(text: &str) -> Option<[u8; N]> {
    decode(text)?.try_into().ok()
}

/// `0x` + 40 lowercase hex characters, the only EVM address spelling this contract accepts.
pub fn parse_evm_address(text: &str) -> Option<[u8; 20]> {
    decode_array::<20>(text.strip_prefix("0x")?)
}

pub fn format_evm_address(address: &[u8; 20]) -> String {
    format!("0x{}", encode(address))
}

pub fn is_hex64(text: &str) -> bool {
    text.len() == 64 && decode(text).is_some()
}
