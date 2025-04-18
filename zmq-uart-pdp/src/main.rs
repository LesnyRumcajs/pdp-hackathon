use std::{sync::Arc, time::Duration};

use anyhow::Context;
use parking_lot::Mutex;
use reqwest::Client;
use serde::Deserialize;
use tokio::sync::mpsc;
use zeromq::{Socket as _, SocketRecv as _, SocketSend as _};

#[derive(Deserialize, Debug)]
struct ProofSetRoots {
    data: Vec<ProofSetRoot>,
    metadata: Metadata,
}

#[derive(Deserialize, Debug)]
struct ProofSetRoot {
    root_id: u64,
    cid: String,
    last_proven_epoch: u64,
    last_faulted_epoch: u64,
}

#[derive(Deserialize, Debug)]
struct Metadata {
    total: u64,
    offset: u64,
    limit: u64,
}

#[tokio::main]
async fn main() {
    let port = serialport::new("/dev/ttyACM1", 9_600)
        .timeout(Duration::from_millis(10))
        .open()
        .expect("Failed to open port");

    let mut socket = zeromq::RepSocket::new();
    socket.bind("tcp://127.0.0.1:5555").await.expect("Failed to bind socket");

    // sleep because arduino will restart after opening the port and adding a sleep is less hassle
    // than adding a capacitor to the reset pin.
    // https://forum.arduino.cc/t/autoreset-disabling/350095/4
    std::thread::sleep(Duration::from_secs(2));

    let current_state = Arc::new(Mutex::new(None::<ZmqPayload>));
    let (tx, mut rx) = mpsc::channel(32);
    let http_client = Client::new();

    // Spawn serial port writer task
    let serial_port = Arc::new(Mutex::new(port));
    let serial_port_clone = serial_port.clone();
    tokio::spawn(async move {
        while let Some((filename, status)) = rx.recv().await {
            let message = format!("{},{}\n", filename, status);
            if let Err(e) = serial_port_clone.lock().write_all(message.as_bytes()) {
                eprintln!("Failed to write to serial port: {}", e);
            }
        }
    });

    // Spawn API checking task
    let current_state_clone = current_state.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            
            let state = current_state_clone.lock();
            if let Some(payload) = &*state {
                if payload.stage == Stage::RootsAdded {
                    if let Some(proofset_id) = &payload.data.proofset_id {
                        if let Ok(roots) = check_proof_status(&http_client, proofset_id).await {
                            let status = if roots.data.iter().any(|root| {
                                root.last_proven_epoch > 0 && root.last_proven_epoch < root.last_faulted_epoch
                            }) {
                                "stored & faulty"
                            } else if roots.data.iter().any(|root| root.last_proven_epoch > 0) {
                                "stored & proven"
                            } else {
                                "stored"
                            };
                            
                            if let Err(e) = tx_clone.send((payload.data.file.clone(), status.to_string())).await {
                                eprintln!("Failed to send message through channel: {}", e);
                            }
                        }
                    }
                }
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

        let payload = parse_zmq_msg(&repl).expect(&format!("Failed to parse message: {}", repl));
        
        // Update state and send message through channel if there's a change
        let mut current_state_guard = current_state.lock();
        let should_update = match &*current_state_guard {
            None => true,
            Some(current) => {
                current.stage != payload.stage || current.data.file != payload.data.file
            }
        };

        if should_update {
            let status = match payload.stage {
                Stage::Uploaded => "uploaded",
                Stage::RootsAdded => "stored",
            };
            if let Err(e) = tx.send((payload.data.file.clone(), status.to_string())).await {
                eprintln!("Failed to send message through channel: {}", e);
            }
            *current_state_guard = Some(payload);
        }
    }
}

async fn check_proof_status(client: &Client, proofset_id: &str) -> anyhow::Result<ProofSetRoots> {
    let url = format!(
        "https://calibration.pdp-explorer.eng.filoz.org/api/proofsets/{}/roots?orderBy=root_id&limit=2",
        proofset_id
    );
    let response = client.get(&url).send().await?;
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
    let payload: ZmqPayload = serde_json::from_str(msg)
        .context(format!("Failed to parse message: {}", msg))?;
    
    println!("Received message - File: {} with id: {} and proofset_id: {:?}, Stage: {:?}", 
        payload.data.file, payload.data.file_id, payload.data.proofset_id, payload.stage);
    Ok(payload)
}
