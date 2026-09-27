use crate::domain::acquisition::ScanRequest;
use crate::domain::image::ImageBuffer;
use crate::error::Result;
use crate::workflows::ports::acquisition::ScanPagesResult;

pub(crate) fn emit_simulated_pages(
    request: &ScanRequest,
    maximum: u32,
    emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
) -> Result<ScanPagesResult> {
    emit_single_pages(request, maximum, emit, |page| {
        super::super::mock::MockDeviceSession::gradient(page)
    })
}

pub(crate) fn emit_single_pages(
    request: &ScanRequest,
    maximum: u32,
    emit: &mut dyn FnMut(ImageBuffer) -> Result<()>,
    mut acquire: impl FnMut(&ScanRequest) -> Result<ImageBuffer>,
) -> Result<ScanPagesResult> {
    for index in 0..maximum {
        let mut page = request.clone();
        page.duplex = false;
        page.seed = request.seed.saturating_add(index);
        emit(acquire(&page)?)?;
    }
    Ok(ScanPagesResult::limit_reached(maximum))
}
