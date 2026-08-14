use super::*;
use crate::device::DeviceSession;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

static ESCL_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn drain_request_body(stream: &mut TcpStream, request: &str) {
    let Some(header_end) = request.find("\r\n\r\n") else {
        return;
    };
    let content_length = request
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let buffered = request.len().saturating_sub(header_end + 4);
    let remaining = content_length.saturating_sub(buffered);
    if remaining != 0 {
        let mut body = vec![0_u8; remaining];
        stream.read_exact(&mut body).unwrap();
    }
}

fn write_chunked(stream: &mut TcpStream, status: &str, extra_headers: &str, body: &[u8]) {
    let split = body.len().min(7);
    let mut response = format!(
        "HTTP/1.1 {status}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n{extra_headers}\r\n"
    )
    .into_bytes();
    for chunk in [&body[..split], &body[split..]] {
        if !chunk.is_empty() {
            response.extend_from_slice(format!("{:X}\r\n", chunk.len()).as_bytes());
            response.extend_from_slice(chunk);
            response.extend_from_slice(b"\r\n");
        }
    }
    response.extend_from_slice(b"0\r\n\r\n");
    stream.write_all(&response).unwrap();
    stream.flush().unwrap();
}

fn fixture_png() -> Vec<u8> {
    fixture_png_pixel([12, 34, 56])
}

fn fixture_png_pixel(pixel: [u8; 3]) -> Vec<u8> {
    let path = std::env::temp_dir().join(format!(
        "open_scanline_escl_fixture_{}_{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, pixel.to_vec()).unwrap();
    crate::imaging::save_image(&path, &image, Some(150), None).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    bytes
}

fn fixture_tiff() -> Vec<u8> {
    let path = std::env::temp_dir().join(format!(
        "open_scanline_escl_fixture_{}_{}.tiff",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let image = ImageBuffer::new(1, 1, PixelFormat::Rgb8, vec![12, 34, 56]).unwrap();
    crate::imaging::save_image(&path, &image, Some(150), None).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(path).unwrap();
    bytes
}

const FULL_CAPABILITIES: &[u8] = br#"
<scan:ScannerCapabilities xmlns:scan="http://schemas.hp.com/imaging/escl/2011/05/03" xmlns:pwg="http://www.pwg.org/schemas/2010/12/sm">
  <pwg:MakeAndModel>Acme Scan Pro</pwg:MakeAndModel>
  <scan:Platen><scan:PlatenInputCaps>
    <scan:MaxWidth>2550</scan:MaxWidth><scan:MaxHeight>3300</scan:MaxHeight>
    <scan:ColorModes><scan:ColorMode>RGB24</scan:ColorMode></scan:ColorModes>
    <scan:DocumentFormats><pwg:DocumentFormat>image/png</pwg:DocumentFormat></scan:DocumentFormats>
    <scan:DiscreteResolutions><scan:Resolution><scan:XResolution>300</scan:XResolution><scan:YResolution>600</scan:YResolution></scan:Resolution></scan:DiscreteResolutions>
  </scan:PlatenInputCaps></scan:Platen>
  <scan:AdfSimplexInputCaps><scan:ColorMode>Grayscale8</scan:ColorMode><pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat></scan:AdfSimplexInputCaps>
  <scan:AdfDuplexInputCaps><scan:ColorMode>RGB24</scan:ColorMode><pwg:DocumentFormat>image/tiff</pwg:DocumentFormat></scan:AdfDuplexInputCaps>
</scan:ScannerCapabilities>"#;

#[test]
fn capabilities_parser_and_settings_negotiate_sources_formats_and_units() {
    let capabilities = parse_escl_capabilities(FULL_CAPABILITIES, "eSCL").unwrap();
    assert_eq!(
        capabilities.make_and_model.as_deref(),
        Some("Acme Scan Pro")
    );
    assert_eq!(capabilities.root, "eSCL");
    assert!(capabilities.source(CapabilitySource::Platen).is_some());
    assert!(capabilities.source(CapabilitySource::AdfSimplex).is_some());
    assert!(capabilities.source(CapabilitySource::AdfDuplex).is_some());

    let session = EsclDeviceSession::new(
        "escl:test".into(),
        Endpoint {
            host: "test".into(),
            port: 80,
            secure: false,
        },
        false,
    );
    let settings = String::from_utf8(
        session
            .build_settings(
                &ScanRequest {
                    dpi_x: 300,
                    dpi_y: 600,
                    width: 600,
                    height: 1200,
                    region: Some(crate::core::Rect::new(300, 600, 600, 1200)),
                    pixel_format: PixelFormat::Rgb8,
                    ..Default::default()
                },
                &capabilities,
            )
            .unwrap()
            .0,
    )
    .unwrap();
    assert!(settings.contains("escl:ThreeHundredthsOfInches"));
    assert!(settings.contains("<pwg:XOffset>300</pwg:XOffset>"));
    assert!(settings.contains("<pwg:YOffset>300</pwg:YOffset>"));
    assert!(settings.contains("<pwg:Width>600</pwg:Width>"));
    assert!(settings.contains("<pwg:Height>600</pwg:Height>"));
    assert!(settings.contains("<scan:XResolution>300</scan:XResolution>"));
    assert!(settings.contains("<scan:YResolution>600</scan:YResolution>"));
    assert!(settings.contains("<pwg:ScanRegions>"));
    assert!(settings.contains("<pwg:InputSource>Platen</pwg:InputSource>"));
    assert!(settings.contains("<pwg:DocumentFormat>image/png</pwg:DocumentFormat>"));
}

#[test]
fn settings_uses_requested_dpi_for_physical_region_and_selected_dpi_for_acquisition() {
    let session = EsclDeviceSession::new(
        "escl:test".into(),
        Endpoint {
            host: "test".into(),
            port: 80,
            secure: false,
        },
        false,
    );
    let source = SourceCapabilities {
        source: Some(CapabilitySource::Platen),
        color_modes: vec!["RGB24".into()],
        document_formats: vec!["image/png".into()],
        resolutions: vec![(300, 300)],
        max_width: Some(2_550),
        max_height: Some(3_300),
    };
    let capabilities = ScannerCapabilities {
        root: "eSCL".into(),
        sources: vec![source],
        ..ScannerCapabilities::default()
    };
    let settings = String::from_utf8(
        session
            .build_settings(
                &ScanRequest {
                    dpi_x: 150,
                    dpi_y: 200,
                    region: Some(crate::core::Rect::new(150, 100, 600, 600)),
                    pixel_format: PixelFormat::Rgb8,
                    ..Default::default()
                },
                &capabilities,
            )
            .unwrap()
            .0,
    )
    .unwrap();

    assert!(settings.contains("<pwg:XOffset>300</pwg:XOffset>"));
    assert!(settings.contains("<pwg:YOffset>150</pwg:YOffset>"));
    assert!(settings.contains("<pwg:Width>1200</pwg:Width>"));
    assert!(settings.contains("<pwg:Height>900</pwg:Height>"));
    assert!(settings.contains("<scan:XResolution>300</scan:XResolution>"));
    assert!(settings.contains("<scan:YResolution>300</scan:YResolution>"));
}

#[test]
fn physical_region_clamps_to_source_bounds_after_requested_dpi_conversion() {
    let source = SourceCapabilities {
        max_width: Some(1_200),
        max_height: Some(200),
        ..SourceCapabilities::default()
    };
    let request = ScanRequest {
        dpi_x: 150,
        dpi_y: 200,
        region: Some(crate::core::Rect::new(500, 50, 600, 100)),
        ..Default::default()
    };

    assert_eq!(
        region_in_three_hundredths(&request, request.dpi_x, request.dpi_y, &source),
        (1_000, 75, 200, 125)
    );
}

#[test]
fn settings_reject_requested_dpi_below_escl_minimum() {
    let capabilities = parse_escl_capabilities(FULL_CAPABILITIES, "eSCL").unwrap();
    let session = EsclDeviceSession::new(
        "escl:test".into(),
        Endpoint {
            host: "test".into(),
            port: 80,
            secure: false,
        },
        false,
    );

    let error = session
        .build_settings(
            &ScanRequest {
                dpi_x: crate::core::MIN_SCAN_DPI - 1,
                ..Default::default()
            },
            &capabilities,
        )
        .unwrap_err();
    assert!(matches!(
        error,
        ScanError::Invalid(message) if message.contains("at least 50 dpi")
    ));
}

#[test]
fn settings_selects_the_nearest_advertised_resolution_for_valid_requests() {
    let source = SourceCapabilities {
        resolutions: vec![(1_200, 1_200), (2_400, 1_200)],
        ..SourceCapabilities::default()
    };
    let request = ScanRequest {
        dpi_x: 2_400,
        dpi_y: 1_200,
        ..Default::default()
    };

    assert_eq!(select_resolution(&request, &source), (2_400, 1_200));
}

#[test]
fn adf_mode_and_duplex_require_advertised_sources() {
    let capabilities = parse_escl_capabilities(FULL_CAPABILITIES, "eSCL").unwrap();
    let simplex = select_source(
        &ScanRequest {
            mode: ScanMode::Document,
            ..Default::default()
        },
        &capabilities,
    )
    .unwrap();
    assert_eq!(simplex.source, Some(CapabilitySource::AdfSimplex));
    let duplex = select_source(
        &ScanRequest {
            mode: ScanMode::Document,
            duplex: true,
            ..Default::default()
        },
        &capabilities,
    )
    .unwrap();
    assert_eq!(duplex.source, Some(CapabilitySource::AdfDuplex));
    assert!(select_source(
        &ScanRequest {
            mode: ScanMode::Film,
            ..Default::default()
        },
        &capabilities,
    )
    .is_err());
    assert!(select_source(
        &ScanRequest {
            duplex: true,
            ..Default::default()
        },
        &capabilities,
    )
    .is_err());
}

#[test]
fn adf_settings_explicitly_distinguish_simplex_from_duplex() {
    let capabilities = ScannerCapabilities {
        root: "eSCL".into(),
        sources: [CapabilitySource::AdfSimplex, CapabilitySource::AdfDuplex]
            .into_iter()
            .map(|source| SourceCapabilities {
                source: Some(source),
                color_modes: vec!["RGB24".into()],
                document_formats: vec!["image/png".into()],
                resolutions: vec![(300, 300)],
                ..SourceCapabilities::default()
            })
            .collect(),
        ..ScannerCapabilities::default()
    };
    let session = EsclDeviceSession::new(
        "escl:test".into(),
        Endpoint {
            host: "test".into(),
            port: 80,
            secure: false,
        },
        false,
    );
    let settings_for = |duplex| {
        String::from_utf8(
            session
                .build_settings(
                    &ScanRequest {
                        mode: ScanMode::Document,
                        duplex,
                        ..Default::default()
                    },
                    &capabilities,
                )
                .unwrap()
                .0,
        )
        .unwrap()
    };

    let simplex = settings_for(false);
    assert!(simplex.contains("<pwg:InputSource>Feeder</pwg:InputSource>"));
    assert!(simplex.contains("<scan:Duplex>false</scan:Duplex>"));

    let duplex = settings_for(true);
    assert!(duplex.contains("<pwg:InputSource>Feeder</pwg:InputSource>"));
    assert!(duplex.contains("<scan:Duplex>true</scan:Duplex>"));
}

#[test]
fn tiff_magic_is_decoded_as_a_raster_not_a_pdf() {
    let bytes = fixture_tiff();
    assert!(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*"));
    let path = std::env::temp_dir().join(format!(
        "open_scanline_escl_tiff_{}.bin",
        std::process::id()
    ));
    std::fs::write(&path, &bytes).unwrap();
    let decode_path = document_path(&path).unwrap();
    assert_eq!(
        decode_path
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("tiff")
    );
    std::fs::remove_file(decode_path).unwrap();
}

#[test]
fn decoded_document_uses_region_dimensions_when_present() {
    let session = EsclDeviceSession::new(
        "escl:test".into(),
        Endpoint {
            host: "test".into(),
            port: 80,
            secure: false,
        },
        false,
    );
    let output = TemporaryOutput::new("escl-test", "bin").unwrap();
    std::fs::write(output.path(), fixture_png()).unwrap();
    let request = ScanRequest {
        width: 20,
        height: 30,
        region: Some(crate::core::Rect::new(5, 7, 2, 3)),
        ..Default::default()
    };
    let image = session
        .decode_document(
            &request,
            &output,
            DocumentLimits::for_request(&request, DocumentRepresentation::Rgb24).unwrap(),
        )
        .unwrap()
        .unwrap();
    assert_eq!((image.width, image.height), (2, 3));
}

#[test]
fn document_limits_allow_large_valid_scans_without_becoming_unbounded() {
    let request = ScanRequest {
        // Roughly US Letter at 600 DPI. The packed RGB result is about 96 MiB,
        // so this guards against regressing to the former 64 MiB eSCL cap.
        width: 5_100,
        height: 6_600,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    let response_limit = document_response_limit(&request, DocumentRepresentation::Rgb24).unwrap();
    let (max_width, max_height, max_allocation) =
        document_decode_limits(&request, DocumentRepresentation::Rgb24).unwrap();

    assert!(response_limit > 64 * 1024 * 1024);
    assert!(max_allocation > 64 * 1024 * 1024);
    assert!(max_width >= request.width);
    assert!(max_height >= request.height);
    assert!(response_limit <= MAX_DOCUMENT_RESPONSE_LIMIT);
    assert!(max_allocation <= crate::core::MAX_IMAGE_BYTES as u64);
    assert!(max_width <= crate::core::MAX_IMAGE_DIMENSION);
    assert!(max_height <= crate::core::MAX_IMAGE_DIMENSION);
}

#[test]
fn document_limits_reject_a_request_beyond_the_global_image_contract() {
    let request = ScanRequest {
        width: crate::core::MAX_IMAGE_DIMENSION,
        height: crate::core::MAX_IMAGE_DIMENSION,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    assert!(document_response_limit(&request, DocumentRepresentation::Rgb24).is_err());
    assert!(document_decode_limits(&request, DocumentRepresentation::Rgb24).is_err());
}

#[test]
fn rgb48_negotiation_uses_six_bytes_per_pixel_and_rejects_the_decode_ceiling() {
    let rgb48_only = SourceCapabilities {
        color_modes: vec!["RGB48".into()],
        ..SourceCapabilities::default()
    };
    let request = ScanRequest {
        width: 5_100,
        height: 6_600,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    let color = select_color_mode(&request, &rgb48_only).unwrap();
    assert_eq!(color.representation, DocumentRepresentation::Rgb48);

    let expected_allocation = u64::from(request.width) * u64::from(request.height) * 6;
    assert_eq!(
        document_response_limit(&request, color.representation).unwrap(),
        expected_allocation + DOCUMENT_CONTAINER_OVERHEAD
    );
    assert_eq!(
        document_decode_limits(&request, color.representation)
            .unwrap()
            .2,
        expected_allocation
    );

    let ceiling_request = ScanRequest {
        width: 10_000,
        height: 10_000,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    let error = DocumentLimits::for_request(&ceiling_request, color.representation).unwrap_err();
    assert!(matches!(
        error,
        ScanError::Invalid(message)
            if message.contains("RGB48") && message.contains("decoded image safety limit")
    ));
}

#[test]
fn rgb48_decode_ceiling_is_rejected_before_creating_a_job() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        write_chunked(
            &mut stream,
            "200 OK",
            "Content-Type: text/xml\r\n",
            b"<ScannerCapabilities><Platen><PlatenInputCaps><ColorMode>RGB48</ColorMode></PlatenInputCaps></Platen></ScannerCapabilities>",
        );
        request
    });
    let session = EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    );

    let error = session
        .scan(&ScanRequest {
            width: 10_000,
            height: 10_000,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        })
        .unwrap_err();
    assert!(matches!(
        error,
        ScanError::Invalid(message)
            if message.contains("RGB48") && message.contains("decoded image safety limit")
    ));
    assert!(server
        .join()
        .unwrap()
        .starts_with("GET /eSCL/ScannerCapabilities "));
}

#[test]
fn next_document_polls_until_the_scanner_is_ready() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let image = fixture_png();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            (
                "201 Created",
                "Location: /eSCL/ScanJobs/ready-later\r\n",
                b"<JobUri>/eSCL/ScanJobs/ready-later</JobUri>".as_slice(),
            ),
            ("503 Service Unavailable", "", b"".as_slice()),
            ("200 OK", "Content-Type: image/png\r\n", image.as_slice()),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
        }
        requests
    });
    let session = EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    );
    let started = Instant::now();
    assert_eq!(
        session
            .scan(&ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            })
            .unwrap()
            .data,
        vec![12, 34, 56]
    );
    assert!(started.elapsed() >= NEXT_DOCUMENT_RETRY_DELAY);
    let requests = server.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("NextDocument"))
            .count(),
        2
    );
}

#[test]
fn scan_pages_streams_one_canonical_job_until_feeder_exhaustion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let first = fixture_png_pixel([1, 2, 3]);
    let second = fixture_png_pixel([4, 5, 6]);
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            (
                "201 Created",
                "Location: /Scan/ScanJobs/canonical-job\r\n",
                b"".as_slice(),
            ),
            ("503 Service Unavailable", "", b"".as_slice()),
            ("200 OK", "Content-Type: image/png\r\n", first.as_slice()),
            ("202 Accepted", "", b"".as_slice()),
            ("200 OK", "Content-Type: image/png\r\n", second.as_slice()),
            ("204 No Content", "", b"".as_slice()),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
        }
        requests
    });
    let session = EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    );
    let mut emitted = Vec::new();
    let result = session
        .scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            3,
            &mut |image| {
                emitted.push(image.data);
                Ok(())
            },
        )
        .unwrap();

    assert_eq!(result, crate::device::ScanPagesResult::feeder_exhausted(2));
    assert_eq!(emitted, vec![vec![1, 2, 3], vec![4, 5, 6]]);
    let requests = server.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /"))
            .count(),
        1
    );
    assert!(requests[2].starts_with("GET /Scan/ScanJobs/canonical-job/NextDocument "));
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.contains("NextDocument"))
            .count(),
        5
    );
}

#[test]
fn scan_pages_limit_and_callback_error_delete_the_single_job() {
    for callback_fails in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let image = fixture_png();
        let server = thread::spawn(move || {
            let mut requests = Vec::new();
            for (status, headers, body) in [
                (
                    "200 OK",
                    "Content-Type: text/xml\r\n",
                    b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
                ),
                (
                    "201 Created",
                    "Location: /eSCL/ScanJobs/cleanup-job\r\n",
                    b"".as_slice(),
                ),
                ("200 OK", "Content-Type: image/png\r\n", image.as_slice()),
                ("200 OK", "", b"".as_slice()),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_request(&mut stream));
                write_chunked(&mut stream, status, headers, body);
            }
            requests
        });
        let session = EsclDeviceSession::new(
            format!("escl:127.0.0.1:{port}"),
            Endpoint {
                host: "127.0.0.1".into(),
                port,
                secure: false,
            },
            false,
        );
        let result = session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            1,
            &mut |_| {
                if callback_fails {
                    Err(ScanError::Other("consumer rejected page".into()))
                } else {
                    Ok(())
                }
            },
        );
        if callback_fails {
            assert!(result.is_err());
        } else {
            assert_eq!(
                result.unwrap(),
                crate::device::ScanPagesResult::limit_reached(1)
            );
        }
        let requests = server.join().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| request.starts_with("POST /"))
                .count(),
            1
        );
        assert!(requests[3].starts_with("DELETE /eSCL/ScanJobs/cleanup-job "));
    }
}

#[test]
fn scan_pages_cancellation_deletes_the_active_job() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (ready_tx, ready_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (index, (status, headers, body)) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            (
                "201 Created",
                "Location: /eSCL/ScanJobs/cancel-job\r\n",
                b"".as_slice(),
            ),
            ("503 Service Unavailable", "", b"".as_slice()),
            ("200 OK", "", b"".as_slice()),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
            if index == 2 {
                ready_tx.send(()).unwrap();
            }
        }
        requests
    });
    let session = Arc::new(EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    ));
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        scanning_session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            2,
            &mut |_| Ok(()),
        )
    });
    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    session.cancel();
    assert!(matches!(scan.join().unwrap(), Err(ScanError::Cancelled(_))));
    let requests = server.join().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /"))
            .count(),
        1
    );
    assert!(requests[3].starts_with("DELETE /eSCL/ScanJobs/cancel-job "));
}

#[test]
fn cancellation_interrupts_stalled_capabilities_without_trying_the_alternate_root() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        request_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        request
    });
    let session = Arc::new(EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    ));
    let cancellation = CancellationToken::new();
    session.bind_cancellation(cancellation.clone());
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        scanning_session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            1,
            &mut |_| Ok(()),
        )
    });

    request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    cancellation.cancel();
    assert!(matches!(scan.join().unwrap(), Err(ScanError::Cancelled(_))));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "header setup elapsed: {:?}",
        started.elapsed()
    );
    assert!(server
        .join()
        .unwrap()
        .starts_with("GET /eSCL/ScannerCapabilities "));
}

#[test]
fn cancellation_interrupts_stalled_create_job_without_trying_the_alternate_root() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut capabilities, _) = listener.accept().unwrap();
        assert!(read_request(&mut capabilities).starts_with("GET /eSCL/ScannerCapabilities "));
        write_chunked(
            &mut capabilities,
            "200 OK",
            "Content-Type: text/xml\r\n",
            b"<ScannerCapabilities><Platen/></ScannerCapabilities>",
        );

        let (mut create_job, _) = listener.accept().unwrap();
        let request = read_request(&mut create_job);
        drain_request_body(&mut create_job, &request);
        request_tx.send(()).unwrap();
        create_job
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(create_job.read(&mut byte).unwrap(), 0);
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        request
    });
    let session = Arc::new(EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    ));
    let cancellation = CancellationToken::new();
    session.bind_cancellation(cancellation.clone());
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        scanning_session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            1,
            &mut |_| Ok(()),
        )
    });

    request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    cancellation.cancel();
    assert!(matches!(scan.join().unwrap(), Err(ScanError::Cancelled(_))));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(server.join().unwrap().starts_with("POST /eSCL/ScanJobs "));
}

#[test]
fn cancellation_cleanup_delete_is_sent_and_bounded_when_its_headers_stall() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (delete_tx, delete_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            (
                "201 Created",
                "Location: /eSCL/ScanJobs/stalled-delete\r\n",
                b"".as_slice(),
            ),
            ("503 Service Unavailable", "", b"".as_slice()),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
        }
        ready_tx.send(()).unwrap();

        let (mut delete_stream, _) = listener.accept().unwrap();
        requests.push(read_request(&mut delete_stream));
        delete_tx.send(()).unwrap();
        delete_stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(delete_stream.read(&mut byte).unwrap(), 0);
        requests
    });
    let session = Arc::new(EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    ));
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        scanning_session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            2,
            &mut |_| Ok(()),
        )
    });

    ready_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    session.cancel();
    assert!(matches!(scan.join().unwrap(), Err(ScanError::Cancelled(_))));
    assert!(started.elapsed() < Duration::from_secs(1));
    delete_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("best-effort cancellation cleanup must send DELETE");
    let requests = server.join().unwrap();
    assert!(requests[3].starts_with("DELETE /eSCL/ScanJobs/stalled-delete "));
}

#[test]
fn stalled_next_document_cancellation_returns_promptly_cleans_output_and_deletes_job() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (body_started_tx, body_started_rx) = mpsc::channel();
    let (document_closed_tx, document_closed_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            (
                "201 Created",
                "Location: /eSCL/ScanJobs/stalled-job\r\n",
                b"".as_slice(),
            ),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
        }

        let (mut document_stream, _) = listener.accept().unwrap();
        requests.push(read_request(&mut document_stream));
        document_stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n1234")
            .unwrap();
        document_stream.flush().unwrap();
        body_started_tx.send(()).unwrap();

        document_stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(document_stream.read(&mut byte).unwrap(), 0);
        document_closed_tx.send(()).unwrap();

        let (mut delete_stream, _) = listener.accept().unwrap();
        requests.push(read_request(&mut delete_stream));
        write_chunked(&mut delete_stream, "200 OK", "", b"");
        drop(document_stream);
        requests
    });
    let session = Arc::new(EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    ));
    let cancellation = CancellationToken::new();
    session.bind_cancellation(cancellation.clone());
    let (result_tx, result_rx) = mpsc::channel();
    let output_directories_before = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".open-scanline-escl-"))
        })
        .collect::<std::collections::BTreeSet<_>>();
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        let result = scanning_session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            1,
            &mut |_| Ok(()),
        );
        result_tx.send(result).unwrap();
    });

    body_started_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
    let output_directory = {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(path) = std::fs::read_dir(std::env::temp_dir())
                .unwrap()
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .find(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy().starts_with(".open-scanline-escl-")
                    }) && !output_directories_before.contains(path)
                })
            {
                break path;
            }
            assert!(
                Instant::now() < deadline,
                "NextDocument did not materialize its temporary output"
            );
            thread::sleep(Duration::from_millis(10));
        }
    };
    let started = Instant::now();
    cancellation.cancel();
    let result = result_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("cancellation should interrupt the stalled body read");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(result, Err(ScanError::Cancelled(_))));
    scan.join().unwrap();
    document_closed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("cancellation must close the NextDocument transport before returning");

    let requests = server.join().unwrap();
    let output_directories_after = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(std::result::Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".open-scanline-escl-"))
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(!output_directory.exists());
    assert_eq!(output_directories_after, output_directories_before);
    assert!(requests[2].starts_with("GET /eSCL/ScanJobs/stalled-job/NextDocument "));
    assert!(requests[3].starts_with("DELETE /eSCL/ScanJobs/stalled-job "));
}

#[test]
fn simulated_batch_supports_duplex_and_advances_each_side_seed() {
    let session = open("escl:sim").unwrap();
    let request = ScanRequest {
        mode: ScanMode::Document,
        duplex: true,
        width: 1,
        height: 1,
        seed: 9,
        ..Default::default()
    };
    assert!(session.scan(&request).is_err());
    let mut seeds = Vec::new();
    assert_eq!(
        session
            .scan_pages(&request, 4, &mut |image| {
                seeds.push(image.data[2]);
                Ok(())
            })
            .unwrap(),
        crate::device::ScanPagesResult::limit_reached(4)
    );
    assert_eq!(seeds, vec![9, 10, 11, 12]);
}

#[test]
fn list_does_not_panic() {
    let _ = list_devices();
    assert_eq!(backend_info().id, "escl");
}

#[test]
fn cancelled_discovery_short_circuits_before_cold_network_work() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(list_escl_devices_safe_with_cancellation(Some(&cancellation)).is_empty());
}

#[test]
fn sim_open_and_scan() {
    let session = open("escl:sim").expect("open escl:sim");
    let req = ScanRequest {
        width: 12,
        height: 10,
        seed: 5,
        pixel_format: PixelFormat::Rgb8,
        ..Default::default()
    };
    let img = session.scan(&req).expect("scan");
    assert_eq!(img.width, 12);
}

#[test]
fn parse_job_id_from_absolute_location() {
    // Common eSCL absolute Location — must NOT become "http"
    let headers =
        "HTTP/1.1 201 Created\r\nLocation: http://192.168.1.50:80/eSCL/ScanJobs/abc-def-123\r\n";
    let id = parse_job_id(headers, b"").expect("job id");
    assert_eq!(id, "abc-def-123");
    assert_ne!(id, "http");

    let headers2 = "Location: https://printer.local:443/eSCL/ScanJobs/uuid-9\r\n";
    assert_eq!(parse_job_id(headers2, b"").as_deref(), Some("uuid-9"));

    let headers3 = "Location: /eSCL/ScanJobs/rel-id\r\n";
    assert_eq!(parse_job_id(headers3, b"").as_deref(), Some("rel-id"));

    let body = br#"<JobUri>/eSCL/ScanJobs/from-body</JobUri>"#;
    assert_eq!(
        parse_job_id("HTTP/1.1 201\r\n", body).as_deref(),
        Some("from-body")
    );
}

#[test]
fn endpoint_parsing_preserves_https_and_subnet_cap() {
    let endpoint = parse_endpoint("https://scanner.local:8443").unwrap();
    assert!(endpoint.secure);
    assert_eq!(endpoint.port, 8443);
    assert_eq!(endpoint.device_id(), "escl:https@scanner.local:8443");

    let candidates = subnet_candidates("192.168.44", 3);
    assert_eq!(candidates.len(), 3);
    assert_eq!(candidates[0].host, "192.168.44.1");
    assert!(subnet_candidates("10.0.0.1/31", 64).is_empty());

    assert!(subnet_candidates("8.8.8", 3).is_empty());
    assert!(subnet_candidates("203.0.113.0/24", 64).is_empty());
    assert!(subnet_candidates("127.0.0.0/24", 64).is_empty());
    assert!(subnet_candidates("224.0.0.0/24", 64).is_empty());
    assert_eq!(subnet_candidates("10.20.30.0/24", 2).len(), 2);
    assert_eq!(subnet_candidates("172.16.0.0/24", 2).len(), 2);
    assert_eq!(subnet_candidates("192.168.0.0/24", 2).len(), 2);
    assert_eq!(subnet_candidates("169.254.10.0/24", 2).len(), 2);
}

#[test]
fn endpoint_parsing_rejects_ambiguous_or_malformed_authorities() {
    for value in [
        " scanner.local",
        "scanner.local ",
        "http://scanner.local/path",
        "http://scanner.local?query",
        "http://scanner.local#fragment",
        "http://user@scanner.local",
        "http://[::1",
        "http://[::1]:",
        "http://[::1]extra",
        "::1",
        "scanner.local:",
        "scanner.local:not-a-port",
        "scanner.local:0",
        "999.1.1.1",
        "-scanner.local",
        "scanner-.local",
    ] {
        assert!(parse_endpoint(value).is_none(), "accepted {value:?}");
    }
    assert_eq!(
        parse_endpoint("[2001:db8::5]:8443").unwrap().host,
        "2001:db8::5"
    );
    assert_eq!(parse_endpoint("printer.local").unwrap().port, 80);
    assert_eq!(parse_endpoint("192.168.1.20:8080").unwrap().port, 8080);
}

#[test]
fn normal_open_requires_exact_allowlist_endpoint_and_opt_in_bypasses_it() {
    let _guard = ESCL_ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let previous = std::env::var_os("OPEN_SCANLINE_ESCL_HOSTS");
    std::env::set_var("OPEN_SCANLINE_ESCL_HOSTS", "printer.local");
    *ESCL_DEVICE_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap() = Some(Vec::new());

    assert!(open("escl:printer.local:80").is_ok());
    assert!(open("escl:printer.local.evil:80").is_err());
    let session = open_unlisted_endpoint("https://printer.local:8443").unwrap();
    assert_eq!(session.device_id, "escl:https@printer.local:8443");

    if let Some(value) = previous {
        std::env::set_var("OPEN_SCANLINE_ESCL_HOSTS", value);
    } else {
        std::env::remove_var("OPEN_SCANLINE_ESCL_HOSTS");
    }
}

#[test]
fn scanner_transport_rejects_redirects_without_contacting_target() {
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let redirect = TcpListener::bind("127.0.0.1:0").unwrap();
    let redirect_port = redirect.local_addr().unwrap().port();
    let target_port = target.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = redirect.accept().unwrap();
        let request = read_request(&mut stream);
        write_chunked(
            &mut stream,
            "302 Found",
            &format!("Location: http://127.0.0.1:{target_port}/target\r\n"),
            b"",
        );
        request
    });

    assert!(http_exchange(
        &Endpoint {
            host: "127.0.0.1".into(),
            port: redirect_port,
            secure: false,
        },
        "GET",
        "/source",
        None,
        None,
        Duration::from_secs(2),
        CAPABILITIES_RESPONSE_LIMIT,
    )
    .is_none());
    assert!(server.join().unwrap().starts_with("GET /source "));
    assert_eq!(
        target.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[test]
fn scanner_transport_ignores_proxy_environment() {
    let _guard = ESCL_ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    proxy.set_nonblocking(true).unwrap();
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    let target_port = target.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = target.accept().unwrap();
        let request = read_request(&mut stream);
        write_chunked(&mut stream, "200 OK", "", b"ok");
        request
    });
    let old_http_proxy = std::env::var_os("HTTP_PROXY");
    let old_http_proxy_lower = std::env::var_os("http_proxy");
    let old_no_proxy = std::env::var_os("NO_PROXY");
    std::env::set_var(
        "HTTP_PROXY",
        format!("http://{}", proxy.local_addr().unwrap()),
    );
    std::env::set_var(
        "http_proxy",
        format!("http://{}", proxy.local_addr().unwrap()),
    );
    std::env::remove_var("NO_PROXY");

    assert_eq!(
        http_exchange(
            &Endpoint {
                host: "127.0.0.1".into(),
                port: target_port,
                secure: false,
            },
            "GET",
            "/capabilities",
            None,
            None,
            Duration::from_secs(2),
            CAPABILITIES_RESPONSE_LIMIT,
        )
        .map(|(status, body, _)| (status, body)),
        Some((200, b"ok".to_vec()))
    );
    assert!(server.join().unwrap().starts_with("GET /capabilities "));
    assert_eq!(
        proxy.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );

    restore_env("HTTP_PROXY", old_http_proxy);
    restore_env("http_proxy", old_http_proxy_lower);
    restore_env("NO_PROXY", old_no_proxy);
}

fn restore_env(name: &str, value: Option<std::ffi::OsString>) {
    if let Some(value) = value {
        std::env::set_var(name, value);
    } else {
        std::env::remove_var(name);
    }
}

#[test]
fn mdns_only_accepts_local_unicast_address_ranges() {
    for address in [
        "10.0.0.1",
        "172.16.0.1",
        "192.168.0.1",
        "169.254.1.1",
        "fd12::1",
        "fe80::1",
    ] {
        assert!(mdns_address_is_local(address.parse().unwrap()), "{address}");
    }
    for address in [
        "127.0.0.1",
        "0.0.0.0",
        "8.8.8.8",
        "224.0.0.1",
        "::1",
        "ff02::1",
    ] {
        assert!(
            !mdns_address_is_local(address.parse().unwrap()),
            "{address}"
        );
    }
}

fn resolved_socket_addrs(addresses: &[&str]) -> ResolvedSocketAddrs {
    let mut resolved = ArrayVec::from_fn(|_| SocketAddr::from(([0, 0, 0, 0], 0)));
    for address in addresses {
        resolved.push(address.parse().unwrap());
    }
    resolved
}

#[test]
fn mdns_resolver_filters_nonlocal_socket_addresses() {
    let uri = "http://scanner.local:8080/eSCL/ScannerCapabilities"
        .parse::<Uri>()
        .unwrap();
    let resolved = filter_mdns_resolved_addresses(
        &uri,
        resolved_socket_addrs(&[
            "10.0.0.8:8080",
            "169.254.10.8:8080",
            "[fd12::8]:8080",
            "[fe80::8]:8080",
            "127.0.0.1:8080",
            "0.0.0.0:8080",
            "8.8.8.8:8080",
            "224.0.0.1:8080",
            "[::1]:8080",
            "[ff02::1]:8080",
        ]),
    )
    .unwrap();

    assert_eq!(
        resolved.as_ref(),
        [
            "10.0.0.8:8080".parse::<SocketAddr>().unwrap(),
            "169.254.10.8:8080".parse().unwrap(),
            "[fd12::8]:8080".parse().unwrap(),
            "[fe80::8]:8080".parse().unwrap(),
        ]
    );
}

#[test]
fn mdns_resolver_returns_host_not_found_when_every_address_is_rejected() {
    let uri = "https://scanner.local/eSCL/ScannerCapabilities"
        .parse::<Uri>()
        .unwrap();
    let error = filter_mdns_resolved_addresses(
        &uri,
        resolved_socket_addrs(&["127.0.0.1:443", "8.8.8.8:443", "[::1]:443"]),
    )
    .unwrap_err();

    assert!(matches!(error, ureq::Error::HostNotFound));
}

#[test]
fn mdns_resolver_leaves_nonlocal_hostnames_unchanged() {
    let uri = "http://scanner.example:8080/eSCL/ScannerCapabilities"
        .parse::<Uri>()
        .unwrap();
    let addresses = resolved_socket_addrs(&["127.0.0.1:8080", "8.8.8.8:8080", "[::1]:8080"]);

    let resolved = filter_mdns_resolved_addresses(&uri, addresses).unwrap();

    assert_eq!(
        resolved.as_ref(),
        [
            "127.0.0.1:8080".parse::<SocketAddr>().unwrap(),
            "8.8.8.8:8080".parse().unwrap(),
            "[::1]:8080".parse().unwrap(),
        ]
    );
}

#[test]
fn secure_mdns_service_forces_tls_on_a_nonstandard_port() {
    let service = mdns_sd::ServiceInfo::new(
        "_uscans._tcp.local.",
        "secure scanner",
        "scanner.local.",
        "192.168.50.8",
        8443,
        None::<std::collections::HashMap<String, String>>,
    )
    .unwrap();

    assert_eq!(
        endpoints_for_service(&service, true),
        vec![Endpoint {
            host: "scanner.local".into(),
            port: 8443,
            secure: true,
        }]
    );
    assert!(!endpoints_for_service(&service, false)[0].secure);
    assert_eq!(
        endpoints_for_service(&service, true)[0].url("/eSCL/ScannerCapabilities"),
        "https://scanner.local:8443/eSCL/ScannerCapabilities"
    );
}

#[test]
fn absolute_job_location_must_match_session_origin() {
    let endpoint = Endpoint {
        host: "192.168.1.20".into(),
        port: 80,
        secure: false,
    };
    assert_eq!(
        canonical_job_path(
            "Location: http://192.168.1.20:80/eSCL/ScanJobs/ok\r\n",
            b"",
            "eSCL",
            &endpoint,
        ),
        Some("/eSCL/ScanJobs/ok".into())
    );
    assert!(canonical_job_path(
        "Location: http://evil.example/eSCL/ScanJobs/not-ok\r\n",
        b"",
        "eSCL",
        &endpoint,
    )
    .is_none());
}

#[test]
fn loopback_http_discovery_and_scan_decode_chunked_responses() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let image = fixture_png();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Scanner/></ScannerCapabilities>".as_slice(),
            ),
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            (
                "201 Created",
                "Location: /eSCL/ScanJobs/loopback-job\r\n",
                b"".as_slice(),
            ),
            ("200 OK", "Content-Type: image/png\r\n", image.as_slice()),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
        }
        requests
    });

    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };
    let device = probe_endpoint(&endpoint).expect("chunked capabilities discovery");
    assert_eq!(device.id, format!("escl:127.0.0.1:{port}"));

    let session = EsclDeviceSession::new(device.id, endpoint, false);
    let image = session
        .scan(&ScanRequest {
            width: 1,
            height: 1,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        })
        .expect("chunked NextDocument image");
    assert_eq!(image.data, vec![12, 34, 56]);

    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /eSCL/ScannerCapabilities "));
    assert!(requests[1].starts_with("GET /eSCL/ScannerCapabilities "));
    assert!(requests[2].starts_with("POST /eSCL/ScanJobs "));
    assert!(requests[3].starts_with("GET /eSCL/ScanJobs/loopback-job/NextDocument "));
}

#[test]
fn loopback_discovery_allows_a_bounded_capability_response_delay() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        // Control setup is capped at one second, but a scanner may still take
        // longer to generate a capability response header.
        thread::sleep(Duration::from_millis(1250));
        write_chunked(
            &mut stream,
            "200 OK",
            "Content-Type: text/xml\r\n",
            b"<ScannerCapabilities><Scanner/></ScannerCapabilities>",
        );
        request
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };

    let started = Instant::now();
    assert!(probe_endpoint(&endpoint).is_some());
    assert!(started.elapsed() < ENDPOINT_PROBE_TIMEOUT);
    assert!(server
        .join()
        .unwrap()
        .starts_with("GET /eSCL/ScannerCapabilities "));
}

#[test]
fn endpoint_probe_pool_obeys_one_overall_budget() {
    let listeners = (0..12)
        .map(|_| TcpListener::bind("127.0.0.1:0").unwrap())
        .collect::<Vec<_>>();
    let endpoints = listeners
        .iter()
        .map(|listener| Endpoint {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
            secure: false,
        })
        .collect::<Vec<_>>();
    let budget = Duration::from_millis(250);
    let started = Instant::now();

    assert!(probe_endpoints(&endpoints, budget).is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "probe pool exceeded its shared deadline: {:?}",
        started.elapsed()
    );
}

#[test]
fn cancelled_probe_pool_returns_without_waiting_for_the_in_flight_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        request_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut byte = [0_u8; 1];
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
        closed_tx.send(()).unwrap();
        request
    });
    let cancellation = CancellationToken::new();
    let probing_cancellation = cancellation.clone();
    let probe = thread::spawn(move || {
        probe_endpoints_with_cancellation(
            &[Endpoint {
                host: "127.0.0.1".into(),
                port,
                secure: false,
            }],
            Duration::from_secs(5),
            Some(&probing_cancellation),
        )
    });

    request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    cancellation.cancel();
    assert!(probe.join().unwrap().is_empty());
    assert!(started.elapsed() < Duration::from_secs(1));
    closed_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("all probe transports must close before the joined probe returns");
    assert!(server
        .join()
        .unwrap()
        .starts_with("GET /eSCL/ScannerCapabilities "));
}

#[test]
fn cancelled_probe_retries_never_exceed_the_worker_socket_cap() {
    const ROUNDS: usize = 3;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let server_active = Arc::clone(&active);
    let server_peak = Arc::clone(&peak);
    let server = thread::spawn(move || {
        let mut readers = Vec::new();
        for _ in 0..(ROUNDS * MAX_PROBE_WORKERS) {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(read_request(&mut stream).starts_with("GET /eSCL/ScannerCapabilities "));
            let active = Arc::clone(&server_active);
            let peak = Arc::clone(&server_peak);
            let closed_tx = closed_tx.clone();
            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(current, Ordering::SeqCst);
            request_tx.send(()).unwrap();
            readers.push(thread::spawn(move || {
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut byte = [0_u8; 1];
                assert_eq!(stream.read(&mut byte).unwrap(), 0);
                active.fetch_sub(1, Ordering::SeqCst);
                closed_tx.send(()).unwrap();
            }));
        }
        for reader in readers {
            reader.join().unwrap();
        }
    });

    for _ in 0..ROUNDS {
        let cancellation = CancellationToken::new();
        let probing_cancellation = cancellation.clone();
        let endpoints = (0..MAX_PROBE_WORKERS)
            .map(|_| Endpoint {
                host: "127.0.0.1".into(),
                port,
                secure: false,
            })
            .collect::<Vec<_>>();
        let probe = thread::spawn(move || {
            probe_endpoints_with_cancellation(
                &endpoints,
                Duration::from_secs(5),
                Some(&probing_cancellation),
            )
        });
        for _ in 0..MAX_PROBE_WORKERS {
            request_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        cancellation.cancel();
        assert!(probe.join().unwrap().is_empty());
        for _ in 0..MAX_PROBE_WORKERS {
            closed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        }
        assert_eq!(active.load(Ordering::SeqCst), 0);
    }
    server.join().unwrap();
    assert!(peak.load(Ordering::SeqCst) <= MAX_PROBE_WORKERS);
}

#[test]
fn capabilities_response_over_limit_is_rejected() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        let mut body = b"<ScannerCapabilities><Scanner/>".to_vec();
        body.resize(CAPABILITIES_RESPONSE_LIMIT as usize + 1, b'X');
        write_chunked(&mut stream, "200 OK", "Content-Type: text/xml\r\n", &body);
        request
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };

    assert!(probe_endpoint_until(&endpoint, Instant::now() + Duration::from_secs(3)).is_none());
    assert!(server
        .join()
        .unwrap()
        .starts_with("GET /eSCL/ScannerCapabilities "));
}

#[test]
fn next_document_stream_accepts_the_exact_limit_and_rejects_one_more_byte() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        for body in [b"12345678".as_slice(), b"123456789".as_slice()] {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = read_request(&mut stream);
            write_chunked(&mut stream, "200 OK", "", body);
        }
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };

    let (_, output) = http_get_to_temporary_output(
        &endpoint,
        "/eSCL/ScanJobs/limit/NextDocument",
        Duration::from_secs(2),
        8,
    )
    .unwrap();
    let output = output.unwrap();
    assert_eq!(std::fs::read(output.path()).unwrap(), b"12345678");
    drop(output);

    let error = http_get_to_temporary_output(
        &endpoint,
        "/eSCL/ScanJobs/limit/NextDocument",
        Duration::from_secs(2),
        8,
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        FetchDocumentError::Failed(message)
            if message == "NextDocument response exceeds byte limit"
    ));
    server.join().unwrap();
}

#[test]
fn next_document_body_can_stream_longer_than_the_setup_timeout() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\n1234")
            .unwrap();
        stream.flush().unwrap();
        // The body may legitimately take longer than the short setup phases.
        thread::sleep(NEXT_DOCUMENT_SETUP_TIMEOUT + Duration::from_millis(250));
        stream.write_all(b"5678").unwrap();
        stream.flush().unwrap();
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };

    let (_, output) = http_get_to_temporary_output_with_phase_timeout(
        &endpoint,
        "/eSCL/ScanJobs/slow/NextDocument",
        Duration::from_secs(15),
        Duration::from_millis(100),
        8,
    )
    .unwrap();
    let output = output.unwrap();
    assert_eq!(std::fs::read(output.path()).unwrap(), b"12345678");
    server.join().unwrap();
}

#[test]
fn next_document_header_wait_has_setup_timeout_and_cancellation() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (request_tx, request_rx) = mpsc::channel();
    let (closed_tx, closed_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            assert!(
                read_request(&mut stream).starts_with("GET /eSCL/ScanJobs/stalled/NextDocument ")
            );
            request_tx.send(()).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut byte = [0_u8; 1];
            assert_eq!(stream.read(&mut byte).unwrap(), 0);
            closed_tx.send(()).unwrap();
        }
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };

    let started = Instant::now();
    assert!(matches!(
        http_get_to_temporary_output_with_phase_timeout(
            &endpoint,
            "/eSCL/ScanJobs/stalled/NextDocument",
            Duration::from_secs(2),
            Duration::from_millis(100),
            8,
        ),
        Err(FetchDocumentError::RetryableTimeout)
    ));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "header setup elapsed: {:?}",
        started.elapsed()
    );
    request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    closed_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let cancellation = CancellationToken::new();
    let request_cancellation = cancellation.clone();
    let cancelling_endpoint = endpoint.clone();
    let request = thread::spawn(move || {
        http_get_to_temporary_output_with_phase_timeout_and_cancellation(
            &cancelling_endpoint,
            "/eSCL/ScanJobs/stalled/NextDocument",
            Duration::from_secs(60),
            NEXT_DOCUMENT_SETUP_TIMEOUT,
            8,
            Some(&request_cancellation),
        )
    });
    request_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    let started = Instant::now();
    cancellation.cancel();
    assert!(matches!(
        request.join().unwrap(),
        Err(FetchDocumentError::Cancelled)
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    closed_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    server.join().unwrap();
}

#[test]
fn next_document_rejects_an_oversized_content_length_before_materializing() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n123456789",
            )
            .unwrap();
        stream.flush().unwrap();
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: false,
    };

    let error = http_get_to_temporary_output(
        &endpoint,
        "/eSCL/ScanJobs/limit/NextDocument",
        Duration::from_secs(2),
        8,
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        FetchDocumentError::Failed(message)
            if message == "NextDocument response exceeds byte limit"
    ));
    server.join().unwrap();
}

#[test]
fn scan_falls_back_from_escl_to_scan_root_after_job_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let image = fixture_png();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, headers, body) in [
            (
                "200 OK",
                "Content-Type: text/xml\r\n",
                b"<ScannerCapabilities><Platen/></ScannerCapabilities>".as_slice(),
            ),
            ("404 Not Found", "", b"".as_slice()),
            (
                "201 Created",
                "Location: /Scan/ScanJobs/fallback-job\r\n",
                b"".as_slice(),
            ),
            ("200 OK", "Content-Type: image/png\r\n", image.as_slice()),
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            requests.push(read_request(&mut stream));
            write_chunked(&mut stream, status, headers, body);
        }
        requests
    });
    let session = EsclDeviceSession::new(
        format!("escl:127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: false,
        },
        false,
    );

    let image = session
        .scan(&ScanRequest {
            width: 1,
            height: 1,
            pixel_format: PixelFormat::Rgb8,
            ..Default::default()
        })
        .expect("scan root fallback image");
    assert_eq!(image.data, vec![12, 34, 56]);

    let requests = server.join().unwrap();
    assert!(requests[0].starts_with("GET /eSCL/ScannerCapabilities "));
    assert!(requests[1].starts_with("POST /eSCL/ScanJobs "));
    assert!(requests[2].starts_with("POST /Scan/ScanJobs "));
    assert!(requests[3].starts_with("GET /Scan/ScanJobs/fallback-job/NextDocument "));
}

#[test]
fn https_transport_sends_tls_and_never_downgrades_on_handshake_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut hello = [0u8; 3];
        stream.read_exact(&mut hello).unwrap();
        hello
    });
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        secure: true,
    };
    assert!(http_exchange(
        &endpoint,
        "GET",
        "/eSCL/ScannerCapabilities",
        None,
        None,
        Duration::from_secs(2),
        CAPABILITIES_RESPONSE_LIMIT,
    )
    .is_none());
    let hello = server.join().unwrap();
    assert_eq!(
        hello[0], 0x16,
        "HTTPS must start with a TLS handshake record"
    );
    assert_eq!(hello[1], 0x03, "TLS record major version");
}

#[test]
fn cancellation_bounds_a_stalled_tls_handshake_without_alternate_root_fallback() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        accepted_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut bytes = [0_u8; 1024];
        loop {
            if stream.read(&mut bytes).unwrap() == 0 {
                break;
            }
        }
        listener.set_nonblocking(true).unwrap();
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    });
    let session = Arc::new(EsclDeviceSession::new(
        format!("escl:https@127.0.0.1:{port}"),
        Endpoint {
            host: "127.0.0.1".into(),
            port,
            secure: true,
        },
        false,
    ));
    let cancellation = CancellationToken::new();
    session.bind_cancellation(cancellation.clone());
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        scanning_session.scan_pages(
            &ScanRequest {
                width: 1,
                height: 1,
                ..Default::default()
            },
            1,
            &mut |_| Ok(()),
        )
    });

    accepted_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let started = Instant::now();
    cancellation.cancel();
    assert!(matches!(scan.join().unwrap(), Err(ScanError::Cancelled(_))));
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "TLS setup exceeded its bounded control-request phase: {:?}",
        started.elapsed()
    );
    server.join().unwrap();
}
