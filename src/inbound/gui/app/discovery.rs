use super::*;
use crate::infrastructure::acquisition::DeviceMaintenanceCapabilities;
use std::sync::{Condvar, Mutex};

type DiscoveryResult = crate::error::Result<DiscoverySnapshot>;

pub(super) struct DiscoverySnapshot {
    devices: Vec<String>,
    selected: String,
    capabilities: DeviceMaintenanceCapabilities,
}

fn reconcile_inventory(
    devices: Vec<String>,
    requested: &str,
    token: &CancellationToken,
    inspect: impl FnOnce(&str) -> crate::error::Result<DeviceMaintenanceCapabilities>,
) -> DiscoveryResult {
    if token.is_cancelled() {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    let selected = if devices.iter().any(|device| device == requested) {
        requested.to_owned()
    } else {
        devices.first().cloned().unwrap_or_else(|| "mock".into())
    };
    let capabilities = inspect(&selected).unwrap_or_else(|error| {
        DeviceMaintenanceCapabilities::unsupported(format!(
            "could not inspect maintenance capabilities: {error}"
        ))
    });
    if token.is_cancelled() {
        return Err(ScanError::Cancelled("discovery cancelled".into()));
    }
    Ok(DiscoverySnapshot {
        devices,
        selected,
        capabilities,
    })
}

pub(super) trait DiscoveryService: Send + Sync {
    fn discover(&self, device: &str, refresh: bool, token: &CancellationToken) -> DiscoveryResult;
}

pub(super) struct NativeDiscovery;
impl DiscoveryService for NativeDiscovery {
    fn discover(&self, device: &str, refresh: bool, token: &CancellationToken) -> DiscoveryResult {
        use crate::infrastructure::acquisition::{
            find_scanners_with_cancellation, maintenance_capabilities_with_cancellation,
        };
        let devices = find_scanners_with_cancellation(refresh, Some(token))
            .into_iter()
            .map(|device| device.id)
            .collect();
        reconcile_inventory(devices, device, token, |selected| {
            maintenance_capabilities_with_cancellation(selected, token)
        })
    }
}

struct Request {
    generation: u64,
    device: String,
    refresh: bool,
    token: CancellationToken,
}

#[derive(Default)]
struct Queue {
    pending: Option<Request>,
    stop: bool,
}

struct Reply {
    generation: u64,
    device: String,
    result: DiscoveryResult,
}

pub(super) struct DiscoveryWorker {
    queue: Arc<(Mutex<Queue>, Condvar)>,
    receiver: Receiver<Reply>,
    handle: Option<JoinHandle<()>>,
    token: CancellationToken,
    generation: u64,
    selected: Option<String>,
    refreshing: bool,
}

impl DiscoveryWorker {
    pub(super) fn new(service: Arc<dyn DiscoveryService>) -> Self {
        let queue = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let worker_queue = Arc::clone(&queue);
        let (sender, receiver) = mpsc::channel();
        let handle = std::thread::spawn(move || run_worker(service, worker_queue, sender));
        Self {
            queue,
            receiver,
            handle: Some(handle),
            token: CancellationToken::new(),
            generation: 0,
            selected: None,
            refreshing: false,
        }
    }

    fn request(&mut self, device: String, refresh: bool) {
        let refresh = refresh || self.refreshing;
        self.refreshing = refresh;
        self.token.cancel();
        self.token = CancellationToken::new();
        self.generation = self.generation.wrapping_add(1);
        self.selected = Some(device.clone());
        let (lock, wake) = &*self.queue;
        let mut queue = lock.lock().unwrap_or_else(|error| error.into_inner());
        let refresh = refresh
            || queue
                .pending
                .as_ref()
                .is_some_and(|request| request.refresh);
        queue.pending = Some(Request {
            generation: self.generation,
            device,
            refresh,
            token: self.token.clone(),
        });
        wake.notify_one();
    }

    fn cancel(&mut self) {
        self.token.cancel();
        self.generation = self.generation.wrapping_add(1);
        self.queue
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending = None;
    }

    fn stop(&mut self) {
        self.cancel();
        self.queue
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .stop = true;
        self.queue.1.notify_one();
    }

    pub(super) fn finished(&self) -> bool {
        self.handle.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for DiscoveryWorker {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn next_request(queue: &Arc<(Mutex<Queue>, Condvar)>) -> Option<Request> {
    let (lock, wake) = &**queue;
    let mut pending = lock.lock().unwrap_or_else(|error| error.into_inner());
    while !pending.stop && pending.pending.is_none() {
        pending = wake
            .wait(pending)
            .unwrap_or_else(|error| error.into_inner());
    }
    if pending.stop {
        None
    } else {
        pending.pending.take()
    }
}

fn run_worker(
    service: Arc<dyn DiscoveryService>,
    queue: Arc<(Mutex<Queue>, Condvar)>,
    sender: Sender<Reply>,
) {
    while let Some(request) = next_request(&queue) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            service.discover(&request.device, request.refresh, &request.token)
        }))
        .unwrap_or_else(|payload| Err(ScanError::Other(panic_message(payload))));
        if !request.token.is_cancelled() {
            let _ = sender.send(Reply {
                generation: request.generation,
                device: request.device,
                result,
            });
        }
    }
}

impl OpenScanlineApp {
    pub(in crate::inbound::gui) fn poll_discovery(&mut self, ctx: &egui::Context) {
        if self.closing {
            return;
        }
        if self.state.discovery_requested
            || self.discovery.selected.as_ref() != Some(&self.state.device)
        {
            self.state.refresh_maintenance_capabilities();
            self.discovery
                .request(self.state.device.clone(), self.state.discovery_refresh);
            self.state.discovery_requested = false;
            self.state.discovery_refresh = false;
        }
        while let Ok(reply) = self.discovery.receiver.try_recv() {
            if reply.generation == self.discovery.generation && reply.device == self.state.device {
                self.apply_discovery(reply.result);
            }
        }
        if self.state.discovery_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    fn apply_discovery(&mut self, result: DiscoveryResult) {
        self.state.discovery_loading = false;
        self.discovery.refreshing = false;
        match result {
            Ok(snapshot) => {
                self.state.devices = snapshot.devices;
                self.discovery.selected = Some(snapshot.selected.clone());
                self.state.device = snapshot.selected;
                self.state.maintenance_capabilities = snapshot.capabilities;
                self.state.status = format!(
                    "{}: {}",
                    self.state.translator.t("devices"),
                    self.state.devices.len()
                );
            }
            Err(error) => self.state.set_error(error),
        }
    }

    pub(in crate::inbound::gui) fn cancel_discovery(&mut self) {
        self.discovery.cancel();
        self.state.discovery_requested = false;
        self.state.discovery_loading = false;
    }

    pub(in crate::inbound::gui) fn stop_discovery(&mut self) {
        self.discovery.stop();
        self.state.discovery_loading = false;
    }

    pub(in crate::inbound::gui) fn discovery_finished(&self) -> bool {
        self.discovery.finished()
    }
}
