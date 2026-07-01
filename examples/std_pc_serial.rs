use log::{LevelFilter, error, info};
use r200_uhf::Rfid;
use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

#[path = "../examples/lib/common.rs"]
mod common;
use crate::common::{AppError, get_args};
use common::logger_builder;
use r200_uhf::connector::Connector;
use r200_uhf::connector::sync::SyncIO;

fn main() -> Result<(), AppError> {
    logger_builder(LevelFilter::Info);

    let (port_name, baud, power) = get_args().unwrap();

    info!("Opening port {} at {} baud...", port_name, baud);
    let port = serialport::new(&port_name, baud)
        //  .parity(serialport::Parity::Even)
        .timeout(Duration::from_millis(500))
        .open()
        .map_err(|e| AppError::Serial(format!("Failed to open {}: {}", port_name, e)))?;

    let mut connector = Connector::new(port);

    // It's possible that the device was not correct terminated and the multiple polling instruction
    // is enabled. Send a stop.
    loop {
        if connector.stop_multiple_polling_instructions().is_err() {
            error!("FAIL: Connector stop multiple polling");
            sleep(Duration::from_millis(500));
        } else {
            break;
        }
    }

    info!(
        "{}",
        connector
            .get_module_info()
            .map_err(|e| AppError::Connector(e.to_string()))?
    );

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
    if trasmission_power != power {
        info!(
            "Set trasmission power {:?}",
            connector
                .set_transmission_power(power)
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

    let mut unique_rfids: HashSet<Rfid> = HashSet::new();

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
        println!("|  TOTAL: {}     |", unique_rfids.len());
    }

    Ok(())
}
