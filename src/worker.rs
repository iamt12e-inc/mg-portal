use crate::{config::Settings, credentials, network, portal};
use std::sync::{
    Arc, Mutex,
    mpsc::{Receiver, Sender},
};
use std::time::{Duration, Instant};

const PORTAL_POLL_INTERVAL: Duration = Duration::from_secs(2);
const NETWORK_STATE_POLL_INTERVAL: Duration = Duration::from_millis(500);
const BACKGROUND_STATE_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorCommand {
    CheckNow,
    ForceSignIn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusKind {
    Disconnected,
    Waiting,
    Online,
    NeedsCredentials,
    Manual,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppStatus {
    pub kind: StatusKind,
    pub title: String,
    pub detail: String,
    pub portal_url: Option<String>,
    pub can_try_sign_in: bool,
}

impl AppStatus {
    fn new(kind: StatusKind, title: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            title: title.into(),
            detail: detail.into(),
            portal_url: None,
            can_try_sign_in: false,
        }
    }

    fn with_portal(mut self, page_url: String) -> Self {
        self.portal_url = Some(page_url);
        self
    }

    fn with_sign_in_action(mut self) -> Self {
        self.can_try_sign_in = true;
        self
    }
}

pub fn run(
    settings: Arc<Mutex<Settings>>,
    sender: Sender<AppStatus>,
    command_receiver: Receiver<MonitorCommand>,
) {
    let mut last_status: Option<AppStatus> = None;
    let mut last_ssid: Option<String> = None;
    let mut last_attempt_key: Option<String> = None;
    let mut failed_submission_key: Option<String> = None;
    let mut last_check = Instant::now() - PORTAL_POLL_INTERVAL;
    let mut force_submission_once = false;

    loop {
        let snapshot = match settings.lock() {
            Ok(settings) => settings.clone(),
            Err(_) => {
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::Error,
                        "Couldn't load the app",
                        "Close and reopen the app. If this keeps happening, reinstall it.",
                    ),
                );
                wait_for_command(
                    &command_receiver,
                    Duration::from_secs(10),
                    &mut last_check,
                    &mut last_attempt_key,
                    &mut force_submission_once,
                );
                continue;
            }
        };

        let ssid = match network::active_wifi_ssid() {
            Ok(ssid) => ssid,
            Err(error) => {
                eprintln!("Could not read the Wi-Fi connection: {error:#}");
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::Error,
                        "Can't check your Wi-Fi",
                        "The app couldn't read your Wi-Fi connection. Check that Wi-Fi is on, then try again.",
                    ),
                );
                wait_for_command(
                    &command_receiver,
                    BACKGROUND_STATE_POLL_INTERVAL,
                    &mut last_check,
                    &mut last_attempt_key,
                    &mut force_submission_once,
                );
                continue;
            }
        };

        let Some(ssid) = ssid else {
            last_ssid = None;
            last_attempt_key = None;
            failed_submission_key = None;
            force_submission_once = false;
            last_check = Instant::now() - PORTAL_POLL_INTERVAL;
            publish(
                &sender,
                &mut last_status,
                AppStatus::new(
                    StatusKind::Disconnected,
                    "Not connected to Wi-Fi",
                    "Waiting for a Wi-Fi connection.",
                ),
            );
            wait_for_command(
                &command_receiver,
                BACKGROUND_STATE_POLL_INTERVAL,
                &mut last_check,
                &mut last_attempt_key,
                &mut force_submission_once,
            );
            continue;
        };

        if last_ssid.as_deref() != Some(ssid.as_str()) {
            last_ssid = Some(ssid.clone());
            last_attempt_key = None;
            failed_submission_key = None;
            force_submission_once = false;
            last_check = Instant::now() - PORTAL_POLL_INTERVAL;
        }

        if !snapshot
            .target_networks
            .iter()
            .any(|target| target == &ssid)
        {
            publish(
                &sender,
                &mut last_status,
                AppStatus::new(
                    StatusKind::Waiting,
                    "Waiting for Mugonat",
                    format!(
                        "Connected to {ssid}. Portal sign-in is set up only for the selected networks."
                    ),
                ),
            );
            force_submission_once = false;
            wait_for_command(
                &command_receiver,
                BACKGROUND_STATE_POLL_INTERVAL,
                &mut last_check,
                &mut last_attempt_key,
                &mut force_submission_once,
            );
            continue;
        }

        let attempt_key = format!("{}|{}", ssid, snapshot.revision);
        let due = last_check.elapsed() >= PORTAL_POLL_INTERVAL;
        let changed = last_attempt_key.as_deref() != Some(attempt_key.as_str());
        if !due && !changed {
            wait_for_command(
                &command_receiver,
                NETWORK_STATE_POLL_INTERVAL,
                &mut last_check,
                &mut last_attempt_key,
                &mut force_submission_once,
            );
            continue;
        }
        last_check = Instant::now();

        let saved = match credentials::load() {
            Ok(saved) => saved,
            Err(error) => {
                eprintln!("Could not read saved sign-in details: {error:#}");
                last_attempt_key = Some(attempt_key);
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::Error,
                        "Sign-in details unavailable",
                        "Unlock your password manager, then try again.",
                    )
                    .with_sign_in_action(),
                );
                continue;
            }
        };
        let credentials = saved
            .as_ref()
            .map(|(username, password)| (username.as_str(), password.as_str()));

        let force_submission = std::mem::take(&mut force_submission_once);
        let allow_submission =
            force_submission || failed_submission_key.as_deref() != Some(attempt_key.as_str());
        match portal::check_and_login(credentials, allow_submission) {
            Ok(portal::Outcome::Online) => {
                failed_submission_key = None;
                last_attempt_key = Some(attempt_key);
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::Online,
                        "You're online",
                        format!("Connected to {ssid}."),
                    )
                    .with_sign_in_action(),
                );
            }
            Ok(portal::Outcome::SignedIn) => {
                failed_submission_key = None;
                last_attempt_key = Some(attempt_key);
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::Online,
                        "Signed in",
                        format!("Portal access is active on {ssid}."),
                    )
                    .with_sign_in_action(),
                );
            }
            Ok(portal::Outcome::NeedsCredentials { page_url }) => {
                last_attempt_key = Some(attempt_key);
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::NeedsCredentials,
                        "Add your sign-in details",
                        "Save your portal username and password in the app to enable automatic sign-in.",
                    )
                    .with_portal(page_url),
                );
            }
            Ok(portal::Outcome::Manual {
                page_url,
                reason,
                submission_made,
            }) => {
                if submission_made {
                    failed_submission_key = Some(attempt_key.clone());
                }
                last_attempt_key = Some(attempt_key);
                let status =
                    AppStatus::new(StatusKind::Manual, "Sign-in needs your attention", reason)
                        .with_sign_in_action();
                publish(
                    &sender,
                    &mut last_status,
                    page_url.map_or(status.clone(), |url| status.with_portal(url)),
                );
            }
            Err(error) => {
                eprintln!("Could not check the sign-in page: {error:#}");
                last_attempt_key = Some(attempt_key);
                publish(
                    &sender,
                    &mut last_status,
                    AppStatus::new(
                        StatusKind::Error,
                        "Couldn't check the sign-in page",
                        "Check your Wi-Fi connection and try again.",
                    )
                    .with_sign_in_action(),
                );
            }
        }
        wait_for_command(
            &command_receiver,
            NETWORK_STATE_POLL_INTERVAL,
            &mut last_check,
            &mut last_attempt_key,
            &mut force_submission_once,
        );
    }
}

fn wait_for_command(
    receiver: &Receiver<MonitorCommand>,
    timeout: Duration,
    last_check: &mut Instant,
    last_attempt_key: &mut Option<String>,
    force_submission_once: &mut bool,
) {
    if let Ok(mut command) = receiver.recv_timeout(timeout) {
        while let Ok(next) = receiver.try_recv() {
            if next == MonitorCommand::ForceSignIn {
                command = next;
            }
        }
        *last_check = Instant::now() - PORTAL_POLL_INTERVAL;
        *last_attempt_key = None;
        if command == MonitorCommand::ForceSignIn {
            *force_submission_once = true;
        }
    }
}

fn publish(sender: &Sender<AppStatus>, last_status: &mut Option<AppStatus>, status: AppStatus) {
    if last_status.as_ref() != Some(&status) {
        let _ = sender.send(status.clone());
        *last_status = Some(status);
    }
}
