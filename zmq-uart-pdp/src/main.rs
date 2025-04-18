use std::{sync::Arc, time::Duration};

use anyhow::Context;
use log::{debug, error, info, warn};
use parking_lot::Mutex;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::mpsc;
use zeromq::{Socket as _, SocketRecv as _, SocketSend as _};

const SERIAL_PORT: &str = "/dev/ttyACM1";
const SERIAL_BAUD_RATE: u32 = 9_600;
const SERIAL_TIMEOUT_MS: u64 = 10;
const ZMQ_BIND_ADDRESS: &str = "tcp://127.0.0.1:5555";
const ARDUINO_RESET_DELAY_SECS: u64 = 2;
const API_CHECK_INTERVAL_SECS: u64 = 5;
const API_BASE_URL: &str = "https://calibration.pdp-explorer.eng.filoz.org";
const API_ROOTS_LIMIT: u64 = 100;
const CHANNEL_BUFFER_SIZE: usize = 32;

// These structs must contain all fields from the API response for proper deserialization,
// even if we don't use all fields in our logic.
#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct ProofSetRoots {
    data: Vec<ProofSetRoot>,
    metadata: Metadata,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct ProofSetRoot {
    #[serde(rename = "rootId")]
    root_id: u64,
    cid: String,
    size: u64,
    removed: bool,
    #[serde(rename = "totalPeriodsFaulted")]
    total_periods_faulted: u64,
    #[serde(rename = "totalProofsSubmitted")]
    total_proofs_submitted: u64,
    #[serde(rename = "lastProvenEpoch")]
    last_proven_epoch: u64,
    #[serde(rename = "lastProvenAt")]
    last_proven_at: Option<String>,
    #[serde(rename = "lastFaultedEpoch")]
    last_faulted_epoch: u64,
    #[serde(rename = "lastFaultedAt")]
    last_faulted_at: Option<String>,
    #[serde(rename = "createdAt")]
    created_at: String,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct Metadata {
    total: u64,
    offset: u64,
    limit: u64,
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    info!("Starting arduino-pdp service");

    let port = serialport::new(SERIAL_PORT, SERIAL_BAUD_RATE)
        .timeout(Duration::from_millis(SERIAL_TIMEOUT_MS))
        .open()
        .expect("Failed to open port");

    let mut socket = zeromq::RepSocket::new();
    socket
        .bind(ZMQ_BIND_ADDRESS)
        .await
        .expect("Failed to bind socket");

    // sleep because arduino will restart after opening the port and adding a sleep is less hassle
    // than adding a capacitor to the reset pin.
    // https://forum.arduino.cc/t/autoreset-disabling/350095/4
    std::thread::sleep(Duration::from_secs(ARDUINO_RESET_DELAY_SECS));

    let current_state = Arc::new(Mutex::new(None::<ZmqPayload>));
    let (tx, mut rx) = mpsc::channel(CHANNEL_BUFFER_SIZE);
    let http_client = Client::new();

    // Spawn serial port writer task
    let serial_port = Arc::new(Mutex::new(port));
    let serial_port_clone = serial_port.clone();
    tokio::spawn(async move {
        while let Some((filename, status)) = rx.recv().await {
            let message = format!("{},{}\n", filename, status);
            if let Err(e) = serial_port_clone.lock().write_all(message.as_bytes()) {
                error!("Failed to write to serial port: {}", e);
            }
        }
    });

    // Spawn API checking task
    let current_state_clone = current_state.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        info!("API checking task started");
        loop {
            tokio::time::sleep(Duration::from_secs(API_CHECK_INTERVAL_SECS)).await;
            debug!("Checking API...");

            let state_data = {
                let state = current_state_clone.lock();
                debug!("Current state: {:?}", *state);
                if let Some(payload) = &*state {
                    if payload.stage == Stage::RootsAdded {
                        debug!("Stage is RootsAdded, checking proofset_id");
                        // Extract the second part of the file_id (after the colon)
                        let root_cid = payload.data.file_id.split(':').nth(1);
                        if let Some(cid) = root_cid {
                            debug!("Found root CID: {}", cid);
                        } else {
                            warn!("No root CID found in file_id: {}", payload.data.file_id);
                        }
                        payload.data.proofset_id.as_ref().and_then(|id| {
                            root_cid
                                .map(|cid| (id.clone(), payload.data.file.clone(), cid.to_string()))
                        })
                    } else {
                        debug!("Stage is not RootsAdded: {:?}", payload.stage);
                        None
                    }
                } else {
                    debug!("No state set yet");
                    None
                }
            };

            if let Some((proofset_id, filename, root_cid)) = state_data {
                info!("Making API request for proofset_id: {}", proofset_id);
                if let Ok(roots) = check_proof_status(&http_client, &proofset_id).await {
                    debug!("Found {} total roots", roots.data.len());
                    debug!("Looking for CID: {}", root_cid);

                    // Find all roots that match our CID and have epochs set
                    let relevant_roots: Vec<_> = roots
                        .data
                        .iter()
                        .filter(|root| {
                            let matches = root.cid == root_cid;
                            if matches {
                                debug!(
                                    "Found matching root: proven={}, faulted={}",
                                    root.last_proven_epoch, root.last_faulted_epoch
                                );
                            }
                            matches
                        })
                        .filter(|root| {
                            let has_epochs =
                                root.last_proven_epoch > 0 || root.last_faulted_epoch > 0;
                            if has_epochs {
                                debug!(
                                    "Root has epochs set: proven={}, faulted={}",
                                    root.last_proven_epoch, root.last_faulted_epoch
                                );
                            }
                            has_epochs
                        })
                        .collect();

                    debug!("Found {} relevant roots", relevant_roots.len());

                    if !relevant_roots.is_empty() {
                        // If any root is faulty, the status is faulty
                        let status = if relevant_roots.iter().any(|root| {
                            root.last_proven_epoch > 0
                                && root.last_proven_epoch < root.last_faulted_epoch
                        }) {
                            "stored & faulty"
                        } else if relevant_roots.iter().any(|root| root.last_proven_epoch > 0) {
                            "stored & proven"
                        } else {
                            "stored"
                        };

                        info!("Setting status to: {}", status);
                        if let Err(e) = tx_clone.send((filename, status.to_string())).await {
                            error!("Failed to send message through channel: {}", e);
                        }
                    } else {
                        // If we found matching roots but none have epochs set, keep status as "stored"
                        if roots.data.iter().any(|root| root.cid == root_cid) {
                            debug!("Found matching roots but none have epochs set");
                            if let Err(e) = tx_clone.send((filename, "stored".to_string())).await {
                                error!("Failed to send message through channel: {}", e);
                            }
                        } else {
                            warn!("Could not find root with CID: {}", root_cid);
                        }
                    }
                } else {
                    error!("Failed to get roots from API");
                }
            } else {
                debug!("No state data available for API check");
            }
        }
    });

    loop {
        let repl: String = socket
            .recv()
            .await
            .expect("Failed to receive message")
            .try_into()
            .unwrap();
        socket.send("ACK".into()).await.expect("Failed to send ACK");

        let payload =
            parse_zmq_msg(&repl).unwrap_or_else(|_| panic!("Failed to parse message: {}", repl));

        // Update state and send message through channel if there's a change
        let should_update = {
            let current_state_guard = current_state.lock();
            match &*current_state_guard {
                None => true,
                Some(current) => {
                    current.stage != payload.stage || current.data.file != payload.data.file
                }
            }
        };

        if should_update {
            let status = match payload.stage {
                Stage::Uploaded => "uploaded",
                Stage::RootsAdded => "stored",
            };
            info!("State changed, sending status: {}", status);
            if let Err(e) = tx
                .send((payload.data.file.clone(), status.to_string()))
                .await
            {
                error!("Failed to send message through channel: {}", e);
            }
            let mut current_state_guard = current_state.lock();
            *current_state_guard = Some(payload);
        }
    }
}

async fn check_proof_status(client: &Client, proofset_id: &str) -> anyhow::Result<ProofSetRoots> {
    let url = format!(
        "{}/api/proofsets/{}/roots?orderBy=root_id&limit={}",
        API_BASE_URL, proofset_id, API_ROOTS_LIMIT
    );
    debug!("Requesting URL: {}", url);
    let response = client.get(&url).send().await?;
    let status = response.status();
    debug!("Response status: {}", status);
    if !status.is_success() {
        let error_text = response.text().await?;
        error!("Error response: {}", error_text);
        anyhow::bail!("API request failed with status: {}", status);
    }
    let roots = response.json().await?;
    Ok(roots)
}

#[derive(serde::Deserialize, Debug, Default, PartialEq, Clone)]
struct FileData {
    file: String,
    file_id: String,
    proofset_id: Option<String>,
}

#[derive(serde::Deserialize, Debug, PartialEq, Clone)]
struct ZmqPayload {
    stage: Stage,
    data: FileData,
}

#[derive(serde::Deserialize, Debug, PartialEq, Clone)]
enum Stage {
    Uploaded,
    RootsAdded,
}

// Sample payloads:
//Received: "{\"stage\": \"UPLOADED\", \"data\": {\"file\": \"cathulhu-rise-of.jpg\", \"file_id\": \"baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy:baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy\"}}"
//Received: "{\"stage\": \"ROOTS_ADDED\", \"data\": {\"file\": \"cathulhu-rise-of.jpg\", \"file_id\": \"baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy:baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy\", \"proofset_id\": \"51\"}}"
//
fn parse_zmq_msg(msg: &str) -> anyhow::Result<ZmqPayload> {
    let payload: ZmqPayload =
        serde_json::from_str(msg).context(format!("Failed to parse message: {}", msg))?;

    info!(
        "Received message - File: {} with id: {} and proofset_id: {:?}, Stage: {:?}",
        payload.data.file, payload.data.file_id, payload.data.proofset_id, payload.stage
    );
    Ok(payload)
}
