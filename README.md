# r200_uhf

Rust library for the [R200 UHF RFID reader](https://www.aliexpress.com/item/4000281733851.html) family (M100, QM100).

## Quick start

```toml
[dependencies]
r200_uhf = "0.7"
serialport = "4"
```

```rust
use std::time::Duration;
use r200_uhf::sync::SyncReader;
use r200_uhf::{GetModuleInfo, ModuleInfoParam, SinglePollingInstruction};

let port = serialport::new("/dev/ttyUSB0", 115200)
    .timeout(Duration::from_millis(500))
    .open()?;

let mut reader = SyncReader::new(port);

let info = reader.send(&GetModuleInfo {
    param: ModuleInfoParam::SoftwareVersion,
})?;
println!("Firmware: {}", info.text);

if let Some(tag) = reader.send(&SinglePollingInstruction)? {
    println!("Found: {tag}");
}
```

### Async

```toml
r200_uhf = { version = "0.7", features = ["async"] }
```

```rust
use r200_uhf::async_transport::AsyncReader;

let port = tokio_serial::new("/dev/ttyUSB0", 115200)
    .open_native_async()?;
let mut reader = AsyncReader::new(port);

let tag = reader.send(&SinglePollingInstruction).await?;
```

### Serde

Enable the `serde` feature to derive `Serialize` / `Deserialize` on `Tag`, `Region`, `MemBank`, and other public types:

```toml
r200_uhf = { version = "0.7", features = ["serde"] }
```

## CLI

The `r200` binary provides a command-line interface, gated behind the `cli` feature.

### Install

```sh
cargo install r200_uhf --features cli
```

Or run directly:

```sh
cargo run --features cli --bin r200 -- --port /dev/ttyUSB0 info
```

### Usage

Set the port once via environment:

```sh
export R200_PORT=/dev/ttyUSB0

r200 info                                  # module info, region, channel, power
r200 poll                                  # wait for a tag
r200 scan                                  # continuous scan (Ctrl+C to stop)
r200 scan --no-stop                        # scan without sending stop command on exit
r200 stop-scan                             # stop a running scan
r200 read --bank 1 --addr 0 --length 6     # read 6 words from EPC bank
r200 write E28069150000501D63E2784F        # write a new EPC
r200 writemem AA --bank 3 --addr 0         # write to User bank
r200 lock --lock-data 020080               # lock User bank
r200 region eu                             # set region
r200 power                                 # show power
r200 power 26.5                            # set power (dBm)
```

Run `r200 --help` or `r200 <command> --help` for full options.

## Feature flags

| Feature | Default | Description |
|---------|---------|-------------|
| `async` | off | `AsyncReader` over tokio `AsyncRead+AsyncWrite` |
| `cli` | off | `r200` binary (clap, serialport, ctrlc) |
| `serde` | off | `Serialize`/`Deserialize` on public types |

The `SyncReader` is always available (no external dependencies).

## Supported commands

| Command | Code | Description |
|---------|------|-------------|
| `GetModuleInfo` | 0x03 | Hardware/software version, manufacturer |
| `SinglePollingInstruction` | 0x22 | Read one tag from the RF field |
| `MultiplePollingInstruction` | 0x27 | Continuous inventory |
| `StopMultiplePolling` | 0x28 | Stop continuous inventory |
| `SetSelect` | 0x0C | Set tag select mask |
| `SetSendSelect` | 0x12 | Enable/disable select |
| `ReadLabel` | 0x39 | Read tag memory |
| `WriteLabel` | 0x49 | Write tag memory |
| `KillTag` | 0x65 | Kill a tag |
| `LockTag` | 0x82 | Lock memory banks |
| `GetWorkingArea` | 0x08 | Get RF region |
| `SetWorkingArea` | 0x07 | Set RF region |
| `GetWorkingChannel` | 0xAA | Get current channel |
| `GetTransmitPower` | 0xB7 | Get power (dBm) |
| `SetTransmitPower` | 0xB6 | Set power (dBm) |

## Legal and safety note

Transmission power and permitted frequencies vary by country/region. Ensure compliance with your local regulations and adjust it responsibly.

## License

MIT License. See LICENSE for details.
