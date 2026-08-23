use std::time::Duration;

use r200_uhf::async_transport::AsyncReader;
use r200_uhf::{GetModuleInfo, ModuleInfoParam, SinglePollingInstruction};
use tokio_serial::SerialPortBuilderExt;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <serial-port> [baud]", args[0]);
        std::process::exit(1);
    }
    let port_name = &args[1];
    let baud: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(115200);

    let port = tokio_serial::new(port_name, baud)
        .open_native_async()
        .expect("open port");

    let mut reader = AsyncReader::new(port);

    let info = reader
        .send(&GetModuleInfo {
            param: ModuleInfoParam::SoftwareVersion,
        })
        .await
        .expect("module info");
    println!("Firmware: {}", info.text);

    println!("Polling...");
    loop {
        match reader.send(&SinglePollingInstruction).await.expect("poll") {
            Some(tag) => {
                println!("Found: {tag}");
                break;
            }
            None => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}
