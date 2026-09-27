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

struct CapabilityParser {
    capabilities: ScannerCapabilities,
    path: Vec<String>,
    values: Vec<String>,
    current_resolution: Option<(Option<u32>, Option<u32>)>,
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
            saw_capabilities: false,
        }
    }

    fn start(&mut self, name: String) {
        self.saw_capabilities |= name == "scannercapabilities";
        if name == "resolution" {
            self.current_resolution = Some((None, None));
        }
        self.path.push(name);
        self.values.push(String::new());
    }

    fn set_value(&mut self, value: String) {
        if let Some(current) = self.values.last_mut() {
            *current = value;
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
            "resolution" => self.finish_resolution(source),
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

    fn finish_resolution(&mut self, source: Option<CapabilitySource>) {
        if let Some((Some(x), Some(y))) = self.current_resolution.take() {
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

pub(crate) fn parse_escl_capabilities(body: &[u8], root: &str) -> Option<ScannerCapabilities> {
    let mut reader = Reader::from_reader(body);
    reader.config_mut().trim_text(true);
    let mut buffer = Vec::new();
    let mut parser = CapabilityParser::new(root);

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(event)) => parser.start(local_name(event.name().as_ref())),
            Ok(Event::Text(event)) => parser.set_value(event.decode().ok()?.into_owned()),
            Ok(Event::CData(event)) => {
                parser.set_value(String::from_utf8_lossy(event.as_ref()).into_owned())
            }
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
        };
        if status == 200 {
            if let Some(capabilities) = parse_escl_capabilities(&body, root) {
                return Ok(Some(capabilities));
            }
        }
    }
    Ok(None)
}
