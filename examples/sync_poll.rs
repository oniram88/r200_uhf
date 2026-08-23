use std::time::Duration;

use r200_uhf::sync::SyncReader;
use r200_uhf::{GetModuleInfo, ModuleInfoParam, SinglePollingInstruction};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <serial-port> [baud]", args[0]);
        std::process::exit(1);
    }
    let port_name = &args[1];
    let baud: u32 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(115200);

    let port = serialport::new(port_name, baud)
        .timeout(Duration::from_millis(500))
        .open()
        .expect("open port");

    let mut reader = SyncReader::new(port);

    let info = reader
        .send(&GetModuleInfo {
            param: ModuleInfoParam::SoftwareVersion,
        })
        .expect("module info");
    println!("Firmware: {}", info.text);

    println!("Polling...");
    loop {
        match reader.send(&SinglePollingInstruction).expect("poll") {
            Some(tag) => {
                println!("Found: {tag}");
                break;
            }
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}
