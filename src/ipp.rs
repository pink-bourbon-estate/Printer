//! Binary IPP (Internet Printing Protocol) codec.
//!
//! Written from the public specifications:
//!   RFC 8010 - IPP/1.1: Encoding and Transport
//!   RFC 8011 - IPP/1.1: Model and Semantics
//!
//! No vendor SDK, no decompiled driver. This is the same open protocol that
//! AirPrint and Mopria use, and that every IPP Everywhere printer implements
//! in its own firmware.

use std::collections::BTreeMap;
use std::fmt;

// --- Delimiter tags (RFC 8010 s3.5.1) ---------------------------------------
pub const TAG_OPERATION_ATTRS: u8 = 0x01;
pub const TAG_JOB_ATTRS: u8 = 0x02;
pub const TAG_END_OF_ATTRS: u8 = 0x03;
pub const TAG_PRINTER_ATTRS: u8 = 0x04;
pub const TAG_UNSUPPORTED_ATTRS: u8 = 0x05;

// --- Value tags (RFC 8010 s3.5.2) -------------------------------------------
pub const TAG_UNKNOWN: u8 = 0x12;
pub const TAG_NO_VALUE: u8 = 0x13;
pub const TAG_INTEGER: u8 = 0x21;
pub const TAG_BOOLEAN: u8 = 0x22;
pub const TAG_ENUM: u8 = 0x23;
pub const TAG_RESOLUTION: u8 = 0x32;
pub const TAG_RANGE_OF_INTEGER: u8 = 0x33;
pub const TAG_TEXT: u8 = 0x41;
pub const TAG_NAME: u8 = 0x42;
pub const TAG_KEYWORD: u8 = 0x44;
pub const TAG_URI: u8 = 0x45;
pub const TAG_CHARSET: u8 = 0x47;
pub const TAG_NATURAL_LANGUAGE: u8 = 0x48;
pub const TAG_MIME_MEDIA_TYPE: u8 = 0x49;

// --- Operations (RFC 8011 s4.2, PWG 5100.x) ---------------------------------
pub const OP_PRINT_JOB: u16 = 0x0002;
pub const OP_VALIDATE_JOB: u16 = 0x0004;
pub const OP_CREATE_JOB: u16 = 0x0005;
pub const OP_SEND_DOCUMENT: u16 = 0x0006;
pub const OP_CANCEL_JOB: u16 = 0x0008;
pub const OP_GET_JOB_ATTRIBUTES: u16 = 0x0009;
pub const OP_GET_JOBS: u16 = 0x000A;
pub const OP_GET_PRINTER_ATTRIBUTES: u16 = 0x000B;

/// A decoded attribute value. A 1setOf comes back as `Multi`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i32),
    Boolean(bool),
    Enum(i32),
    Resolution(i32, i32, u8),
    Range(i32, i32),
    Text(String),
    NoValue,
    Multi(Vec<Value>),
}

impl Value {
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Value::Integer(v) | Value::Enum(v) => Some(*v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Boolean(b) => Some(*b),
            _ => None,
        }
    }
    /// Normalise a scalar-or-1setOf attribute into a flat list of strings.
    pub fn strings(&self) -> Vec<String> {
        match self {
            Value::Multi(items) => items.iter().filter_map(|v| v.as_str()).map(String::from).collect(),
            Value::Text(s) => vec![s.clone()],
            _ => vec![],
        }
    }
    pub fn ints(&self) -> Vec<i32> {
        match self {
            Value::Multi(items) => items.iter().filter_map(|v| v.as_i32()).collect(),
            Value::Integer(v) | Value::Enum(v) => vec![*v],
            _ => vec![],
        }
    }
}

fn enc_name(out: &mut Vec<u8>, name: &str) {
    let raw = name.as_bytes();
    out.extend_from_slice(&(raw.len() as u16).to_be_bytes());
    out.extend_from_slice(raw);
}

/// One attribute to send: its tag, name, and one or more values (a `Vec` of
/// length > 1 becomes a 1setOf with zero-length names on the extra values).
pub struct Attr {
    pub tag: u8,
    pub name: &'static str,
    pub values: Vec<AttrValue>,
}

pub enum AttrValue {
    Int(i32),
    Bool(bool),
    Text(String),
    Resolution(i32, i32, u8),
}

fn enc_value(out: &mut Vec<u8>, tag: u8, v: &AttrValue) {
    match (tag, v) {
        (TAG_INTEGER, AttrValue::Int(i)) | (TAG_ENUM, AttrValue::Int(i)) => {
            out.extend_from_slice(&4u16.to_be_bytes());
            out.extend_from_slice(&i.to_be_bytes());
        }
        (TAG_BOOLEAN, AttrValue::Bool(b)) => {
            out.extend_from_slice(&1u16.to_be_bytes());
            out.push(if *b { 1 } else { 0 });
        }
        (TAG_RESOLUTION, AttrValue::Resolution(x, y, u)) => {
            out.extend_from_slice(&9u16.to_be_bytes());
            out.extend_from_slice(&x.to_be_bytes());
            out.extend_from_slice(&y.to_be_bytes());
            out.push(*u);
        }
        (_, AttrValue::Text(s)) => {
            let raw = s.as_bytes();
            out.extend_from_slice(&(raw.len() as u16).to_be_bytes());
            out.extend_from_slice(raw);
        }
        _ => panic!("ipp: value/tag mismatch encoding attribute"),
    }
}

pub fn encode_attr(out: &mut Vec<u8>, attr: &Attr) {
    for (i, v) in attr.values.iter().enumerate() {
        out.push(attr.tag);
        if i == 0 {
            enc_name(out, attr.name);
        } else {
            out.extend_from_slice(&0u16.to_be_bytes()); // zero-length name: extra 1setOf value
        }
        enc_value(out, attr.tag, v);
    }
}

/// Builds one IPP request message.
pub struct Request {
    pub operation: u16,
    pub request_id: u32,
    pub operation_attrs: Vec<Attr>,
    pub job_attrs: Vec<Attr>,
    pub data: Vec<u8>,
}

impl Request {
    /// Starts a request with the three attributes every IPP operation needs,
    /// in the order RFC 8010 s3.1.4 requires.
    pub fn new(operation: u16, printer_uri: &str, request_id: u32) -> Self {
        let mut req = Request {
            operation,
            request_id,
            operation_attrs: Vec::new(),
            job_attrs: Vec::new(),
            data: Vec::new(),
        };
        req.op_text(TAG_CHARSET, "attributes-charset", "utf-8");
        req.op_text(TAG_NATURAL_LANGUAGE, "attributes-natural-language", "en");
        req.op_text(TAG_URI, "printer-uri", printer_uri);
        req
    }

    pub fn op_text(&mut self, tag: u8, name: &'static str, value: &str) {
        self.operation_attrs.push(Attr { tag, name, values: vec![AttrValue::Text(value.to_string())] });
    }
    pub fn op_int(&mut self, tag: u8, name: &'static str, value: i32) {
        self.operation_attrs.push(Attr { tag, name, values: vec![AttrValue::Int(value)] });
    }
    pub fn op_bool(&mut self, name: &'static str, value: bool) {
        self.operation_attrs.push(Attr { tag: TAG_BOOLEAN, name, values: vec![AttrValue::Bool(value)] });
    }
    pub fn job_text(&mut self, tag: u8, name: &'static str, value: &str) {
        self.job_attrs.push(Attr { tag, name, values: vec![AttrValue::Text(value.to_string())] });
    }
    pub fn job_int(&mut self, tag: u8, name: &'static str, value: i32) {
        self.job_attrs.push(Attr { tag, name, values: vec![AttrValue::Int(value)] });
    }
    pub fn job_resolution(&mut self, name: &'static str, x: i32, y: i32, units: u8) {
        self.job_attrs.push(Attr { tag: TAG_RESOLUTION, name, values: vec![AttrValue::Resolution(x, y, units)] });
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256 + self.data.len());
        out.push(2);
        out.push(0); // IPP/2.0
        out.extend_from_slice(&self.operation.to_be_bytes());
        out.extend_from_slice(&self.request_id.to_be_bytes());

        out.push(TAG_OPERATION_ATTRS);
        for a in &self.operation_attrs {
            encode_attr(&mut out, a);
        }
        if !self.job_attrs.is_empty() {
            out.push(TAG_JOB_ATTRS);
            for a in &self.job_attrs {
                encode_attr(&mut out, a);
            }
        }
        out.push(TAG_END_OF_ATTRS);
        out.extend_from_slice(&self.data);
        out
    }
}

/// A group of attributes under one delimiter tag (e.g. one job in Get-Jobs).
pub type Group = BTreeMap<String, Value>;

#[derive(Debug)]
pub struct Response {
    pub status_code: u16,
    pub request_id: u32,
    /// Every delimited group in encounter order, tagged by its delimiter.
    pub groups: Vec<(u8, Group)>,
}

#[derive(Debug)]
pub struct IppError {
    pub code: u16,
    pub detail: String,
}

impl fmt::Display for IppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IPP status 0x{:04x}", self.code)?;
        if !self.detail.is_empty() {
            write!(f, ": {}", self.detail)?;
        }
        Ok(())
    }
}
impl std::error::Error for IppError {}

pub fn is_success(code: u16) -> bool {
    code < 0x0100
}

impl Response {
    pub fn group(&self, delim: u8) -> Group {
        let mut merged = Group::new();
        for (tag, g) in &self.groups {
            if *tag == delim {
                for (k, v) in g {
                    merged.insert(k.clone(), v.clone());
                }
            }
        }
        merged
    }
    pub fn printer_attrs(&self) -> Group {
        self.group(TAG_PRINTER_ATTRS)
    }
    pub fn job_attrs(&self) -> Group {
        self.group(TAG_JOB_ATTRS)
    }
    pub fn job_groups(&self) -> Vec<Group> {
        self.groups.iter().filter(|(t, _)| *t == TAG_JOB_ATTRS).map(|(_, g)| g.clone()).collect()
    }
    pub fn ok(&self) -> bool {
        is_success(self.status_code)
    }
    pub fn raise_for_status(&self) -> Result<(), IppError> {
        if self.ok() {
            return Ok(());
        }
        let unsupported = self.group(TAG_UNSUPPORTED_ATTRS);
        let detail = if !unsupported.is_empty() {
            format!("printer rejected: {}", unsupported.keys().cloned().collect::<Vec<_>>().join(", "))
        } else {
            self.group(TAG_OPERATION_ATTRS)
                .get("status-message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        Err(IppError { code: self.status_code, detail })
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn need(&self, n: usize) -> Result<(), IppError> {
        if self.pos + n > self.buf.len() {
            return Err(IppError {
                code: 0,
                detail: format!(
                    "truncated IPP message: wanted {} bytes at offset {}, only {} remain",
                    n,
                    self.pos,
                    self.buf.len().saturating_sub(self.pos)
                ),
            });
        }
        Ok(())
    }
    fn u8(&mut self) -> Result<u8, IppError> {
        self.need(1)?;
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }
    fn u16(&mut self) -> Result<u16, IppError> {
        self.need(2)?;
        let v = u16::from_be_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, IppError> {
        self.need(4)?;
        let v = u32::from_be_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }
    fn raw(&mut self, n: usize) -> Result<&'a [u8], IppError> {
        self.need(n)?;
        let v = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(v)
    }
}

fn dec_value(tag: u8, raw: &[u8]) -> Value {
    match tag {
        TAG_INTEGER | TAG_ENUM => {
            if raw.len() == 4 {
                let v = i32::from_be_bytes(raw.try_into().unwrap());
                if tag == TAG_ENUM { Value::Enum(v) } else { Value::Integer(v) }
            } else {
                Value::Integer(0)
            }
        }
        TAG_BOOLEAN => Value::Boolean(raw.first().map(|b| *b != 0).unwrap_or(false)),
        TAG_RESOLUTION => {
            if raw.len() == 9 {
                let x = i32::from_be_bytes(raw[0..4].try_into().unwrap());
                let y = i32::from_be_bytes(raw[4..8].try_into().unwrap());
                Value::Resolution(x, y, raw[8])
            } else {
                Value::Resolution(0, 0, 0)
            }
        }
        TAG_RANGE_OF_INTEGER => {
            if raw.len() == 8 {
                let lo = i32::from_be_bytes(raw[0..4].try_into().unwrap());
                let hi = i32::from_be_bytes(raw[4..8].try_into().unwrap());
                Value::Range(lo, hi)
            } else {
                Value::Range(0, 0)
            }
        }
        TAG_UNKNOWN | TAG_NO_VALUE => Value::NoValue,
        _ => Value::Text(String::from_utf8_lossy(raw).into_owned()),
    }
}

/// Decodes an IPP response message (RFC 8010 s3.1.1).
pub fn decode_response(buf: &[u8]) -> Result<Response, IppError> {
    if buf.len() < 8 {
        return Err(IppError { code: 0, detail: format!("response too short: {} bytes", buf.len()) });
    }
    let mut r = Reader { buf, pos: 0 };
    let _major = r.u8()?;
    let _minor = r.u8()?;
    let status_code = r.u16()?;
    let request_id = r.u32()?;

    let mut groups: Vec<(u8, Group)> = Vec::new();
    let mut current_tag: Option<u8> = None;
    let mut current: Group = Group::new();
    let mut last_name: Option<String> = None;

    macro_rules! flush {
        () => {
            if let Some(t) = current_tag {
                groups.push((t, std::mem::take(&mut current)));
            }
        };
    }

    loop {
        if r.pos >= buf.len() {
            break;
        }
        let tag = r.u8()?;

        if tag == TAG_END_OF_ATTRS {
            flush!();
            current_tag = None;
            break;
        }
        if matches!(tag, TAG_OPERATION_ATTRS | TAG_JOB_ATTRS | TAG_PRINTER_ATTRS | TAG_UNSUPPORTED_ATTRS) {
            flush!();
            current_tag = Some(tag);
            last_name = None;
            continue;
        }

        let name_len = r.u16()? as usize;
        let name = if name_len > 0 { String::from_utf8_lossy(r.raw(name_len)?).into_owned() } else { String::new() };
        let val_len = r.u16()? as usize;
        let raw = r.raw(val_len)?;
        let value = dec_value(tag, raw);

        let key = if !name.is_empty() {
            last_name = Some(name.clone());
            name
        } else if let Some(n) = &last_name {
            n.clone()
        } else {
            continue;
        };

        current
            .entry(key)
            .and_modify(|existing| {
                let old = std::mem::replace(existing, Value::NoValue);
                *existing = match old {
                    Value::Multi(mut items) => {
                        items.push(value.clone());
                        Value::Multi(items)
                    }
                    other => Value::Multi(vec![other, value.clone()]),
                };
            })
            .or_insert(value);
    }

    flush!();
    Ok(Response { status_code, request_id, groups })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_response(status: u16, rid: u32, groups: &[(u8, Vec<(u8, &'static str, AttrValue)>)]) -> Vec<u8> {
        let mut out = vec![2, 0];
        out.extend_from_slice(&status.to_be_bytes());
        out.extend_from_slice(&rid.to_be_bytes());
        for (delim, attrs) in groups {
            out.push(*delim);
            for (tag, name, value) in attrs {
                let attr = Attr { tag: *tag, name, values: vec![match value {
                    AttrValue::Int(i) => AttrValue::Int(*i),
                    AttrValue::Bool(b) => AttrValue::Bool(*b),
                    AttrValue::Text(s) => AttrValue::Text(s.clone()),
                    AttrValue::Resolution(x, y, u) => AttrValue::Resolution(*x, *y, *u),
                }] };
                encode_attr(&mut out, &attr);
            }
        }
        out.push(TAG_END_OF_ATTRS);
        out
    }

    #[test]
    fn request_header_layout() {
        let req = Request::new(OP_GET_PRINTER_ATTRIBUTES, "ipp://192.168.1.50:631/ipp/print", 42);
        let blob = req.encode();
        assert_eq!(&blob[0..2], &[2, 0]);
        assert_eq!(u16::from_be_bytes([blob[2], blob[3]]), OP_GET_PRINTER_ATTRIBUTES);
        assert_eq!(u32::from_be_bytes(blob[4..8].try_into().unwrap()), 42);
        assert_eq!(blob[8], TAG_OPERATION_ATTRS);
        // No document data was attached, so the message ends right at the
        // end-of-attributes tag.
        assert_eq!(*blob.last().unwrap(), TAG_END_OF_ATTRS);
        assert!(blob.contains(&TAG_END_OF_ATTRS));
        assert_eq!(blob[9], TAG_CHARSET);
    }

    #[test]
    fn round_trip_every_value_type() {
        let groups = vec![(
            TAG_PRINTER_ATTRS,
            vec![
                (TAG_NAME, "printer-name", AttrValue::Text("ENVY 6432".into())),
                (TAG_ENUM, "printer-state", AttrValue::Int(3)),
                (TAG_INTEGER, "queued-job-count", AttrValue::Int(0)),
                (TAG_BOOLEAN, "color-supported", AttrValue::Bool(true)),
                (TAG_RESOLUTION, "printer-resolution-default", AttrValue::Resolution(600, 600, 3)),
                (TAG_KEYWORD, "media-default", AttrValue::Text("iso_a4_210x297mm".into())),
            ],
        )];
        let resp = decode_response(&build_response(0x0000, 42, &groups)).unwrap();
        assert!(resp.ok());
        assert_eq!(resp.request_id, 42);
        let pa = resp.printer_attrs();
        assert_eq!(pa["printer-name"].as_str(), Some("ENVY 6432"));
        assert_eq!(pa["printer-state"].as_i32(), Some(3));
        assert_eq!(pa["queued-job-count"].as_i32(), Some(0));
        assert_eq!(pa["color-supported"].as_bool(), Some(true));
        assert_eq!(pa["printer-resolution-default"], Value::Resolution(600, 600, 3));
        assert_eq!(pa["media-default"].as_str(), Some("iso_a4_210x297mm"));
    }

    #[test]
    fn one_set_of_encoding_uses_zero_length_name_for_extras() {
        let attr = Attr {
            tag: TAG_KEYWORD,
            name: "sides",
            values: vec![AttrValue::Text("one-sided".into()), AttrValue::Text("two-sided-long-edge".into())],
        };
        let mut out = Vec::new();
        encode_attr(&mut out, &attr);
        assert_eq!(out[0], TAG_KEYWORD);
        let nl = u16::from_be_bytes([out[1], out[2]]) as usize;
        assert_eq!(nl, "sides".len());
        let mut off = 3 + nl;
        let vl = u16::from_be_bytes([out[off], out[off + 1]]) as usize;
        off += 2 + vl;
        assert_eq!(out[off], TAG_KEYWORD);
        assert_eq!(u16::from_be_bytes([out[off + 1], out[off + 2]]), 0, "second value has a zero-length name");
    }

    #[test]
    fn marker_levels_decode_as_multi() {
        let groups = vec![(
            TAG_PRINTER_ATTRS,
            vec![
                (TAG_INTEGER, "marker-levels", AttrValue::Int(12)),
                // second value of the same 1setOf, same attribute, encoded manually below
            ],
        )];
        // Hand-build the multi-value case: zero-length name on the second value.
        let mut out = vec![2u8, 0];
        out.extend_from_slice(&0x0000u16.to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.push(TAG_PRINTER_ATTRS);
        let a1 = Attr { tag: TAG_INTEGER, name: "marker-levels", values: vec![AttrValue::Int(12), AttrValue::Int(4)] };
        encode_attr(&mut out, &a1);
        out.push(TAG_END_OF_ATTRS);
        let _ = groups; // silence unused in this hand-built variant
        let resp = decode_response(&out).unwrap();
        assert_eq!(resp.printer_attrs()["marker-levels"].ints(), vec![12, 4]);
    }

    #[test]
    fn error_status_and_unsupported_attrs() {
        let mut out = vec![2u8, 0];
        out.extend_from_slice(&0x040Au16.to_be_bytes());
        out.extend_from_slice(&7u32.to_be_bytes());
        out.push(TAG_UNSUPPORTED_ATTRS);
        encode_attr(&mut out, &Attr { tag: TAG_UNKNOWN, name: "document-format", values: vec![AttrValue::Text(String::new())] });
        out.push(TAG_END_OF_ATTRS);
        let resp = decode_response(&out).unwrap();
        assert!(!resp.ok());
        let err = resp.raise_for_status().unwrap_err();
        assert_eq!(err.code, 0x040A);
        assert!(err.detail.contains("document-format"));
    }

    #[test]
    fn truncated_input_errors_instead_of_misparsing() {
        assert!(decode_response(&[2, 0, 0]).is_err());
        let mut short = vec![2u8, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_ATTRS, TAG_NAME];
        short.extend_from_slice(&4u16.to_be_bytes());
        short.extend_from_slice(b"name");
        short.extend_from_slice(&99u16.to_be_bytes()); // claims 99 bytes, supplies 2
        short.extend_from_slice(b"xx");
        assert!(decode_response(&short).is_err());
    }

    #[test]
    fn multiple_job_groups_stay_separate() {
        let mut out = vec![2u8, 0];
        out.extend_from_slice(&0x0000u16.to_be_bytes());
        out.extend_from_slice(&9u32.to_be_bytes());
        for (id, name) in [(11, "a"), (12, "b")] {
            out.push(TAG_JOB_ATTRS);
            encode_attr(&mut out, &Attr { tag: TAG_INTEGER, name: "job-id", values: vec![AttrValue::Int(id)] });
            encode_attr(&mut out, &Attr { tag: TAG_NAME, name: "job-name", values: vec![AttrValue::Text(name.into())] });
        }
        out.push(TAG_END_OF_ATTRS);
        let resp = decode_response(&out).unwrap();
        let jobs = resp.job_groups();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0]["job-id"].as_i32(), Some(11));
        assert_eq!(jobs[1]["job-id"].as_i32(), Some(12));
    }
}
