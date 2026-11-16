//! eSCL (AirScan) client: driverless scanning over the local network.
//!
//! Written from the public eSCL specification — the protocol Apple published
//! as AirScan, which HP, Canon, Epson and Brother all implement in printer
//! firmware. No HP software, no account, no internet connection.

use roxmltree::Document;
use std::time::Duration;

pub const SOURCE_PLATEN: &str = "Platen";
pub const SOURCE_FEEDER: &str = "Feeder";
pub const COLOR_RGB: &str = "RGB24";
pub const COLOR_GRAY: &str = "Grayscale8";

/// A4 at 1/300-inch eSCL units.
pub const PAGE_A4: (u32, u32) = (2480, 3507);
pub const PAGE_LETTER: (u32, u32) = (2550, 3300);

#[derive(Debug, Default)]
pub struct ScannerCapabilities {
    pub make_and_model: String,
    pub platen: bool,
    pub feeder: bool,
    pub resolutions: Vec<u32>,
    pub color_modes: Vec<String>,
    pub formats: Vec<String>,
    pub max_width: u32,
    pub max_height: u32,
}

impl ScannerCapabilities {
    pub fn nearest_resolution(&self, wanted: u32) -> u32 {
        if self.resolutions.is_empty() {
            return wanted;
        }
        *self.resolutions.iter().min_by_key(|r| (r.abs_diff(wanted), **r)).unwrap()
    }
}

pub struct ScanSettings {
    pub source: String,
    pub resolution: u32,
    pub color_mode: String,
    pub document_format: String,
    pub width: u32,
    pub height: u32,
}

impl ScanSettings {
    pub fn for_page(w: u32, h: u32, source: &str) -> Self {
        ScanSettings { source: source.into(), resolution: 300, color_mode: COLOR_RGB.into(), document_format: "image/jpeg".into(), width: w, height: h }
    }

    fn to_xml(&self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScanSettings xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm" xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03">
<pwg:Version>2.6</pwg:Version>
<scan:Intent>Document</scan:Intent>
<pwg:ScanRegions>
<pwg:ScanRegion>
<pwg:XOffset>0</pwg:XOffset>
<pwg:YOffset>0</pwg:YOffset>
<pwg:Width>{}</pwg:Width>
<pwg:Height>{}</pwg:Height>
<pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>
</pwg:ScanRegion>
</pwg:ScanRegions>
<pwg:InputSource>{}</pwg:InputSource>
<pwg:DocumentFormat>{}</pwg:DocumentFormat>
<scan:ColorMode>{}</scan:ColorMode>
<scan:XResolution>{}</scan:XResolution>
<scan:YResolution>{}</scan:YResolution>
</scan:ScanSettings>"#,
            self.width, self.height, self.source, self.document_format, self.color_mode, self.resolution, self.resolution
        )
    }
}

pub struct ScannedPage {
    pub index: usize,
    pub data: Vec<u8>,
    pub content_type: String,
}

impl ScannedPage {
    pub fn extension(&self) -> &'static str {
        let ct = self.content_type.to_lowercase();
        if ct.contains("png") {
            "png"
        } else if ct.contains("pdf") {
            "pdf"
        } else {
            "jpg"
        }
    }
}

pub struct EsclScanner {
    pub host: String,
    pub port: u16,
    pub root: String,
    timeout: Duration,
}

impl EsclScanner {
    pub fn new(host: &str, port: u16, root: &str) -> Self {
        let root = if root.starts_with('/') { root.to_string() } else { format!("/{root}") };
        EsclScanner { host: host.to_string(), port, root, timeout: Duration::from_secs(15) }
    }

    pub fn base(&self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.root)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(self.timeout).build()
    }

    fn get_text(&self, path: &str) -> anyhow::Result<String> {
        let url = self.base() + path;
        let resp = self.agent().get(&url).call().map_err(|e| anyhow::anyhow!("{} - {}", path, e))?;
        Ok(resp.into_string()?)
    }

    pub fn is_reachable(&self) -> bool {
        self.get_text("/ScannerStatus").is_ok()
    }

    fn text_at(doc: &Document, local_name: &str) -> String {
        doc.descendants()
            .find(|n| n.has_tag_name(local_name))
            .and_then(|n| n.text())
            .unwrap_or("")
            .trim()
            .to_string()
    }

    pub fn capabilities(&self) -> anyhow::Result<ScannerCapabilities> {
        let text = self.get_text("/ScannerCapabilities")?;
        let doc = Document::parse(&text)?;
        let root = doc.root_element();

        let mut caps = ScannerCapabilities { max_width: PAGE_A4.0, max_height: PAGE_A4.1, ..Default::default() };
        caps.make_and_model = Self::text_at(&doc, "MakeAndModel");
        caps.platen = root.descendants().any(|n| n.has_tag_name("Platen"));
        caps.feeder = root.descendants().any(|n| n.has_tag_name("Adf"));

        let mut res: Vec<u32> = root
            .descendants()
            .filter(|n| n.has_tag_name("XResolution"))
            .filter_map(|n| n.text().and_then(|t| t.trim().parse().ok()))
            .collect();
        res.sort_unstable();
        res.dedup();
        caps.resolutions = res;

        let mut modes: Vec<String> = root.descendants().filter(|n| n.has_tag_name("ColorMode")).filter_map(|n| n.text().map(str::to_string)).collect();
        modes.sort();
        modes.dedup();
        caps.color_modes = modes;

        let mut fmts: Vec<String> = root
            .descendants()
            .filter(|n| n.has_tag_name("DocumentFormat") || n.has_tag_name("DocumentFormatExt"))
            .filter_map(|n| n.text().map(str::to_string))
            .collect();
        fmts.sort();
        fmts.dedup();
        caps.formats = fmts;

        if let Some(w) = root.descendants().filter(|n| n.has_tag_name("MaxWidth")).filter_map(|n| n.text().and_then(|t| t.trim().parse().ok())).max() {
            caps.max_width = w;
        }
        if let Some(h) = root.descendants().filter(|n| n.has_tag_name("MaxHeight")).filter_map(|n| n.text().and_then(|t| t.trim().parse().ok())).max() {
            caps.max_height = h;
        }

        Ok(caps)
    }

    pub fn status(&self) -> anyhow::Result<(String, String)> {
        let text = self.get_text("/ScannerStatus")?;
        let doc = Document::parse(&text)?;
        Ok((Self::text_at(&doc, "State"), Self::text_at(&doc, "AdfState")))
    }

    pub fn has_paper_in_feeder(&self) -> bool {
        self.status().map(|(_, adf)| adf.eq_ignore_ascii_case("ScannerAdfLoaded")).unwrap_or(false)
    }

    fn submit(&self, settings: &ScanSettings) -> anyhow::Result<String> {
        let url = self.base() + "/ScanJobs";
        let resp = self
            .agent()
            .post(&url)
            .set("Content-Type", "text/xml")
            .send_string(&settings.to_xml())
            .map_err(|e| anyhow::anyhow!("the scanner refused the scan job - {e}"))?;
        let location = resp.header("Location").unwrap_or("").to_string();
        if location.is_empty() {
            anyhow::bail!("scan job accepted but no job location was returned");
        }
        Ok(if location.starts_with("http") { location } else { format!("http://{}:{}{}", self.host, self.port, location) })
    }

    fn next_page(&self, job_uri: &str, index: usize) -> anyhow::Result<Option<ScannedPage>> {
        let url = format!("{}/NextDocument", job_uri.trim_end_matches('/'));
        let agent = self.agent();
        // The lamp warms up and the carriage moves before the first page; the
        // scanner answers 503 meanwhile, so that is waited out, not treated
        // as failure.
        for attempt in 0..40 {
            match agent.get(&url).timeout(Duration::from_secs(120)).call() {
                Ok(resp) => {
                    let content_type = resp.header("Content-Type").unwrap_or("application/octet-stream").to_string();
                    let mut data = Vec::new();
                    std::io::Read::read_to_end(&mut resp.into_reader(), &mut data)?;
                    if data.is_empty() {
                        return Ok(None);
                    }
                    return Ok(Some(ScannedPage { index, data, content_type }));
                }
                Err(ureq::Error::Status(404, _)) | Err(ureq::Error::Status(410, _)) => return Ok(None), // normal end of job
                Err(ureq::Error::Status(503, _)) | Err(ureq::Error::Status(500, _)) if attempt < 39 => {
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => anyhow::bail!("fetching page {} failed: {e}", index + 1),
            }
        }
        Ok(None)
    }

    pub fn cancel(&self, job_uri: &str) {
        let _ = self.agent().request("DELETE", job_uri).call();
    }

    /// Runs a scan to completion, calling `on_page` as each page arrives.
    pub fn scan(&self, settings: &ScanSettings, mut on_page: impl FnMut(&ScannedPage)) -> anyhow::Result<Vec<ScannedPage>> {
        let job_uri = self.submit(settings)?;
        let mut pages = Vec::new();
        for i in 0..200 {
            match self.next_page(&job_uri, i) {
                Ok(Some(page)) => {
                    on_page(&page);
                    pages.push(page);
                    if settings.source == SOURCE_PLATEN {
                        break; // the flatbed holds exactly one sheet
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    self.cancel(&job_uri);
                    return Err(e);
                }
            }
        }
        Ok(pages)
    }
}

/// Ports and roots eSCL devices are found on. HP uses 8080 most often.
pub const CANDIDATE_ENDPOINTS: &[(u16, &str)] = &[(8080, "/eSCL"), (80, "/eSCL")];

pub fn probe(host: &str) -> Option<EsclScanner> {
    for (port, root) in CANDIDATE_ENDPOINTS {
        let scanner = EsclScanner::new(host, *port, root);
        if scanner.is_reachable() {
            return Some(scanner);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_settings_xml_is_well_formed_and_ordered() {
        let s = ScanSettings::for_page(PAGE_A4.0, PAGE_A4.1, SOURCE_PLATEN);
        let xml = s.to_xml();
        assert!(roxmltree::Document::parse(&xml).is_ok(), "settings XML must parse");
        assert!(xml.contains("<pwg:InputSource>Platen</pwg:InputSource>"));
        assert!(xml.contains("escl:ThreeHundredthsOfInches"));
        let regions = xml.find("<pwg:ScanRegions>").unwrap();
        let source = xml.find("<pwg:InputSource>").unwrap();
        let color = xml.find("<scan:ColorMode>").unwrap();
        assert!(regions < source && source < color, "elements must appear in schema order");
    }

    #[test]
    fn nearest_resolution_snaps_correctly() {
        let caps = ScannerCapabilities { resolutions: vec![75, 300, 600], ..Default::default() };
        assert_eq!(caps.nearest_resolution(400), 300);
        assert_eq!(caps.nearest_resolution(600), 600);
        assert_eq!(caps.nearest_resolution(50), 75);
    }

    #[test]
    fn capabilities_xml_parses_platen_and_adf() {
        let xml = r#"<?xml version="1.0"?>
<scan:ScannerCapabilities xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm" xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03">
<pwg:MakeAndModel>HP ENVY 6400 series</pwg:MakeAndModel>
<scan:Platen><scan:PlatenInputCaps>
<scan:SettingProfiles><scan:SettingProfile>
<scan:ColorModes><scan:ColorMode>RGB24</scan:ColorMode><scan:ColorMode>Grayscale8</scan:ColorMode></scan:ColorModes>
<scan:DocumentFormats><pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat></scan:DocumentFormats>
<scan:SupportedResolutions><scan:DiscreteResolutions>
<scan:DiscreteResolution><scan:XResolution>300</scan:XResolution><scan:YResolution>300</scan:YResolution></scan:DiscreteResolution>
</scan:DiscreteResolutions></scan:SupportedResolutions>
</scan:SettingProfile></scan:SettingProfiles>
</scan:PlatenInputCaps></scan:Platen>
<scan:Adf><scan:AdfSimplexInputCaps/></scan:Adf>
</scan:ScannerCapabilities>"#;
        let doc = Document::parse(xml).unwrap();
        let root = doc.root_element();
        assert!(root.descendants().any(|n| n.has_tag_name("Platen")));
        assert!(root.descendants().any(|n| n.has_tag_name("Adf")));
        assert_eq!(EsclScanner::text_at(&doc, "MakeAndModel"), "HP ENVY 6400 series");
    }
}
