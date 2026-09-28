//! RTen-backed OCRS execution behind injectable runner and engine boundaries.

use crate::domain::image::ImageBuffer;
use crate::error::{Result, ScanError};
use crate::infrastructure::media::ocr::model_pack;
use crate::operation::CancellationToken;
#[cfg(feature = "ocrs")]
use std::borrow::Cow;
use std::path::Path;

#[cfg(feature = "ocrs")]
use crate::infrastructure::media::to_rgb_bytes;
#[cfg(feature = "ocrs")]
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
#[cfg(feature = "ocrs")]
use rten::Model;
#[cfg(feature = "ocrs")]
use std::sync::{Arc, Mutex};

pub const OCRS_ENGINE: &str = "ocrs";

/// Public injection seam for callers that provide an OCRS-compatible runner.
pub trait OcrsRunner: Send + Sync {
    fn recognize(
        &self,
        image: &ImageBuffer,
        detection: &[u8],
        recognition: &[u8],
    ) -> Result<String>;
}

#[derive(Debug, Default)]
pub struct LocalOcrsRunner;

impl OcrsRunner for LocalOcrsRunner {
    fn recognize(
        &self,
        image: &ImageBuffer,
        detection: &[u8],
        recognition: &[u8],
    ) -> Result<String> {
        recognize_once(image, detection, recognition)
    }
}

#[cfg(feature = "ocrs")]
fn recognize_once(image: &ImageBuffer, detection: &[u8], recognition: &[u8]) -> Result<String> {
    ProductionFactory
        .construct(detection.to_vec(), recognition.to_vec())?
        .recognize(image)
}

#[cfg(not(feature = "ocrs"))]
fn recognize_once(_: &ImageBuffer, _: &[u8], _: &[u8]) -> Result<String> {
    Err(not_compiled())
}

pub fn validate_model_files(detection: &Path, recognition: &Path) -> Result<()> {
    #[cfg(not(feature = "ocrs"))]
    {
        let _ = (detection, recognition);
        Err(not_compiled())
    }
    #[cfg(feature = "ocrs")]
    {
        let detection = model_pack::read_model_source_bytes(detection, "OCRS detection model")?;
        let recognition =
            model_pack::read_model_source_bytes(recognition, "OCRS recognition model")?;
        ProductionFactory
            .construct(detection, recognition)
            .map(|_| ())
    }
}

pub fn recognize_with_runner(
    image: &ImageBuffer,
    language: &str,
    cancellation: Option<&CancellationToken>,
    runner: &dyn OcrsRunner,
) -> Result<crate::infrastructure::media::ocr::OcrResult> {
    require_compiled()?;
    validate_language(language)?;
    check_cancellation(cancellation)?;
    let pair = model_pack::active_verified_model_pair(cancellation)?.ok_or_else(not_installed)?;
    let text = runner.recognize(image, &pair.detection, &pair.recognition)?;
    finish_result(text, cancellation)
}

#[cfg(all(test, feature = "ocrs"))]
fn recognize_pack_with_runner(
    image: &ImageBuffer,
    cancellation: Option<&CancellationToken>,
    pack: &model_pack::InstalledModelPack,
    runner: &dyn OcrsRunner,
) -> Result<crate::infrastructure::media::ocr::OcrResult> {
    require_compiled()?;
    check_cancellation(cancellation)?;
    let (detection, recognition) = model_pack::verified_model_bytes(pack)?;
    let text = runner.recognize(image, &detection, &recognition)?;
    finish_result(text, cancellation)
}

pub fn recognize(
    image: &ImageBuffer,
    language: &str,
    cancellation: Option<&CancellationToken>,
) -> Result<crate::infrastructure::media::ocr::OcrResult> {
    recognize_with_runner(image, language, cancellation, &LocalOcrsRunner)
}

fn finish_result(
    text: String,
    cancellation: Option<&CancellationToken>,
) -> Result<crate::infrastructure::media::ocr::OcrResult> {
    check_cancellation(cancellation)?;
    Ok(crate::infrastructure::media::ocr::OcrResult {
        text: normalized_text(text),
        engine: OCRS_ENGINE.into(),
        confidence: 0.0,
        language: "eng".into(),
    })
}

fn normalized_text(text: String) -> String {
    if text.trim().is_empty() {
        "[no text recognized]".into()
    } else {
        text.trim().into()
    }
}

fn validate_language(language: &str) -> Result<()> {
    if language == "eng" {
        Ok(())
    } else {
        Err(ScanError::Unsupported(
            "OCRS model packs support only printed Latin `eng`".into(),
        ))
    }
}

fn require_compiled() -> Result<()> {
    if cfg!(feature = "ocrs") {
        Ok(())
    } else {
        Err(not_compiled())
    }
}

fn not_compiled() -> ScanError {
    ScanError::Unsupported("OCRS support was not compiled into this build".into())
}

fn not_installed() -> ScanError {
    ScanError::Unsupported(
        "OCRS model pack is not installed; run `open-scanline ocr-model install --detection PATH --recognition PATH`".into(),
    )
}

fn check_cancellation(cancellation: Option<&CancellationToken>) -> Result<()> {
    crate::operation::check_cancellation(cancellation, "OCRS OCR cancelled")
}

/// Export-scoped lazy OCRS engine cache. Every page verifies fresh selected
/// model bytes before a matching cached engine may be reused.
pub(crate) struct OcrsJob {
    #[cfg(feature = "ocrs")]
    state: EnabledJob,
}

impl OcrsJob {
    pub(crate) fn new() -> Self {
        Self {
            #[cfg(feature = "ocrs")]
            state: EnabledJob::production(),
        }
    }

    pub(crate) fn recognize(
        &self,
        image: &ImageBuffer,
        language: &str,
        cancellation: Option<&CancellationToken>,
    ) -> Result<String> {
        require_compiled()?;
        validate_language(language)?;
        #[cfg(feature = "ocrs")]
        {
            self.state.recognize(image, cancellation)
        }
        #[cfg(not(feature = "ocrs"))]
        {
            let _ = (image, cancellation);
            Err(not_compiled())
        }
    }
}

#[cfg(feature = "ocrs")]
trait EngineRuntime: Send {
    fn recognize(&self, image: &ImageBuffer) -> Result<String>;
}

#[cfg(feature = "ocrs")]
trait EngineFactory: Send + Sync {
    fn construct(&self, detection: Vec<u8>, recognition: Vec<u8>)
        -> Result<Box<dyn EngineRuntime>>;
}

#[cfg(feature = "ocrs")]
struct ProductionFactory;

#[cfg(feature = "ocrs")]
impl EngineFactory for ProductionFactory {
    fn construct(
        &self,
        detection: Vec<u8>,
        recognition: Vec<u8>,
    ) -> Result<Box<dyn EngineRuntime>> {
        let detection_model = Model::load(detection).map_err(|error| {
            ScanError::Invalid(format!("OCRS detection model is invalid: {error}"))
        })?;
        let recognition_model = Model::load(recognition).map_err(|error| {
            ScanError::Invalid(format!("OCRS recognition model is invalid: {error}"))
        })?;
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(detection_model),
            recognition_model: Some(recognition_model),
            ..Default::default()
        })
        .map_err(|error| ScanError::Invalid(format!("OCRS model pair is incompatible: {error}")))?;
        Ok(Box::new(ProductionEngine(engine)))
    }
}

#[cfg(feature = "ocrs")]
struct ProductionEngine(OcrEngine);

#[cfg(feature = "ocrs")]
impl EngineRuntime for ProductionEngine {
    fn recognize(&self, image: &ImageBuffer) -> Result<String> {
        let rgb = match image.pixel_format {
            crate::domain::image::PixelFormat::Rgb8 => Cow::Borrowed(image.data.as_slice()),
            _ => Cow::Owned(to_rgb_bytes(image)?),
        };
        let source = ImageSource::from_bytes(&rgb, (image.width, image.height))
            .map_err(|error| ScanError::Invalid(format!("OCRS image input is invalid: {error}")))?;
        let input = self
            .0
            .prepare_input(source)
            .map_err(|error| ScanError::Other(format!("OCRS input preparation failed: {error}")))?;
        self.0
            .get_text(&input)
            .map_err(|error| ScanError::Other(format!("OCRS recognition failed: {error}")))
    }
}

#[cfg(feature = "ocrs")]
struct CachedEngine {
    key: String,
    engine: Box<dyn EngineRuntime>,
}

#[cfg(feature = "ocrs")]
struct EnabledJob {
    loader: Arc<dyn ModelLoader>,
    factory: Arc<dyn EngineFactory>,
    cache: Mutex<Option<CachedEngine>>,
}

#[cfg(feature = "ocrs")]
impl EnabledJob {
    fn production() -> Self {
        Self {
            loader: Arc::new(ProductionLoader),
            factory: Arc::new(ProductionFactory),
            cache: Mutex::new(None),
        }
    }

    fn recognize(
        &self,
        image: &ImageBuffer,
        cancellation: Option<&CancellationToken>,
    ) -> Result<String> {
        check_cancellation(cancellation)?;
        let pair = self.loader.load(cancellation)?.ok_or_else(not_installed)?;
        self.recognize_pair(image, pair, cancellation)
    }

    fn recognize_pair(
        &self,
        image: &ImageBuffer,
        pair: model_pack::VerifiedModelPair,
        cancellation: Option<&CancellationToken>,
    ) -> Result<String> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| ScanError::Other("OCRS engine cache lock was poisoned".into()))?;
        self.prepare_engine(&mut cache, pair, cancellation)?;
        check_cancellation(cancellation)?;
        let text = cache
            .as_ref()
            .expect("OCRS cache initialized")
            .engine
            .recognize(image)?;
        check_cancellation(cancellation)?;
        Ok(normalized_text(text))
    }

    fn prepare_engine(
        &self,
        cache: &mut Option<CachedEngine>,
        pair: model_pack::VerifiedModelPair,
        cancellation: Option<&CancellationToken>,
    ) -> Result<()> {
        if cache.as_ref().is_some_and(|cached| cached.key == pair.key) {
            return Ok(());
        }
        check_cancellation(cancellation)?;
        drop(cache.take());
        let engine = self.factory.construct(pair.detection, pair.recognition)?;
        check_cancellation(cancellation)?;
        *cache = Some(CachedEngine {
            key: pair.key,
            engine,
        });
        Ok(())
    }
}

#[cfg(feature = "ocrs")]
trait ModelLoader: Send + Sync {
    fn load(
        &self,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<model_pack::VerifiedModelPair>>;
}

#[cfg(feature = "ocrs")]
struct ProductionLoader;

#[cfg(feature = "ocrs")]
impl ModelLoader for ProductionLoader {
    fn load(
        &self,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Option<model_pack::VerifiedModelPair>> {
        model_pack::active_verified_model_pair(cancellation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::image::PixelFormat;

    fn sample() -> ImageBuffer {
        ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![255; 3]).unwrap()
    }

    #[cfg(not(feature = "ocrs"))]
    #[test]
    fn disabled_runner_rejects_before_paths_or_inference() {
        struct PanicRunner;
        impl OcrsRunner for PanicRunner {
            fn recognize(&self, _: &ImageBuffer, _: &[u8], _: &[u8]) -> Result<String> {
                panic!("runner must not execute")
            }
        }
        let error = recognize_with_runner(&sample(), "eng", None, &PanicRunner).unwrap_err();
        assert!(
            matches!(error, ScanError::Unsupported(message) if message.contains("not compiled"))
        );
    }
}
