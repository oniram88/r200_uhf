use log::{Level, LevelFilter, warn};
use std::io::Write;
use std::{env, fmt};

/*
TRANSMISSION POWER: 23.6 W
The working range could be between 15 and 26 for this chip.
Having not found a national update that allows a power greater than that provided by
ETSI for 867.9 MHz in that portion (0.5 W ERP),
the safe assumption is that in Europe the limit for 867.9 MHz remains 0.5 W ERP,
as a common technical interpretation.
*/
pub const POWER_TRANSMISSION: f64 = 23.6;

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

pub(crate) fn logger_builder(level: LevelFilter) {
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

pub fn get_args() -> Result<(String, u32, f64), AppError> {
    // use arguments: <port> [baud]
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        warn!(
            "Usage: {} <serial-port> [baud [power]]\nExample: {} /dev/ttyUSB0 115200",
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
    let power: f64 = if args.len() == 4 {
        args[3]
            .parse()
            .map_err(|_| AppError::Parse("Invalid power 15 -> 23.6".to_string()))?
    } else {
        POWER_TRANSMISSION
    };

    Ok((port_name.to_string(), baud, power))
}
