use std::collections::HashSet;
use log::{Level, LevelFilter, info, warn};
use r200_uhf::{Connector, Rfid};
use std::env;
use std::fmt;
use std::io::Write;
use std::thread::sleep;
use std::time::Duration;

#[derive(Debug)]
pub enum AppError {
    Connector(String),
    Serial(String),
    Parse(String),
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AppError::Connector(msg) => write!(f, "Connector error: {}", msg),
            AppError::Serial(msg) => write!(f, "Serial error: {}", msg),
            AppError::Parse(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl std::error::Error for AppError {}

impl From<serialport::Error> for AppError {
    fn from(err: serialport::Error) -> Self {
        AppError::Serial(err.to_string())
    }
}

fn logger_builder(level: LevelFilter) {
    let mut builder = env_logger::Builder::new();
    builder
        .filter_level(level)
        .format(|buf, record| {
            let tm = buf.timestamp();
            let level_string = match record.level() {
                Level::Warn => "⚠️ WARNING",
                Level::Info => "ℹ️ INFO",
                l => l.as_str(),
            };
            writeln!(buf, "T{tm} [{level_string}]: {}", record.args())
        })
        .write_style(env_logger::fmt::WriteStyle::Always)
        .init();
}

/*
TRANSMISSION POWER: 23.6 W
The working range could be between 15 and 26 for this chip.
Having not found a national update that allows a power greater than that provided by
ETSI for 867.9 MHz in that portion (0.5 W ERP),
the safe assumption is that in Europe the limit for 867.9 MHz remains 0.5 W ERP,
as a common technical interpretation.
*/
const POWER_TRANSMISSION: f64 = 23.6;

fn main() -> Result<(), AppError> {
    logger_builder(LevelFilter::Info);

    // use arguments: <port> [baud]
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        warn!(
            "Usage: {} <serial-port> [baud]\nExample: {} /dev/ttyUSB0 115200",
            args[0], args[0]
        );
        std::process::exit(1);
    }
    let port_name = &args[1];
    let baud: u32 = if args.len() > 2 {
        args[2]
            .parse()
            .map_err(|_| AppError::Parse("Invalid baud rate".to_string()))?
    } else {
        115200
    };

    info!("Opening port {} at {} baud...", port_name, baud);
    let port = serialport::new(port_name, baud)
        //  .parity(serialport::Parity::Even)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| AppError::Serial(format!("Failed to open {}: {}", port_name, e)))?;

    let mut connector = Connector::new(port);


    // It's possible that the device was not correct terminated and the multiple polling instruction
    // is enabled. Send a stop.
    connector.stop_multiple_polling_instructions().unwrap();


    info!("{}",connector
        .get_module_info()
        .map_err(|e| AppError::Connector(e.to_string()))?);

    info!(
        "Working area: {:?}",
        connector
            .get_working_area()
            .map_err(|e| AppError::Connector(e.to_string()))?
    );
    info!(
        "Working channel: {:?}",
        connector
            .get_working_channel()
            .map_err(|e| AppError::Connector(e.to_string()))?
    );

    let trasmission_power = connector
        .get_transmit_power()
        .map_err(|e| AppError::Connector(e.to_string()))?;
    info!("Trasmissione power {:?}", trasmission_power);
    if trasmission_power != POWER_TRANSMISSION {
        info!(
            "Set trasmission power {:?}",
            connector
                .set_trasmission_power(POWER_TRANSMISSION)
                .map_err(|e| AppError::Connector(e.to_string()))?
        );
    }

    // Loop with single polling instruction
    /*loop {
        for i in connector
            .single_polling_instruction()
            .map_err(|e| AppError::Connector(e.to_string()))?
        {
            info!("{i}");
        }

        sleep(Duration::from_millis(150));
    }*/

    let mut unique_rfids: HashSet<Rfid> =  HashSet::new();

    // Loop for 10 times with multiple polling instruction
    for sequence in 0..10 {
        for i in connector
            .multi_polling_instruction()
            .map_err(|e| AppError::Connector(e.to_string()))?
        {
            unique_rfids.insert(i.clone());
        }

        println!("|     SEQUENCE: {sequence}   |");
        println!("|     RFID_UNICI     |");
            for rfid in unique_rfids.iter() {
            println!("| {} |", rfid);
        }
        println!("|  TOTAL: {}     |",unique_rfids.len());

    }

    Ok(())
}
