use super::*;
use std::time::{Duration, Instant};

struct ControlledDiscovery {
    started: Sender<(String, bool)>,
    release: Mutex<Receiver<()>>,
}

impl DiscoveryService for ControlledDiscovery {
    fn discover(&self, device: &str, refresh: bool, _: &CancellationToken) -> DiscoveryResult {
        self.started.send((device.into(), refresh)).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(3))
            .unwrap();
        if device == "failure" {
            return Err(ScanError::Other("fixture discovery failure".into()));
        }
        Ok(DiscoverySnapshot {
            devices: vec![device.into()],
            selected: device.into(),
            capabilities: DeviceMaintenanceCapabilities::simulated_point_focus(),
        })
    }
}

fn controlled_app() -> (OpenScanlineApp, Receiver<(String, bool)>, Sender<()>) {
    let (started, requests) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let mut app = OpenScanlineApp::new(None);
    app.discovery = DiscoveryWorker::new(Arc::new(ControlledDiscovery {
        started,
        release: Mutex::new(wait),
    }));
    (app, requests, release)
}

fn drain(app: &mut OpenScanlineApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while app.state.discovery_loading {
        assert!(
            Instant::now() < deadline,
            "discovery worker did not complete"
        );
        app.poll_discovery(ctx);
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn delayed_startup_is_nonblocking_and_disables_maintenance() {
    let (mut app, requests, release) = controlled_app();
    let ctx = egui::Context::default();
    assert!(app.state.discovery_loading);
    app.poll_discovery(&ctx);
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    for _ in 0..20 {
        app.poll_discovery(&ctx);
    }
    assert!(!app.state.calibration_available());
    release.send(()).unwrap();
    drain(&mut app, &ctx);
    assert!(app.state.calibration_available());
}

#[test]
fn superseded_requests_coalesce_and_reject_stale_selection() {
    let (mut app, requests, release) = controlled_app();
    let ctx = egui::Context::default();
    app.poll_discovery(&ctx);
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    app.state.refresh_devices();
    app.state.device = "intermediate".into();
    app.poll_discovery(&ctx);
    app.state.device = "latest".into();
    app.poll_discovery(&ctx);
    release.send(()).unwrap();
    let (selected, refresh) = requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(selected, "latest");
    assert!(refresh);
    app.poll_discovery(&ctx);
    assert!(!app.state.calibration_available());
    release.send(()).unwrap();
    drain(&mut app, &ctx);
    assert_eq!(app.state.devices, ["latest"]);
    assert!(requests.try_recv().is_err());
}

#[test]
fn cancelled_and_failed_discovery_preserve_inventory() {
    let (mut app, requests, release) = controlled_app();
    let ctx = egui::Context::default();
    let initial = app.state.devices.clone();
    app.poll_discovery(&ctx);
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    app.cancel_discovery();
    release.send(()).unwrap();
    app.state.device = "failure".into();
    app.poll_discovery(&ctx);
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(app.state.devices, initial);
    release.send(()).unwrap();
    drain(&mut app, &ctx);
    assert_eq!(app.state.devices, initial);
    assert!(app.state.status.contains("fixture discovery failure"));
}

#[test]
fn close_defers_until_discovery_worker_exits() {
    let (mut app, requests, release) = controlled_app();
    let ctx = egui::Context::default();
    app.poll_discovery(&ctx);
    requests.recv_timeout(Duration::from_secs(1)).unwrap();
    app.request_close(&ctx);
    assert!(app.closing);
    assert!(!app.discovery_finished());
    assert!(app.discovery.token.is_cancelled());
    release.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !app.discovery_finished() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.finish_deferred_close(&ctx);
}

struct ReconciledDiscovery {
    maintenance_fails: bool,
}

impl DiscoveryService for ReconciledDiscovery {
    fn discover(&self, device: &str, _: bool, token: &CancellationToken) -> DiscoveryResult {
        reconcile_inventory(
            vec!["mock".into(), "other".into()],
            device,
            token,
            |selected| {
                assert_eq!(
                    selected, "mock",
                    "capabilities must inspect the reconciled selection"
                );
                if self.maintenance_fails {
                    Err(ScanError::Other("fixture maintenance failure".into()))
                } else {
                    Ok(DeviceMaintenanceCapabilities::simulated_point_focus())
                }
            },
        )
    }
}

fn reconciled_app(maintenance_fails: bool) -> OpenScanlineApp {
    let mut app = OpenScanlineApp::new(None);
    app.state.device = "disconnected".into();
    app.state.devices = vec!["disconnected".into()];
    app.discovery = DiscoveryWorker::new(Arc::new(ReconciledDiscovery { maintenance_fails }));
    app
}

#[test]
fn refresh_replaces_missing_selection_and_uses_replacement_capabilities() {
    let mut app = reconciled_app(false);
    let ctx = egui::Context::default();
    app.state.refresh_devices();
    drain(&mut app, &ctx);
    assert_eq!(app.state.devices, ["mock", "other"]);
    assert_eq!(app.state.device, "mock");
    assert!(app.state.calibration_available());
    assert_eq!(app.discovery.selected.as_deref(), Some("mock"));
    app.poll_discovery(&ctx);
    assert!(
        !app.state.discovery_loading,
        "reconciliation must not launch redundant discovery"
    );
}

#[test]
fn startup_reconciles_persisted_selection_despite_maintenance_failure() {
    let mut app = reconciled_app(true);
    let ctx = egui::Context::default();
    drain(&mut app, &ctx);
    assert_eq!(app.state.devices, ["mock", "other"]);
    assert_eq!(app.state.device, "mock");
    assert!(!app.state.calibration_available());
    assert!(app
        .state
        .calibration_explanation()
        .contains("fixture maintenance failure"));
}

#[test]
fn cancellation_during_maintenance_discards_reconciled_inventory() {
    let token = CancellationToken::new();
    let result = reconcile_inventory(vec!["mock".into()], "disconnected", &token, |_| {
        token.cancel();
        Ok(DeviceMaintenanceCapabilities::simulated_point_focus())
    });
    assert!(matches!(result, Err(ScanError::Cancelled(_))));
}
