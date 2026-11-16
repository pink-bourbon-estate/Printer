//! IPP client: talks to a printer on the local network over plain HTTP.
//!
//! Only `ureq` (an HTTP client) is used to reach it. Nothing is contacted
//! except the printer's own IP address on your LAN: no vendor cloud, no
//! account, no telemetry.

use crate::ipp::*;
use std::time::Duration;

pub const PREFERRED_FORMATS: &[&str] = &["application/pdf", "image/pwg-raster", "image/jpeg", "image/urf"];

#[derive(Debug)]
pub struct InkLevel {
    pub name: String,
    pub level: i32, // percent, -1 if unknown
    pub kind: String,
}

impl std::fmt::Display for InkLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.level >= 0 {
            write!(f, "{}: {}%", self.name, self.level)
        } else {
            write!(f, "{}: unknown", self.name)
        }
    }
}

#[derive(Debug, Default)]
pub struct PrinterInfo {
    pub name: String,
    pub make_and_model: String,
    pub state: String,
    pub state_reasons: Vec<String>,
    pub is_accepting: bool,
    pub color_supported: bool,
    pub document_formats: Vec<String>,
    pub media_default: String,
    pub media_supported: Vec<String>,
    pub sides_supported: Vec<String>,
    pub ink: Vec<InkLevel>,
}

impl PrinterInfo {
    pub fn supports(&self, fmt: &str) -> bool {
        self.document_formats.iter().any(|f| f.eq_ignore_ascii_case(fmt))
    }
    pub fn best_format(&self) -> Option<&str> {
        if self.document_formats.is_empty() {
            return None;
        }
        for pref in PREFERRED_FORMATS {
            if self.supports(pref) {
                return Some(pref);
            }
        }
        Some(self.document_formats[0].as_str())
    }
}

pub struct IppPrinter {
    pub host: String,
    pub port: u16,
    pub resource: String,
    request_id: std::cell::Cell<u32>,
    timeout: Duration,
}

impl IppPrinter {
    pub fn new(host: &str, port: u16, resource: &str) -> Self {
        let resource = if resource.starts_with('/') { resource.to_string() } else { format!("/{resource}") };
        IppPrinter { host: host.to_string(), port, resource, request_id: std::cell::Cell::new(0), timeout: Duration::from_secs(15) }
    }

    pub fn with_timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }

    pub fn uri(&self) -> String {
        if self.port == 631 {
            format!("ipp://{}{}", self.host, self.resource)
        } else {
            format!("ipp://{}:{}{}", self.host, self.port, self.resource)
        }
    }
    pub fn http_url(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.resource)
    }
    pub fn ews_url(&self) -> String {
        format!("http://{}/", self.host)
    }

    fn next_id(&self) -> u32 {
        let v = self.request_id.get() + 1;
        self.request_id.set(v);
        v
    }

    fn post(&self, payload: &[u8]) -> anyhow::Result<Response> {
        let agent = ureq::AgentBuilder::new().timeout(self.timeout).build();
        let result = agent
            .post(&self.http_url())
            .set("Content-Type", "application/ipp")
            .set("Accept", "application/ipp")
            .set("User-Agent", "printer-rs/1.0")
            .send_bytes(payload);

        let body: Vec<u8> = match result {
            Ok(resp) => {
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf)?;
                buf
            }
            Err(ureq::Error::Status(code, resp)) => {
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut resp.into_reader(), &mut buf).ok();
                if buf.is_empty() {
                    anyhow::bail!("{}:{} answered HTTP {} for {}", self.host, self.port, code, self.resource);
                }
                buf
            }
            Err(e) => anyhow::bail!("cannot reach {}:{} - {}", self.host, self.port, e),
        };

        decode_response(&body).map_err(|e| anyhow::anyhow!("{e}"))
    }

    pub fn get_printer_attributes(&self) -> anyhow::Result<Group> {
        let mut req = Request::new(OP_GET_PRINTER_ATTRIBUTES, &self.uri(), self.next_id());
        req.operation_attrs.push(Attr {
            tag: TAG_KEYWORD,
            name: "requested-attributes",
            values: vec![AttrValue::Text("all".into())],
        });
        let resp = self.post(&req.encode())?;
        resp.raise_for_status()?;
        Ok(resp.printer_attrs())
    }

    pub fn info(&self) -> anyhow::Result<PrinterInfo> {
        let a = self.get_printer_attributes()?;
        let state_num = a.get("printer-state").and_then(|v| v.as_i32()).unwrap_or(0);
        let state = match state_num {
            3 => "idle",
            4 => "processing",
            5 => "stopped",
            _ => "",
        }
        .to_string();

        let names = a.get("marker-names").map(|v| v.strings()).unwrap_or_default();
        let levels = a.get("marker-levels").map(|v| v.ints()).unwrap_or_default();
        let types = a.get("marker-types").map(|v| v.strings()).unwrap_or_default();
        let ink = (0..names.len().max(levels.len()))
            .map(|i| InkLevel {
                name: names.get(i).cloned().unwrap_or_else(|| format!("supply {}", i + 1)),
                level: levels.get(i).copied().unwrap_or(-1),
                kind: types.get(i).cloned().unwrap_or_default(),
            })
            .collect();

        Ok(PrinterInfo {
            name: a.get("printer-name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            make_and_model: a.get("printer-make-and-model").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            state,
            state_reasons: a.get("printer-state-reasons").map(|v| v.strings()).unwrap_or_default(),
            is_accepting: a.get("printer-is-accepting-jobs").and_then(|v| v.as_bool()).unwrap_or(true),
            color_supported: a.get("color-supported").and_then(|v| v.as_bool()).unwrap_or(false),
            document_formats: a.get("document-format-supported").map(|v| v.strings()).unwrap_or_default(),
            media_default: a.get("media-default").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            media_supported: a.get("media-supported").map(|v| v.strings()).unwrap_or_default(),
            sides_supported: a.get("sides-supported").map(|v| v.strings()).unwrap_or_default(),
            ink,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn print_bytes(
        &self,
        data: &[u8],
        document_format: &str,
        job_name: &str,
        copies: i32,
        media: Option<&str>,
        sides: Option<&str>,
        color: Option<bool>,
        resolution: Option<(i32, i32)>,
    ) -> anyhow::Result<i32> {
        let mut req = Request::new(OP_PRINT_JOB, &self.uri(), self.next_id());
        req.op_text(TAG_NAME, "requesting-user-name", "owner");
        req.op_text(TAG_NAME, "job-name", &job_name.chars().take(255).collect::<String>());
        req.op_text(TAG_MIME_MEDIA_TYPE, "document-format", document_format);
        if copies > 1 {
            req.job_int(TAG_INTEGER, "copies", copies);
        }
        if let Some(m) = media {
            req.job_text(TAG_KEYWORD, "media", m);
        }
        if let Some(s) = sides {
            req.job_text(TAG_KEYWORD, "sides", s);
        }
        if let Some(c) = color {
            req.job_text(TAG_KEYWORD, "print-color-mode", if c { "color" } else { "monochrome" });
        }
        if let Some((x, y)) = resolution {
            req.job_resolution("printer-resolution", x, y, 3);
        }
        req.data = data.to_vec();

        let resp = self.post(&req.encode())?;
        resp.raise_for_status()?;
        Ok(resp.job_attrs().get("job-id").and_then(|v| v.as_i32()).unwrap_or(0))
    }

    pub fn validate(&self, document_format: &str) -> anyhow::Result<(bool, String)> {
        let mut req = Request::new(OP_VALIDATE_JOB, &self.uri(), self.next_id());
        req.op_text(TAG_NAME, "requesting-user-name", "owner");
        req.op_text(TAG_MIME_MEDIA_TYPE, "document-format", document_format);
        let resp = self.post(&req.encode())?;
        if resp.ok() {
            Ok((true, "accepted".into()))
        } else {
            let unsupported = resp.group(TAG_UNSUPPORTED_ATTRS);
            let detail = if !unsupported.is_empty() {
                unsupported.keys().cloned().collect::<Vec<_>>().join(", ")
            } else {
                format!("status 0x{:04x}", resp.status_code)
            };
            Ok((false, detail))
        }
    }

    pub fn create_job(&self, job_name: &str, copies: i32) -> anyhow::Result<i32> {
        let mut req = Request::new(OP_CREATE_JOB, &self.uri(), self.next_id());
        req.op_text(TAG_NAME, "requesting-user-name", "owner");
        req.op_text(TAG_NAME, "job-name", &job_name.chars().take(255).collect::<String>());
        if copies > 1 {
            req.job_int(TAG_INTEGER, "copies", copies);
        }
        let resp = self.post(&req.encode())?;
        resp.raise_for_status()?;
        Ok(resp.job_attrs().get("job-id").and_then(|v| v.as_i32()).unwrap_or(0))
    }

    pub fn send_document(&self, job_id: i32, data: &[u8], document_format: &str, last: bool) -> anyhow::Result<()> {
        let mut req = Request::new(OP_SEND_DOCUMENT, &self.uri(), self.next_id());
        req.op_int(TAG_INTEGER, "job-id", job_id);
        req.op_text(TAG_NAME, "requesting-user-name", "owner");
        req.op_text(TAG_MIME_MEDIA_TYPE, "document-format", document_format);
        req.op_bool("last-document", last);
        req.data = data.to_vec();
        self.post(&req.encode())?.raise_for_status()?;
        Ok(())
    }

    pub fn jobs(&self, completed: bool) -> anyhow::Result<Vec<Group>> {
        let mut req = Request::new(OP_GET_JOBS, &self.uri(), self.next_id());
        req.op_text(TAG_KEYWORD, "which-jobs", if completed { "completed" } else { "not-completed" });
        let resp = self.post(&req.encode())?;
        resp.raise_for_status()?;
        Ok(resp.job_groups())
    }

    pub fn cancel(&self, job_id: i32) -> anyhow::Result<bool> {
        let mut req = Request::new(OP_CANCEL_JOB, &self.uri(), self.next_id());
        req.op_int(TAG_INTEGER, "job-id", job_id);
        req.op_text(TAG_NAME, "requesting-user-name", "owner");
        Ok(self.post(&req.encode())?.ok())
    }
}

/// Resource paths seen on IPP Everywhere hardware, tried in order.
pub const CANDIDATE_RESOURCES: &[&str] = &["/ipp/print", "/ipp/printer", "/ipp", "/printers/print", "/"];

pub fn probe(host: &str, port: u16) -> Option<IppPrinter> {
    for resource in CANDIDATE_RESOURCES {
        let printer = IppPrinter::new(host, port, resource).with_timeout(Duration::from_secs(3));
        if printer.get_printer_attributes().map(|a| !a.is_empty()).unwrap_or(false) {
            return Some(printer);
        }
    }
    None
}

pub fn job_state_name(n: i32) -> &'static str {
    match n {
        3 => "pending",
        4 => "pending-held",
        5 => "processing",
        6 => "processing-stopped",
        7 => "canceled",
        8 => "aborted",
        9 => "completed",
        _ => "unknown",
    }
}
