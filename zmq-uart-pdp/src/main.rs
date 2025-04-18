use std::{sync::Arc, time::Duration};

use anyhow::Context;
use parking_lot::Mutex;
use zeromq::{Socket as _, SocketRecv as _, SocketSend as _};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut port = serialport::new("/dev/ttyACM1", 9_600)
        .timeout(Duration::from_millis(10))
        .open()
        .expect("Failed to open port");

    let mut socket = zeromq::RepSocket::new();
    socket.bind("tcp://127.0.0.1:5555").await?;

    // sleep because arduino will restart after opening the port and adding a sleep is less hassle
    // than adding a capacitor to the reset pin.
    // https://forum.arduino.cc/t/autoreset-disabling/350095/4
    std::thread::sleep(Duration::from_secs(2));
    // write funny name with newline
    //port.write_all(b"Hello, world")
    //    .expect("Failed to write to port");

    let current_file = Arc::new(Mutex::new(FileData::default()));

    loop {
        let repl: String = socket
            .recv()
            .await
            .context("Failed to receive message")?
            .try_into()
            .unwrap();
        socket.send("ACK".into()).await?;

        let msg = parse_zmq_msg(&repl).context(format!("Failed to parse message: {}", repl))?;
    }

    Ok(())
}

#[derive(serde::Deserialize, Debug, Default)]
struct FileData {
    file: String,
    file_id: String,
    proofset_id: Option<String>,
}

#[derive(serde::Deserialize, Debug)]
struct ZmqPayload {
    stage: Stage,
    data: FileData,
}

#[derive(serde::Deserialize, Debug)]
enum Stage {
    Uploaded,
    RootsAdded,
}

// Sample payloads:
//Received: "{\"stage\": \"UPLOADED\", \"data\": {\"file\": \"cathulhu-rise-of.jpg\", \"file_id\": \"baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy:baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy\"}}"
//Received: "{\"stage\": \"ROOTS_ADDED\", \"data\": {\"file\": \"cathulhu-rise-of.jpg\", \"file_id\": \"baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy:baga6ea4seaqa66jndptvpxfbo3qimxysbivikdpqint4t2kvnnkrfxrfoi2nufy\", \"proofset_id\": \"51\"}}"
//
fn parse_zmq_msg(msg: &str) -> anyhow::Result<Stage> {
    let payload: ZmqPayload =
        serde_json::from_str(msg).context(format!("Failed to parse message: {}", msg))?;
    match payload.stage {
        Stage::Uploaded => {
            println!("File uploaded: {:?}", payload.data.file);
            Ok(Stage::Uploaded)
        }
        Stage::RootsAdded => {
            println!("Roots added for file: {:?}", payload.data.file);
            Ok(Stage::RootsAdded)
        }
    }
}
