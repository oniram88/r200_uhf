//! A sans-io serial protocol library for R200 UHF RFID readers (M100 / QM100).
//!
//! The crate is organized around a small, dependency-free [`core`] that encodes
//! and decodes the wire protocol, plus transport adapters that drive it over
//! actual I/O:
//!
//! - [`sync::SyncReader`] — blocking, works over any [`std::io::Read`] +
//!   [`std::io::Write`] (e.g. a `serialport` handle). Always available.
//! - [`async_transport::AsyncReader`] — tokio-based, behind the `async` feature.
//!
//! Commands implement the [`Command`](core::command::Command) trait, which pairs
//! a request encoding with a typed response decoding. A [`Frame`] wraps each
//! command on the wire with a header, length, checksum and end marker.
//!
//! # Example
//!
//! ```ignore
//! use std::time::Duration;
//! use r200_uhf::sync::SyncReader;
//! use r200_uhf::{GetModuleInfo, ModuleInfoParam, SinglePollingInstruction};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let port = serialport::new("/dev/ttyUSB0", 115200)
//!     .timeout(Duration::from_millis(500))
//!     .open()?;
//! let mut reader = SyncReader::new(port);
//!
//! let info = reader.send(&GetModuleInfo { param: ModuleInfoParam::SoftwareVersion })?;
//! println!("Firmware: {}", info.text);
//!
//! if let Some(tag) = reader.send(&SinglePollingInstruction)? {
//!     println!("Found: {tag}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Feature flags
//!
//! - `async` — [`async_transport::AsyncReader`] over tokio.
//! - `cli` — the `r200` binary.
//! - `serde` — `Serialize`/`Deserialize` on public data types.

pub mod core;
mod util;

pub mod sync;

#[cfg(feature = "async")]
pub mod async_transport;

#[cfg(feature = "cli")]
pub mod cli;

pub use core::command::{
    GetModuleInfo, GetTransmitPower, GetWorkingArea, GetWorkingChannel, KillTag, LockTag, MemBank,
    ModuleInfoParam, ModuleInfoResponse, MultiplePollingInstruction, ReadLabel, SetSelect,
    SetSendSelect, SetTransmitPower, SetWorkingArea, SinglePollingInstruction, StopMultiplePolling,
    WriteLabel,
};
pub use core::error::{CommandError, CoreError, FrameError};
pub use core::frame::{Frame, FrameType};
pub use core::region::Region;
pub use core::tag::Tag;
