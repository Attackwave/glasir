//! Strict PEM reader for the TLS material Glasir accepts.
//!
//! It intentionally supports only certificate chains and unencrypted PKCS#8,
//! PKCS#1 or SEC1 private keys. Encrypted PEM belongs in an operator-managed
//! secret store, not in a server that would otherwise need password handling.

use rustls::pki_types::{
    CertificateDer, PrivateKeyDer, PrivatePkcs1KeyDer, PrivatePkcs8KeyDer, PrivateSec1KeyDer,
};
use std::io::BufRead;

pub fn certs(reader: &mut impl BufRead) -> std::io::Result<Vec<CertificateDer<'static>>> {
    let blocks = blocks(reader)?;
    let certs: Vec<_> = blocks
        .into_iter()
        .filter_map(|(label, der)| (label == "CERTIFICATE").then_some(CertificateDer::from(der)))
        .collect();
    if certs.is_empty() {
        return Err(std::io::Error::other("no CERTIFICATE PEM block found"));
    }
    Ok(certs)
}

pub fn private_key(reader: &mut impl BufRead) -> std::io::Result<PrivateKeyDer<'static>> {
    for (label, der) in blocks(reader)? {
        let key = match label.as_str() {
            "PRIVATE KEY" => PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(der)),
            "RSA PRIVATE KEY" => PrivateKeyDer::Pkcs1(PrivatePkcs1KeyDer::from(der)),
            "EC PRIVATE KEY" => PrivateKeyDer::Sec1(PrivateSec1KeyDer::from(der)),
            _ => continue,
        };
        return Ok(key);
    }
    Err(std::io::Error::other(
        "no unencrypted private-key PEM block found",
    ))
}

fn blocks(reader: &mut impl BufRead) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut text = String::new();
    reader.read_to_string(&mut text)?;
    let mut out = Vec::new();
    let mut active: Option<(String, String)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(label) = line
            .strip_prefix("-----BEGIN ")
            .and_then(|rest| rest.strip_suffix("-----"))
        {
            if active.is_some() || label.is_empty() {
                return Err(std::io::Error::other("malformed PEM begin marker"));
            }
            active = Some((label.to_owned(), String::new()));
            continue;
        }
        if let Some(label) = line
            .strip_prefix("-----END ")
            .and_then(|rest| rest.strip_suffix("-----"))
        {
            let Some((begin, encoded)) = active.take() else {
                return Err(std::io::Error::other("PEM end marker without begin marker"));
            };
            if begin != label {
                return Err(std::io::Error::other("mismatched PEM markers"));
            }
            out.push((begin, base64(&encoded)?));
            continue;
        }
        if let Some((_, encoded)) = &mut active {
            if line.contains(':') || line.is_empty() {
                return Err(std::io::Error::other(
                    "PEM headers and blank body lines are unsupported",
                ));
            }
            encoded.push_str(line);
        } else if !line.is_empty() {
            return Err(std::io::Error::other("data outside PEM block"));
        }
    }
    if active.is_some() {
        return Err(std::io::Error::other("unterminated PEM block"));
    }
    Ok(out)
}

fn base64(input: &str) -> std::io::Result<Vec<u8>> {
    if input.is_empty() || !input.len().is_multiple_of(4) {
        return Err(std::io::Error::other("invalid PEM base64 length"));
    }
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let blocks = input.len() / 4;
    let (chunks, []) = input.as_bytes().as_chunks::<4>() else {
        unreachable!("base64 length was checked above");
    };
    for (block, chunk) in chunks.iter().enumerate() {
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        if pad > 2 || (pad > 0 && chunk[..4 - pad].contains(&b'=')) {
            return Err(std::io::Error::other("invalid PEM base64 padding"));
        }
        let mut value = 0u32;
        for (i, &byte) in chunk.iter().enumerate() {
            let six = if byte == b'=' {
                if i < 4 - pad {
                    return Err(std::io::Error::other("invalid PEM base64 padding"));
                }
                0
            } else {
                base64_digit(byte)
                    .ok_or_else(|| std::io::Error::other("invalid PEM base64 character"))?
            };
            value = (value << 6) | u32::from(six);
        }
        out.push((value >> 16) as u8);
        if pad < 2 {
            out.push((value >> 8) as u8);
        }
        if pad == 0 {
            out.push(value as u8);
        }
        if pad > 0 && block + 1 != blocks {
            return Err(std::io::Error::other(
                "PEM padding before final base64 block",
            ));
        }
    }
    Ok(out)
}

fn base64_digit(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}
