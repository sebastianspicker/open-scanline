use super::session::{
    region_in_three_hundredths, select_color_mode, select_document_format, select_resolution,
    select_source,
};
use super::{CapabilitySource, DocumentRepresentation, ScannerCapabilities};
use crate::domain::acquisition::{validate_scan_dpi, ScanRequest};
use crate::error::Result;

pub(crate) fn build_settings(
    request: &ScanRequest,
    capabilities: &ScannerCapabilities,
) -> Result<(Vec<u8>, DocumentRepresentation)> {
    validate_scan_dpi(request.dpi_x, request.dpi_y)?;
    let source = select_source(request, capabilities)?;
    let color = select_color_mode(request, &source)?;
    let document_format = select_document_format(&source)?;
    let (dpi_x, dpi_y) = select_resolution(request, &source);
    let region = region_in_three_hundredths(request, request.dpi_x, request.dpi_y, &source);
    let settings = render_settings(&SettingsValues {
        region,
        source: source.source.expect("selected source"),
        dpi_x,
        dpi_y,
        color: &color.name,
        document_format: &document_format,
    });
    Ok((settings.into_bytes(), color.representation))
}

struct SettingsValues<'a> {
    region: (u32, u32, u32, u32),
    source: CapabilitySource,
    dpi_x: u32,
    dpi_y: u32,
    color: &'a str,
    document_format: &'a str,
}

fn render_settings(values: &SettingsValues<'_>) -> String {
    let (x, y, width, height) = values.region;
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<scan:ScanSettings xmlns:scan=\"http://schemas.hp.com/imaging/escl/2011/05/03\" xmlns:pwg=\"http://www.pwg.org/schemas/2010/12/sm\" xmlns:escl=\"http://schemas.hp.com/imaging/escl/2011/05/03\">\n",
            "  <pwg:Version>2.0</pwg:Version>\n  <scan:Intent>Document</scan:Intent>\n",
            "  <pwg:ScanRegions>\n    <pwg:ScanRegion>\n      <pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>\n",
            "      <pwg:XOffset>{}</pwg:XOffset>\n      <pwg:YOffset>{}</pwg:YOffset>\n      <pwg:Width>{}</pwg:Width>\n      <pwg:Height>{}</pwg:Height>\n",
            "    </pwg:ScanRegion>\n  </pwg:ScanRegions>\n  <pwg:InputSource>{}</pwg:InputSource>\n{}  <scan:XResolution>{}</scan:XResolution>\n",
            "  <scan:YResolution>{}</scan:YResolution>\n  <scan:ColorMode>{}</scan:ColorMode>\n  <pwg:DocumentFormat>{}</pwg:DocumentFormat>\n</scan:ScanSettings>\n",
        ),
        x,
        y,
        width,
        height,
        values.source.input_source(),
        duplex_setting(values.source),
        values.dpi_x,
        values.dpi_y,
        values.color,
        values.document_format,
    )
}

fn duplex_setting(source: CapabilitySource) -> &'static str {
    match source {
        CapabilitySource::AdfSimplex => "  <scan:Duplex>false</scan:Duplex>\n",
        CapabilitySource::AdfDuplex => "  <scan:Duplex>true</scan:Duplex>\n",
        CapabilitySource::Platen | CapabilitySource::Film => "",
    }
}
