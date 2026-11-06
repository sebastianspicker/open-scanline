use super::*;

pub(crate) fn requested_document_dimensions(request: &ScanRequest) -> (u32, u32) {
    request
        .region
        .map(|region| (region.width, region.height))
        .unwrap_or((request.width, request.height))
}

pub(crate) fn requested_document_pixels(request: &ScanRequest) -> Result<u64> {
    let (width, height) = requested_document_dimensions(request);
    crate::domain::image::checked_image_len(width, height, request.pixel_format.bpp())?;
    u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| ScanError::Invalid("eSCL document dimensions overflow".into()))
}

pub(crate) fn document_decoded_bytes(
    request: &ScanRequest,
    representation: DocumentRepresentation,
) -> Result<u64> {
    let bytes = requested_document_pixels(request)?
        .checked_mul(representation.decoded_bytes_per_pixel())
        .ok_or_else(|| ScanError::Invalid("eSCL document dimensions overflow".into()))?;
    if bytes > MAX_DOCUMENT_DECODE_ALLOCATION {
        return Err(ScanError::Invalid(format!(
            "eSCL {} document exceeds the {MAX_DOCUMENT_DECODE_ALLOCATION}-byte decoded image safety limit",
            representation.name()
        )));
    }
    Ok(bytes)
}

pub(crate) fn document_response_limit(
    request: &ScanRequest,
    representation: DocumentRepresentation,
) -> Result<u64> {
    let response_limit = document_decoded_bytes(request, representation)?
        .checked_add(DOCUMENT_CONTAINER_OVERHEAD)
        .ok_or_else(|| ScanError::Invalid("eSCL document response limit overflow".into()))?;
    if response_limit > MAX_DOCUMENT_RESPONSE_LIMIT {
        return Err(ScanError::Invalid(format!(
            "eSCL {} document exceeds the {MAX_DOCUMENT_RESPONSE_LIMIT}-byte response safety limit",
            representation.name()
        )));
    }
    Ok(response_limit.max(MIN_DOCUMENT_RESPONSE_LIMIT))
}

pub(crate) fn document_decode_limits(
    request: &ScanRequest,
    representation: DocumentRepresentation,
) -> Result<(u32, u32, u64)> {
    let (width, height) = requested_document_dimensions(request);
    let max_width = width
        .saturating_mul(4)
        .clamp(MIN_DOCUMENT_DIMENSION_LIMIT, MAX_DOCUMENT_DIMENSION_LIMIT);
    let max_height = height
        .saturating_mul(4)
        .clamp(MIN_DOCUMENT_DIMENSION_LIMIT, MAX_DOCUMENT_DIMENSION_LIMIT);
    let max_allocation =
        document_decoded_bytes(request, representation)?.max(MIN_DOCUMENT_DECODE_ALLOCATION);
    Ok((max_width, max_height, max_allocation))
}

pub(crate) fn local_name(name: &[u8]) -> String {
    let name = std::str::from_utf8(name).unwrap_or_default();
    name.rsplit(':')
        .next()
        .unwrap_or(name)
        .trim_matches(|character| character == '{' || character == '}')
        .to_ascii_lowercase()
}

pub(crate) fn source_for_path(path: &[String]) -> Option<CapabilitySource> {
    let path = path.join("/");
    if is_duplex_adf(&path) {
        Some(CapabilitySource::AdfDuplex)
    } else if is_simplex_adf(&path) {
        Some(CapabilitySource::AdfSimplex)
    } else if is_film(&path) {
        Some(CapabilitySource::Film)
    } else if is_platen(&path) {
        Some(CapabilitySource::Platen)
    } else {
        None
    }
}

fn is_duplex_adf(path: &str) -> bool {
    path.contains("adfduplex") || path.contains("duplexadf")
}

fn is_simplex_adf(path: &str) -> bool {
    ["adfsimplex", "simplexadf", "feeder", "adf"]
        .iter()
        .any(|name| path.contains(name))
}

fn is_film(path: &str) -> bool {
    path.contains("film") || path.contains("transparen")
}

fn is_platen(path: &str) -> bool {
    path.contains("platen") || path.contains("flatbed")
}

pub(crate) fn source_capabilities_mut(
    capabilities: &mut ScannerCapabilities,
    source: Option<CapabilitySource>,
) -> &mut SourceCapabilities {
    if let Some(index) = capabilities
        .sources
        .iter()
        .position(|candidate| candidate.source == source)
    {
        return &mut capabilities.sources[index];
    }
    capabilities.sources.push(SourceCapabilities {
        source,
        ..SourceCapabilities::default()
    });
    capabilities.sources.last_mut().expect("source inserted")
}

pub(crate) fn push_unique(values: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if !value.is_empty()
        && !values
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(value))
    {
        values.push(value.to_string());
    }
}

#[derive(Default)]
struct RangeBuilder {
    active: bool,
    // [min, max, step] per axis.
    x: [Option<u32>; 3],
    y: [Option<u32>; 3],
}

impl RangeBuilder {
    fn axis(values: [Option<u32>; 3]) -> Option<AxisRange> {
        let [min, max, step] = values;
        let (min, max) = (min?, max?);
        (min > 0 && min <= max).then_some(AxisRange {
            min,
            max,
            step: step.filter(|step| *step > 0).unwrap_or(1),
        })
    }

    fn finish(&mut self) -> Option<ResolutionRange> {
        let range = std::mem::take(self);
        range.active.then_some(())?;
        Some(ResolutionRange {
            x: Self::axis(range.x)?,
            y: Self::axis(range.y)?,
        })
    }
}

struct CapabilityParser {
    capabilities: ScannerCapabilities,
    path: Vec<String>,
    values: Vec<String>,
    current_resolution: Option<(Option<u32>, Option<u32>)>,
    current_range: RangeBuilder,
    saw_capabilities: bool,
}

impl CapabilityParser {
    fn new(root: &str) -> Self {
        Self {
            capabilities: ScannerCapabilities {
                root: root.to_string(),
                ..ScannerCapabilities::default()
            },
            path: Vec::new(),
            values: Vec::new(),
            current_resolution: None,
            current_range: RangeBuilder::default(),
            saw_capabilities: false,
        }
    }

    fn start(&mut self, name: String) -> bool {
        // Bounds the per-element path scans; real capability trees are ~10 deep.
        if self.path.len() >= MAX_XML_DEPTH {
            return false;
        }
        self.saw_capabilities |= name == "scannercapabilities";
        // `Resolution` is the legacy pair element; eSCL profiles list pairs as
        // `DiscreteResolution` inside `DiscreteResolutions`.
        if matches!(name.as_str(), "resolution" | "discreteresolution") {
            self.current_resolution = Some((None, None));
        }
        if name == "resolutionrange" {
            self.current_range = RangeBuilder {
                active: true,
                ..RangeBuilder::default()
            };
        }
        self.path.push(name);
        self.values.push(String::new());
        true
    }

    fn push_text(&mut self, text: &str) {
        if let Some(current) = self.values.last_mut() {
            current.push_str(text);
        }
    }

    fn end(&mut self, name: String) {
        let value = self.values.pop().unwrap_or_default();
        let source = source_for_path(&self.path);
        self.apply_value(&name, &value, source);
        self.path.pop();
    }

    fn source(&mut self, source: Option<CapabilitySource>) -> &mut SourceCapabilities {
        source_capabilities_mut(&mut self.capabilities, source)
    }

    fn apply_value(&mut self, name: &str, value: &str, source: Option<CapabilitySource>) {
        // Keep the original parser's behavior: each closing element establishes
        // the source bucket inferred from its still-open XML path.
        let _ = self.source(source);
        match name {
            "makeandmodel" => {
                let value = value.trim();
                self.capabilities.make_and_model = (!value.is_empty()).then(|| value.into())
            }
            "colormode" => push_unique(&mut self.source(source).color_modes, value),
            "documentformat" | "documentformatext" => {
                push_unique(&mut self.source(source).document_formats, value)
            }
            "inputsource" => self.register_input_source(source, value),
            "maxwidth" | "maxscanwidth" => {
                self.source(source).max_width = value.trim().parse().ok()
            }
            "maxheight" | "maxscanheight" => {
                self.source(source).max_height = value.trim().parse().ok()
            }
            "xresolution" => self.set_resolution_axis(value, true),
            "yresolution" => self.set_resolution_axis(value, false),
            "resolution" | "discreteresolution" => self.finish_resolution(source),
            "resolutionrange" => {
                if let Some(range) = self.current_range.finish() {
                    let ranges = &mut self.source(source).resolution_ranges;
                    if !ranges.contains(&range) {
                        ranges.push(range);
                    }
                }
            }
            "min" | "max" | "step" => self.set_range_bound(name, value),
            _ => {}
        }
    }

    fn register_input_source(&mut self, path_source: Option<CapabilitySource>, value: &str) {
        let _ = self.source(path_source);
        if let Some(source) = input_source(value) {
            let _ = self.source(Some(source));
        }
    }

    fn set_resolution_axis(&mut self, value: &str, x_axis: bool) {
        let Some((x, y)) = self.current_resolution.as_mut() else {
            return;
        };
        if x_axis {
            *x = value.trim().parse().ok();
        } else {
            *y = value.trim().parse().ok();
        }
    }

    fn set_range_bound(&mut self, name: &str, value: &str) {
        if !self.current_range.active {
            return;
        }
        // The closing element is still on the path; its parent names the axis.
        let axis = match self.path.iter().rev().nth(1).map(String::as_str) {
            Some("xresolutionrange") => &mut self.current_range.x,
            Some("yresolutionrange") => &mut self.current_range.y,
            _ => return,
        };
        let index = match name {
            "min" => 0,
            "max" => 1,
            _ => 2,
        };
        axis[index] = value.trim().parse().ok();
    }

    fn finish_resolution(&mut self, source: Option<CapabilitySource>) {
        // Zero is not a resolution; `ResolutionRange` rejects it the same way.
        if let Some((Some(x), Some(y))) = self.current_resolution.take() {
            if x == 0 || y == 0 {
                return;
            }
            self.source(source).resolutions.push((x, y));
        }
    }
}

fn input_source(value: &str) -> Option<CapabilitySource> {
    let value = value.to_ascii_lowercase();
    if value.contains("platen") || value.contains("flatbed") {
        Some(CapabilitySource::Platen)
    } else if value.contains("duplex") {
        Some(CapabilitySource::AdfDuplex)
    } else if value.contains("adf") || value.contains("feed") {
        Some(CapabilitySource::AdfSimplex)
    } else if value.contains("film") || value.contains("transparen") {
        Some(CapabilitySource::Film)
    } else {
        None
    }
}

const MAX_XML_DEPTH: usize = 32;

/// Text of a predefined entity or character reference such as `&amp;`.
pub(crate) fn general_ref_text(event: &quick_xml::events::BytesRef) -> Option<String> {
    if let Ok(Some(character)) = event.resolve_char_ref() {
        return Some(character.to_string());
    }
    let text = match event.decode().ok()?.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => return None,
    };
    Some(text.to_string())
}

pub(crate) fn parse_escl_capabilities(body: &[u8], root: &str) -> Option<ScannerCapabilities> {
    // Text is not trimmed by the reader, which would eat the spaces around an
    // entity (`HP &amp; Co`); values are trimmed where they are used.
    let mut reader = Reader::from_reader(body);
    let mut buffer = Vec::new();
    let mut parser = CapabilityParser::new(root);

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => {
                if !parser.start(local_name(event.name().as_ref())) {
                    return None;
                }
            }
            Ok(Event::Text(event)) => parser.push_text(&event.decode().ok()?),
            Ok(Event::GeneralRef(event)) => parser.push_text(&general_ref_text(&event)?),
            Ok(Event::CData(event)) => parser.push_text(&String::from_utf8_lossy(event.as_ref())),
            Ok(Event::End(event)) => parser.end(local_name(event.name().as_ref())),
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buffer.clear();
    }
    parser.saw_capabilities.then_some(parser.capabilities)
}

pub(crate) fn capabilities_for_endpoint_with_cancellation(
    endpoint: &Endpoint,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
) -> Option<ScannerCapabilities> {
    capabilities_for_endpoint_with_cancellation_result(endpoint, deadline, cancellation)
        .ok()
        .flatten()
}

pub(crate) fn capabilities_for_endpoint_with_cancellation_result(
    endpoint: &Endpoint,
    deadline: Instant,
    cancellation: Option<CancellationToken>,
) -> std::result::Result<Option<ScannerCapabilities>, HttpExchangeError> {
    for (path, root) in [
        ("/eSCL/ScannerCapabilities", "eSCL"),
        ("/Scan/ScannerCapabilities", "Scan"),
    ] {
        if cancellation
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(HttpExchangeError::Cancelled);
        }
        let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        if timeout.is_zero() {
            return Ok(None);
        }
        let response = http_exchange_with_cancellation_result(
            endpoint,
            "GET",
            path,
            HttpExchangeOptions {
                body: None,
                content_type: None,
                global_timeout: timeout,
                setup_timeout: timeout.min(CONTROL_REQUEST_SETUP_TIMEOUT),
                response_header_timeout: timeout,
                response_limit: CAPABILITIES_RESPONSE_LIMIT,
                cancellation: cancellation.clone(),
            },
        );
        let (status, body, _) = match response {
            Ok(response) => response,
            Err(HttpExchangeError::Cancelled) => return Err(HttpExchangeError::Cancelled),
            Err(HttpExchangeError::Failed) => continue,
            Err(error @ HttpExchangeError::CertificatePinMismatch(_)) => return Err(error),
        };
        if status == 200 {
            if let Some(capabilities) = parse_escl_capabilities(&body, root) {
                return Ok(Some(capabilities));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::super::session::select_resolution;
    use super::*;

    const DISCRETE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScannerCapabilities xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
  <pwg:Version>2.63</pwg:Version>
  <pwg:MakeAndModel>Test Scanner</pwg:MakeAndModel>
  <scan:Platen>
    <scan:PlatenInputCaps>
      <scan:MaxWidth>2550</scan:MaxWidth>
      <scan:MaxHeight>3507</scan:MaxHeight>
      <scan:SettingProfiles>
        <scan:SettingProfile>
          <scan:ColorModes>
            <scan:ColorMode>RGB24</scan:ColorMode>
          </scan:ColorModes>
          <scan:DocumentFormats>
            <pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>
          </scan:DocumentFormats>
          <scan:SupportedResolutions>
            <scan:DiscreteResolutions>
              <scan:DiscreteResolution>
                <scan:XResolution>200</scan:XResolution>
                <scan:YResolution>200</scan:YResolution>
              </scan:DiscreteResolution>
              <scan:DiscreteResolution>
                <scan:XResolution>600</scan:XResolution>
                <scan:YResolution>600</scan:YResolution>
              </scan:DiscreteResolution>
            </scan:DiscreteResolutions>
          </scan:SupportedResolutions>
        </scan:SettingProfile>
      </scan:SettingProfiles>
    </scan:PlatenInputCaps>
  </scan:Platen>
</scan:ScannerCapabilities>"#;

    const RANGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<scan:ScannerCapabilities xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
  <scan:Platen>
    <scan:PlatenInputCaps>
      <scan:SettingProfiles>
        <scan:SettingProfile>
          <scan:SupportedResolutions>
            <scan:ResolutionRange>
              <scan:XResolutionRange>
                <scan:Min>75</scan:Min>
                <scan:Max>1200</scan:Max>
                <scan:Normal>300</scan:Normal>
                <scan:Step>25</scan:Step>
              </scan:XResolutionRange>
              <scan:YResolutionRange>
                <scan:Min>100</scan:Min>
                <scan:Max>600</scan:Max>
                <scan:Normal>300</scan:Normal>
                <scan:Step>50</scan:Step>
              </scan:YResolutionRange>
            </scan:ResolutionRange>
          </scan:SupportedResolutions>
        </scan:SettingProfile>
      </scan:SettingProfiles>
    </scan:PlatenInputCaps>
  </scan:Platen>
</scan:ScannerCapabilities>"#;

    fn platen(xml: &str) -> SourceCapabilities {
        parse_escl_capabilities(xml.as_bytes(), "eSCL")
            .unwrap()
            .source(CapabilitySource::Platen)
            .unwrap()
            .clone()
    }

    fn request(dpi: u32) -> ScanRequest {
        ScanRequest {
            dpi_x: dpi,
            dpi_y: dpi,
            ..ScanRequest::default()
        }
    }

    #[test]
    fn deeply_nested_xml_is_rejected_quickly() {
        let xml = "<a>".repeat(100_000);
        let started = Instant::now();
        assert!(parse_escl_capabilities(xml.as_bytes(), "eSCL").is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn entities_in_text_are_decoded() {
        let xml = r#"<scan:ScannerCapabilities xmlns:scan="x" xmlns:pwg="y">
  <pwg:MakeAndModel>HP &amp; Co Scanner &#x31; &lt;</pwg:MakeAndModel>
</scan:ScannerCapabilities>"#;
        let capabilities = parse_escl_capabilities(xml.as_bytes(), "eSCL").unwrap();
        assert_eq!(
            capabilities.make_and_model.as_deref(),
            Some("HP & Co Scanner 1 <")
        );
    }

    #[test]
    fn zero_resolution_pairs_are_skipped() {
        let xml = DISCRETE.replace("<scan:XResolution>200", "<scan:XResolution>0");
        assert_eq!(platen(&xml).resolutions, vec![(600, 600)]);
    }

    #[test]
    fn parses_discrete_resolutions() {
        let source = platen(DISCRETE);
        assert_eq!(source.resolutions, vec![(200, 200), (600, 600)]);
        assert!(source.resolution_ranges.is_empty());
    }

    #[test]
    fn parses_resolution_range() {
        let source = platen(RANGE);
        assert!(source.resolutions.is_empty());
        assert_eq!(
            source.resolution_ranges,
            vec![ResolutionRange {
                x: AxisRange {
                    min: 75,
                    max: 1200,
                    step: 25
                },
                y: AxisRange {
                    min: 100,
                    max: 600,
                    step: 50
                },
            }]
        );
    }

    #[test]
    fn discrete_selection_picks_nearest_advertised_pair() {
        let source = platen(DISCRETE);
        assert_eq!(select_resolution(&request(300), &source), (200, 200));
        assert_eq!(select_resolution(&request(450), &source), (600, 600));
        assert_eq!(select_resolution(&request(1200), &source), (600, 600));
    }

    #[test]
    fn range_selection_clamps_and_snaps_to_step() {
        let source = platen(RANGE);
        assert_eq!(select_resolution(&request(300), &source), (300, 300));
        assert_eq!(select_resolution(&request(310), &source), (300, 300));
        assert_eq!(select_resolution(&request(10), &source), (75, 100));
        assert_eq!(select_resolution(&request(2400), &source), (1200, 600));
    }

    #[test]
    fn missing_resolutions_keep_the_requested_value() {
        let source = SourceCapabilities::default();
        assert_eq!(select_resolution(&request(300), &source), (300, 300));
    }
}
