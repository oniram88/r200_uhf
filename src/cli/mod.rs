use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::core::command::Command;

mod display;

/// Parse a hex string (with or without `0x` prefix) into bytes.
///
/// # Errors
///
/// Returns an error if the string has odd length or contains non-hex characters.
pub fn parse_hex(s: &str) -> Result<Vec<u8>> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    if !s.len().is_multiple_of(2) {
        anyhow::bail!("hex string must have even length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16))
        .collect::<std::result::Result<Vec<u8>, _>>()
        .context("invalid hex character")
}

#[derive(Parser)]
#[command(name = "r200", about = "R200 UHF RFID reader")]
struct Cli {
    #[arg(long, env = "R200_PORT")]
    port: String,

    #[arg(long, default_value = "115200")]
    baud: u32,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Show module info, region, channel, power
    Info,
    /// Poll for a single tag
    Poll,
    /// Continuous scan with deduplication (Ctrl+C to stop)
    Scan {
        /// Skip sending `StopMultiplePolling` on exit
        #[arg(long)]
        no_stop: bool,
    },
    /// Stop a running multi-polling scan
    StopScan,
    /// Read memory from a tag
    Read {
        /// Memory bank: 0=Reserved, 1=EPC, 2=TID, 3=User
        #[arg(long, default_value = "1")]
        bank: u8,
        /// Word address to start reading from
        #[arg(long, default_value = "0")]
        addr: u16,
        /// Number of 16-bit words to read
        #[arg(long, default_value = "6")]
        length: u16,
        /// Only read tag matching this EPC (hex)
        #[arg(long)]
        select: Option<String>,
    },
    /// Write a new EPC to a tag
    Write {
        /// New EPC in hex (24 hex chars = 12 bytes)
        epc: String,
        /// Only write to tag matching this EPC (hex)
        #[arg(long)]
        select: Option<String>,
    },
    /// Write data to any memory bank
    WriteMem {
        /// Data bytes in hex (even length)
        data: String,
        /// Memory bank: 0=Reserved, 1=EPC, 2=TID, 3=User
        #[arg(long, default_value = "0")]
        bank: u8,
        /// Word address to start writing at
        #[arg(long, default_value = "0")]
        addr: u16,
        /// Only write to tag matching this EPC (hex)
        #[arg(long)]
        select: Option<String>,
    },
    /// Lock a tag's memory banks
    Lock {
        /// Access password in hex (8 chars, default 00000000)
        #[arg(long, default_value = "00000000")]
        password: String,
        /// Lock data in hex (6 chars, e.g. 020080 to lock User bank)
        #[arg(long, default_value = "020080")]
        lock_data: String,
        /// Only lock tag matching this EPC (hex)
        #[arg(long)]
        select: Option<String>,
    },
    /// Get or set the RF region
    Region {
        /// Region to set: china900, china800, eu, us, korea
        area: Option<String>,
    },
    /// Get or set transmit power (dBm)
    Power {
        /// Power level in dBm (omit to display current)
        level: Option<f64>,
    },
}

type Reader = crate::sync::SyncReader<Box<dyn serialport::SerialPort>>;

fn wait_for_tag(reader: &mut Reader) -> Result<Vec<u8>> {
    eprint!("Place a tag on the antenna... ");
    loop {
        if let Ok(Some(tag)) = reader.send(&crate::SinglePollingInstruction) {
            eprintln!("found {}", tag.epc_hex());
            return Ok(tag.epc);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn select_tag(reader: &mut Reader, epc: &[u8]) -> Result<()> {
    reader.send(&crate::SetSelect::by_epc(epc))?;
    reader.send(&crate::SetSendSelect(true))?;
    Ok(())
}

fn clear_select(reader: &mut Reader) -> Result<()> {
    reader.send(&crate::SetSendSelect(false))?;
    reader.send(&crate::SetSelect::by_epc(&[]))?;
    Ok(())
}

fn stop_scan(reader: &mut Reader) -> Result<()> {
    reader.send_only(&crate::StopMultiplePolling)?;
    let _ = reader.recv();
    Ok(())
}

fn resolve_select_epc(select: Option<&String>) -> Result<Option<Vec<u8>>> {
    match select {
        Some(s) => Ok(Some(parse_hex(s)?)),
        None => Ok(None),
    }
}

fn cmd_info(reader: &mut Reader) -> Result<()> {
    let hw = reader.send(&crate::GetModuleInfo {
        param: crate::ModuleInfoParam::HardwareVersion,
    })?;
    let sw = reader.send(&crate::GetModuleInfo {
        param: crate::ModuleInfoParam::SoftwareVersion,
    })?;
    let mfr = reader.send(&crate::GetModuleInfo {
        param: crate::ModuleInfoParam::Manufacturer,
    })?;
    let region = reader.send(&crate::GetWorkingArea)?;
    let channel = reader.send(&crate::GetWorkingChannel)?;
    let power = reader.send(&crate::GetTransmitPower)?;

    let freq = region.channel_frequency(channel);

    println!("Hardware:      {}", hw.text);
    println!("Firmware:      {}", sw.text);
    println!("Manufacturer:  {}", mfr.text);
    println!("Region:        {region}");
    println!("Channel:       {channel} ({freq:.2} MHz)");
    println!("Power:         {power:.1} dBm");
    Ok(())
}

fn cmd_poll(reader: &mut Reader) {
    println!("Polling for tags...");
    loop {
        if let Ok(Some(tag)) = reader.send(&crate::SinglePollingInstruction) {
            display::display_tag(&tag);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn cmd_scan(reader: &mut Reader, no_stop: bool) -> Result<()> {
    println!("Scanning... (Ctrl+C to stop)");
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .context("setting up Ctrl+C handler")?;

    let mut seen = HashSet::new();
    while running.load(Ordering::SeqCst) {
        let _ = reader.send_only(&crate::MultiplePollingInstruction { pool_times: 100 });

        loop {
            match reader.recv() {
                Ok(frame) if frame.command_code == 0x22 => {
                    if let Ok(Some(tag)) =
                        crate::SinglePollingInstruction.decode_response(&frame.data)
                    {
                        if seen.insert(tag.epc_hex()) {
                            display::display_tag(&tag);
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    if !no_stop {
        stop_scan(reader)?;
    }
    println!("Scan stopped.");
    Ok(())
}

fn cmd_read(
    reader: &mut Reader,
    bank: u8,
    addr: u16,
    length: u16,
    select: Option<&String>,
) -> Result<()> {
    let mem_bank = crate::MemBank::from_byte(bank)
        .ok_or_else(|| anyhow::anyhow!("invalid bank {bank}, must be 0-3"))?;

    let select_epc = resolve_select_epc(select)?;

    if let Some(ref epc) = select_epc {
        select_tag(reader, epc)?;
    } else {
        let _ = wait_for_tag(reader)?;
    }

    println!("Reading {length} words from {mem_bank} bank, addr 0x{addr:04X}...");

    let data = reader.send(&crate::ReadLabel {
        access_password: [0x00; 4],
        bank: mem_bank,
        address: addr,
        length,
    })?;

    if select_epc.is_some() {
        clear_select(reader)?;
    }

    let hex_str = crate::util::hex_upper(&data);
    println!("Hex: {hex_str}");
    Ok(())
}

fn cmd_write(reader: &mut Reader, epc: &str, select: Option<&String>) -> Result<()> {
    let epc_bytes = parse_hex(epc)?;
    if epc_bytes.len() != 12 {
        anyhow::bail!("EPC must be exactly 24 hex characters (12 bytes)");
    }

    let select_epc = resolve_select_epc(select)?;

    let current_epc = wait_for_tag(reader)?;
    let target = select_epc.as_ref().unwrap_or(&current_epc);
    select_tag(reader, target)?;

    println!("Writing EPC: {}", display::hex(&epc_bytes));
    reader.send(&crate::WriteLabel {
        access_password: [0x00; 4],
        bank: crate::MemBank::Epc,
        address: 0,
        data: epc_bytes.clone(),
    })?;

    clear_select(reader)?;

    let verify_epc = wait_for_tag(reader)?;
    if verify_epc == epc_bytes {
        println!("Verified: EPC written correctly");
    } else {
        println!(
            "Warning: read back {} expected {}",
            display::hex(&verify_epc),
            display::hex(&epc_bytes)
        );
    }
    Ok(())
}

fn cmd_write_mem(
    reader: &mut Reader,
    data: &str,
    bank: u8,
    addr: u16,
    select: Option<&String>,
) -> Result<()> {
    let data_bytes = parse_hex(data)?;
    if data_bytes.is_empty() || !data_bytes.len().is_multiple_of(2) {
        anyhow::bail!("data must be a non-empty even number of hex characters");
    }

    let mem_bank = crate::MemBank::from_byte(bank)
        .ok_or_else(|| anyhow::anyhow!("invalid bank {bank}, must be 0-3"))?;

    let select_epc = resolve_select_epc(select)?;

    let current_epc = wait_for_tag(reader)?;
    let target = select_epc.as_ref().unwrap_or(&current_epc);
    select_tag(reader, target)?;

    println!(
        "Writing {} bytes to {mem_bank} bank, addr 0x{addr:04X}...",
        data_bytes.len(),
    );

    reader.send(&crate::WriteLabel {
        access_password: [0x00; 4],
        bank: mem_bank,
        address: addr,
        data: data_bytes,
    })?;

    clear_select(reader)?;
    println!("Done");
    Ok(())
}

fn cmd_lock(
    reader: &mut Reader,
    password: &str,
    lock_data: &str,
    select: Option<&String>,
) -> Result<()> {
    let pwd_bytes = parse_hex(password)?;
    if pwd_bytes.len() != 4 {
        anyhow::bail!("password must be exactly 8 hex characters (4 bytes)");
    }

    let ld_bytes = parse_hex(lock_data)?;
    if ld_bytes.len() != 3 {
        anyhow::bail!("lock-data must be exactly 6 hex characters (3 bytes)");
    }

    let select_epc = resolve_select_epc(select)?;

    let current_epc = wait_for_tag(reader)?;
    let target = select_epc.as_ref().unwrap_or(&current_epc);
    select_tag(reader, target)?;

    let mut lock_data_arr = [0u8; 3];
    lock_data_arr.copy_from_slice(&ld_bytes);
    println!("Locking tag (lock_data={lock_data})...");

    let mut pwd_arr = [0u8; 4];
    pwd_arr.copy_from_slice(&pwd_bytes);

    reader.send(&crate::LockTag {
        password: pwd_arr,
        lock_data: lock_data_arr,
    })?;

    clear_select(reader)?;
    println!("Tag locked");
    Ok(())
}

fn cmd_region(reader: &mut Reader, area: Option<&String>) -> Result<()> {
    match area {
        None => {
            let region = reader.send(&crate::GetWorkingArea)?;
            let channel = reader.send(&crate::GetWorkingChannel)?;
            let freq = region.channel_frequency(channel);
            println!("Region:  {region}");
            println!("Channel: {channel} ({freq:.2} MHz)");
        }
        Some(area_str) => {
            let region = match area_str.to_lowercase().as_str() {
                "china900" | "cn900" => crate::Region::China900Mhz,
                "china800" | "cn800" => crate::Region::China800Mhz,
                "eu" => crate::Region::Eu,
                "us" => crate::Region::Us,
                "korea" | "kr" => crate::Region::Korea,
                _ => anyhow::bail!(
                    "unknown region: {area_str} (valid: china900, china800, eu, us, korea)"
                ),
            };
            println!("Setting region to {region}...");
            reader.send(&crate::SetWorkingArea(region))?;
            println!("Done");
        }
    }
    Ok(())
}

fn cmd_power(reader: &mut Reader, level: Option<&f64>) -> Result<()> {
    match level {
        None => {
            let power = reader.send(&crate::GetTransmitPower)?;
            println!("Power: {power:.1} dBm");
        }
        Some(dbm) => {
            if !(0.0..=30.0).contains(dbm) {
                anyhow::bail!("power must be between 0.0 and 30.0 dBm");
            }
            println!("Setting power to {dbm:.1} dBm...");
            reader.send(&crate::SetTransmitPower(*dbm))?;
            let verify = reader.send(&crate::GetTransmitPower)?;
            println!("Power set to {verify:.1} dBm");
        }
    }
    Ok(())
}

/// Run the CLI application.
///
/// # Errors
///
/// Returns an error if the serial port cannot be opened, a command fails,
/// or the user provides invalid arguments.
pub fn run() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let port = serialport::new(&cli.port, cli.baud)
        .timeout(Duration::from_millis(500))
        .open()
        .context("failed to open serial port")?;

    let mut reader = crate::sync::SyncReader::new(port);

    match cli.command {
        Commands::Info => cmd_info(&mut reader)?,
        Commands::Poll => cmd_poll(&mut reader),
        Commands::Scan { no_stop } => cmd_scan(&mut reader, no_stop)?,
        Commands::StopScan => {
            stop_scan(&mut reader)?;
            println!("Scan stopped.");
        }
        Commands::Read {
            bank,
            addr,
            length,
            select,
        } => cmd_read(&mut reader, bank, addr, length, select.as_ref())?,
        Commands::Write { epc, select } => cmd_write(&mut reader, &epc, select.as_ref())?,
        Commands::WriteMem {
            data,
            bank,
            addr,
            select,
        } => cmd_write_mem(&mut reader, &data, bank, addr, select.as_ref())?,
        Commands::Lock {
            password,
            lock_data,
            select,
        } => cmd_lock(&mut reader, &password, &lock_data, select.as_ref())?,
        Commands::Region { area } => cmd_region(&mut reader, area.as_ref())?,
        Commands::Power { level } => cmd_power(&mut reader, level.as_ref())?,
    }

    Ok(())
}
