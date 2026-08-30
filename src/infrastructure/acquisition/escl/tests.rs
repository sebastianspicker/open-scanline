use super::*;
use crate::workflows::ports::acquisition::DeviceSession;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{mpsc, Arc};
use std::thread;

fn read_request(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 1024];
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

fn write_chunked(stream: &mut TcpStream, status: &str, extra_headers: &str, body: &[u8]) {
    let mut response = format!(
        "HTTP/1.1 {status}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n{extra_headers}\r\n"
    )
    .into_bytes();
    if !body.is_empty() {
        response.extend_from_slice(format!("{:X}\r\n", body.len()).as_bytes());
        response.extend_from_slice(body);
        response.extend_from_slice(b"\r\n");
    }
    response.extend_from_slice(b"0\r\n\r\n");
    stream.write_all(&response).unwrap();
    stream.flush().unwrap();
}

fn wait_for_connection_close(stream: &mut TcpStream, timeout: Duration) {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let deadline = Instant::now() + timeout;
    let mut byte = [0_u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) => return,
            Ok(_) => panic!("client sent unexpected data while closing NextDocument transport"),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) && Instant::now() < deadline => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                panic!("timed out waiting for NextDocument transport to close: {error}")
            }
            Err(error) => {
                panic!("failed while waiting for NextDocument transport to close: {error}")
            }
        }
    }
}

#[test]
fn stalled_next_document_cancellation_cleans_output_and_deletes_the_job() {
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
        wait_for_connection_close(&mut document_stream, Duration::from_secs(2));
        document_closed_tx.send(()).unwrap();

        let (mut delete_stream, _) = listener.accept().unwrap();
        requests.push(read_request(&mut delete_stream));
        write_chunked(&mut delete_stream, "200 OK", "", b"");
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
    let scanning_session = Arc::clone(&session);
    let scan = thread::spawn(move || {
        result_tx
            .send(scanning_session.scan_pages(
                &ScanRequest {
                    width: 1,
                    height: 1,
                    ..Default::default()
                },
                1,
                &mut |_| Ok(()),
            ))
            .unwrap();
    });

    body_started_rx
        .recv_timeout(Duration::from_secs(2))
        .unwrap();
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
    assert!(requests[2].starts_with("GET /eSCL/ScanJobs/stalled-job/NextDocument "));
    assert!(requests[3].starts_with("DELETE /eSCL/ScanJobs/stalled-job "));
}
