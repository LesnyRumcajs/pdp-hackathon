# Arduino PDP Status Display

A Rust service that listens for ZMQ messages about PDP (Proof of Data Possession) status updates and displays them on an Arduino LCD screen.

## Features

- Listens for ZMQ messages on `tcp://127.0.0.1:5555`
- Communicates with Arduino via serial port
- Tracks file upload and proof status
- Queries PDP Explorer API for proof status
- Supports multiple status states:
  - `uploaded`
  - `stored`
  - `stored & proven`
  - `stored & faulty`

## Requirements

- Rust 1.85.0 or later
- Arduino with LCD display
- Serial port access
- ZMQ message source
- PDP Explorer API access

Note! You might need to run `sudo chmod a+rw /dev/ttyACM1` if you the program is not able to open the port.

## Configuration

The constants can be adjusted in `src/main.rs`.

## Running

```bash
cargo run
```

## Message Format

The service expects ZMQ messages in the following JSON format:

```json
{
  "stage": "UPLOADED" | "ROOTS_ADDED",
  "data": {
    "file": "filename.ext",
    "file_id": "baga...:baga...",
    "proofset_id": "123"  // Only present in ROOTS_ADDED stage
  }
}
```

## Arduino Communication

The service sends messages to the Arduino in the format:
```
filename.ext,status\n
```

Where status is one of:
- `uploaded`
- `stored`
- `stored & proven`
- `stored & faulty` 