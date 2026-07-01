use log::{LevelFilter, error, info};
use serialport::{DataBits, FlowControl, Parity, StopBits};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::sleep;
use tokio_serial::SerialPortBuilderExt;

#[path = "../examples/lib/common.rs"]
mod common;
use crate::common::{AppError, get_args};
use common::logger_builder;
use r200_uhf::Rfid;
use r200_uhf::connector::{AsyncIO, Connector};

#[allow(unreachable_code)]
#[tokio::main]
async fn main() -> Result<(), AppError> {
    logger_builder(LevelFilter::Info);

    let (port_name, baud, power) = get_args()?;

    info!("Opening port {} at {} baud...", port_name, baud);
    let port = tokio_serial::new(&port_name, baud)
        .data_bits(DataBits::Eight)
        .parity(Parity::None)
        .stop_bits(StopBits::One)
        .flow_control(FlowControl::None)
        .open_native_async()?;

    let mut connector = Connector::new(port);

    // It's possible that the device was not correct terminated and the multiple polling instruction
    // is enabled. Send a stop.
    loop {
        if connector
            .stop_multiple_polling_instructions()
            .await
            .is_err()
        {
            error!("FAIL: Connector stop multiple polling");
            sleep(Duration::from_millis(500)).await;
        } else {
            break;
        }
    }

    info!(
        "{}",
        connector
            .get_module_info()
            .await
            .map_err(|e| AppError::Connector(e.to_string()))?
    );

    info!(
        "Working area: {:?}",
        connector
            .get_working_area()
            .await
            .map_err(|e| AppError::Connector(e.to_string()))?
    );
    info!(
        "Working channel: {:?}",
        connector
            .get_working_channel()
            .await
            .map_err(|e| AppError::Connector(e.to_string()))?
    );

    let transmission_power = connector
        .get_transmit_power()
        .await
        .map_err(|e| AppError::Connector(e.to_string()))?;
    info!("Transmission power {:?}", transmission_power);
    if transmission_power != power {
        info!(
            "Set transmission power {:?}",
            connector
                .set_transmission_power(power)
                .await
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
            .await
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
