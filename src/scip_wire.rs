//! The tiny protobuf subset SCIP needs.
//!
//! Glasir consumes three messages and six fields from SCIP. Pulling a code
//! generator, a vendored compiler and a general protobuf runtime into every
//! build for that is disproportionate. This decoder accepts exactly those
//! wire forms, bounds every length before slicing, and skips future SCIP fields
//! by their protobuf wire type.

#[derive(Default)]
pub struct Index {
    pub documents: Vec<Document>,
}

#[derive(Default)]
pub struct Document {
    pub relative_path: String,
    pub occurrences: Vec<Occurrence>,
}

#[derive(Default)]
pub struct Occurrence {
    pub range: Vec<i32>,
    pub symbol: String,
    pub symbol_roles: i32,
}

pub fn decode_index(bytes: &[u8]) -> Result<Index, String> {
    let mut index = Index::default();
    fields(bytes, |number, wire, input, pos| {
        if number == 2 && wire == 2 {
            index
                .documents
                .push(decode_document(bytes_field(input, pos)?)?);
        } else {
            skip(wire, input, pos)?;
        }
        Ok(())
    })?;
    Ok(index)
}

fn decode_document(bytes: &[u8]) -> Result<Document, String> {
    let mut document = Document::default();
    fields(bytes, |number, wire, input, pos| {
        match (number, wire) {
            (1, 2) => document.relative_path = text(bytes_field(input, pos)?)?,
            (2, 2) => document
                .occurrences
                .push(decode_occurrence(bytes_field(input, pos)?)?),
            _ => skip(wire, input, pos)?,
        }
        Ok(())
    })?;
    Ok(document)
}

fn decode_occurrence(bytes: &[u8]) -> Result<Occurrence, String> {
    let mut occurrence = Occurrence::default();
    fields(bytes, |number, wire, input, pos| {
        match (number, wire) {
            // Repeated numeric fields are normally packed, but accepting the
            // unpacked form costs one branch and keeps old producers usable.
            (1, 2) => {
                let packed = bytes_field(input, pos)?;
                let mut p = 0;
                while p < packed.len() {
                    occurrence.range.push(varint(packed, &mut p)? as i32);
                }
            }
            (1, 0) => occurrence.range.push(varint(input, pos)? as i32),
            (2, 2) => occurrence.symbol = text(bytes_field(input, pos)?)?,
            (3, 0) => occurrence.symbol_roles = varint(input, pos)? as i32,
            _ => skip(wire, input, pos)?,
        }
        Ok(())
    })?;
    Ok(occurrence)
}

fn fields(
    bytes: &[u8],
    mut field: impl FnMut(u32, u8, &[u8], &mut usize) -> Result<(), String>,
) -> Result<(), String> {
    let mut pos = 0;
    while pos < bytes.len() {
        let tag = varint(bytes, &mut pos)?;
        let number = (tag >> 3) as u32;
        let wire = (tag & 7) as u8;
        if number == 0 {
            return Err("protobuf field number 0".into());
        }
        field(number, wire, bytes, &mut pos)?;
    }
    Ok(())
}

fn varint(bytes: &[u8], pos: &mut usize) -> Result<u64, String> {
    let mut out = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(*pos).ok_or("truncated protobuf varint")?;
        *pos += 1;
        if shift == 63 && byte > 1 {
            return Err("protobuf varint overflows u64".into());
        }
        out |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(out);
        }
    }
    Err("protobuf varint overflow".into())
}

fn bytes_field<'a>(bytes: &'a [u8], pos: &mut usize) -> Result<&'a [u8], String> {
    let len = usize::try_from(varint(bytes, pos)?).map_err(|_| "protobuf length overflow")?;
    let end = pos.checked_add(len).ok_or("protobuf length overflow")?;
    let value = bytes
        .get(*pos..end)
        .ok_or("truncated protobuf bytes field")?;
    *pos = end;
    Ok(value)
}

fn text(bytes: &[u8]) -> Result<String, String> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| "protobuf string is not UTF-8".into())
}

fn skip(wire: u8, bytes: &[u8], pos: &mut usize) -> Result<(), String> {
    match wire {
        0 => {
            let _ = varint(bytes, pos)?;
        }
        1 => {
            *pos = pos
                .checked_add(8)
                .filter(|end| *end <= bytes.len())
                .ok_or("truncated fixed64")?
        }
        2 => {
            let _ = bytes_field(bytes, pos)?;
        }
        5 => {
            *pos = pos
                .checked_add(4)
                .filter(|end| *end <= bytes.len())
                .ok_or("truncated fixed32")?
        }
        3 | 4 => return Err("protobuf groups are unsupported".into()),
        _ => return Err("unknown protobuf wire type".into()),
    }
    Ok(())
}

pub type TestOccurrence = (&'static str, bool, i32);
pub type TestDocument<'a> = (&'a str, &'a [TestOccurrence]);

pub fn encode_index_for_test(docs: &[TestDocument<'_>]) -> Vec<u8> {
    let mut index = Vec::new();
    for (path, occurrences) in docs {
        let mut document = Vec::new();
        put_bytes(1, path.as_bytes(), &mut document);
        for (symbol, definition, line) in *occurrences {
            let mut occurrence = Vec::new();
            let mut range = Vec::new();
            for value in [*line, 0, *line, 10] {
                put_varint(value as u64, &mut range);
            }
            put_bytes(1, &range, &mut occurrence);
            put_bytes(2, symbol.as_bytes(), &mut occurrence);
            if *definition {
                put_key(3, 0, &mut occurrence);
                put_varint(1, &mut occurrence);
            }
            put_bytes(2, &occurrence, &mut document);
        }
        put_bytes(2, &document, &mut index);
    }
    index
}

fn put_bytes(field: u32, value: &[u8], out: &mut Vec<u8>) {
    put_key(field, 2, out);
    put_varint(value.len() as u64, out);
    out.extend_from_slice(value);
}

fn put_key(field: u32, wire: u8, out: &mut Vec<u8>) {
    put_varint((u64::from(field) << 3) | u64::from(wire), out);
}

fn put_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}
