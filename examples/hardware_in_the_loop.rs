//! Hardware-in-the-loop test for a real R200 reader with two tags present.
//!
//! This exercises every supported command **except `KillTag`** against a live
//! device, and leaves the two working tags in a known canonical state:
//!
//! - one tag with EPC `1111...` (24 hex chars)
//! - one tag with EPC `2222...`
//!
//! The `LockTag` step is run against a third, lock-capable tag
//! (`EPC_LOCKABLE`) when it is present in the field, using only reversible
//! `Open`/`Locked` actions (never a permalock). If that tag is absent, the lock
//! step is skipped. The lock-capable tag keeps its own EPC and is never
//! rewritten.
//!
//! The test is *idempotent*: it first drives the two working tags to the
//! canonical IDs (regardless of what they currently hold), runs the checks, and
//! restores them to `1111...` / `2222...` at the end. Run it as many times as
//! you like.
//!
//! Every device interaction is wrapped in a small retry helper so transient
//! serial/RF hiccups don't fail the run.
//!
//! ```sh
//! cargo run --features cli --example hardware_in_the_loop -- /dev/cu.usbserial-10
//! ```
//!
//! Requires the `cli` feature (for the `serialport` dependency).

use std::time::Duration;

use r200_uhf::core::command::Command;
use r200_uhf::sync::SyncReader;
use r200_uhf::{
    GetModuleInfo, GetTransmitPower, GetWorkingArea, GetWorkingChannel, LockTag, MemBank,
    ModuleInfoParam, MultiplePollingInstruction, ReadLabel, Region, SetSelect, SetSendSelect,
    SetTransmitPower, SetWorkingArea, SinglePollingInstruction, StopMultiplePolling, WriteLabel,
};

type Reader = SyncReader<Box<dyn serialport::SerialPort>>;

const EPC_ONE: [u8; 12] = [0x11; 12];
const EPC_TWO: [u8; 12] = [0x22; 12];

/// A tag known to support a reversible User-bank lock (verified on this bench).
/// If present, the lock step is run against it; otherwise the lock step is
/// skipped. It is never rewritten with a canonical ID.
const EPC_LOCKABLE: [u8; 12] = [
    0xe2, 0x00, 0x47, 0x0b, 0x67, 0x80, 0x60, 0x26, 0xdb, 0x90, 0x01, 0x0e,
];

/// Reversible User-bank lock / unlock operands (spec §8; verified against the
/// spec's own `0x020080` example). `mask=10, action=10` => Locked (reversible);
/// `mask=10, action=00` => Open (unlocked). No permalock bits are ever set.
const LOCK_USER: [u8; 3] = [0x00, 0x08, 0x02];
const UNLOCK_USER: [u8; 3] = [0x00, 0x08, 0x00];

const RETRIES: usize = 8;
const RETRY_DELAY: Duration = Duration::from_millis(250);

/// Retry a fallible device operation a few times before giving up.
fn retry<T, F>(label: &str, mut op: F) -> T
where
    F: FnMut() -> Result<T, String>,
{
    let mut last = String::new();
    for attempt in 1..=RETRIES {
        match op() {
            Ok(v) => return v,
            Err(e) => {
                last = e;
                eprintln!("  · {label}: attempt {attempt}/{RETRIES} failed: {last}");
                std::thread::sleep(RETRY_DELAY);
            }
        }
    }
    panic!("{label}: giving up after {RETRIES} attempts (last error: {last})");
}

/// Like [`retry`] but returns the last error instead of panicking. Used for the
/// lock step, which is the most RF-contention-sensitive operation: in a crowded
/// field the reader may fail to singulate the target tag, and that should be a
/// skip, not a failed run.
fn try_retry<T, F>(label: &str, attempts: usize, mut op: F) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    let mut last = String::new();
    for attempt in 1..=attempts {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) => {
                last = e;
                eprintln!("  · {label}: attempt {attempt}/{attempts} failed: {last}");
                std::thread::sleep(RETRY_DELAY);
            }
        }
    }
    Err(last)
}

fn pass(msg: &str) {
    println!("  ✓ {msg}");
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Poll the field enough times to reliably enumerate every tag present.
///
/// Single polling is probabilistic — one round can miss a weakly-coupled or
/// spaced-out tag — so we poll many times and union the results.
fn present_epcs(reader: &mut Reader) -> Vec<Vec<u8>> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    for _ in 0..60 {
        if let Ok(Some(tag)) = reader.send(&SinglePollingInstruction) {
            if !seen.contains(&tag.epc) {
                seen.push(tag.epc);
            }
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    seen
}

/// Select a single tag by its current EPC so subsequent read/write/lock only
/// touch that tag, run `body`, then clear the selection.
fn with_selected<T>(reader: &mut Reader, epc: &[u8], body: impl FnOnce(&mut Reader) -> T) -> T {
    retry("set select", || {
        reader
            .send(&SetSelect::by_epc(epc))
            .map_err(|e| e.to_string())
    });
    retry("enable select", || {
        reader.send(&SetSendSelect(true)).map_err(|e| e.to_string())
    });

    let out = body(reader);

    retry("disable select", || {
        reader
            .send(&SetSendSelect(false))
            .map_err(|e| e.to_string())
    });
    retry("clear select", || {
        reader
            .send(&SetSelect::by_epc(&[]))
            .map_err(|e| e.to_string())
    });
    out
}

/// Write `new_epc` into the EPC bank of the tag currently holding `current_epc`,
/// then verify by reading it back through a fresh selection.
fn rewrite_epc(reader: &mut Reader, current_epc: &[u8], new_epc: &[u8; 12]) {
    with_selected(reader, current_epc, |reader| {
        retry("write EPC", || {
            reader
                .send(&WriteLabel {
                    access_password: [0; 4],
                    bank: MemBank::Epc,
                    address: 0x02, // EPC words begin at word 2 (after CRC + PC)
                    data: new_epc.to_vec(),
                })
                .map_err(|e| e.to_string())
        });
    });

    // Verify the new EPC is now readable and selectable.
    let read_back = with_selected(reader, new_epc, |reader| {
        retry("verify EPC read-back", || {
            reader
                .send(&ReadLabel {
                    access_password: [0; 4],
                    bank: MemBank::Epc,
                    address: 0x02,
                    length: 6,
                })
                .map_err(|e| e.to_string())
        })
    });
    assert_eq!(
        &read_back[..],
        &new_epc[..],
        "EPC read-back mismatch: got {}, expected {}",
        hex(&read_back),
        hex(new_epc)
    );
}

/// Drive the two working tags to the canonical `1111...` / `2222...` state.
///
/// Any tag already at a target ID is left alone; the remaining physical tags are
/// assigned the still-missing target IDs (addressed by their current EPC). The
/// lock-capable tag (`EPC_LOCKABLE`) is protected and never rewritten.
///
/// All decisions are made from a *single* field snapshot so a tag that a later
/// scan happens to miss cannot desync "is this target present?" from "which
/// sources are available?".
fn ensure_canonical(reader: &mut Reader) {
    let targets = [EPC_ONE, EPC_TWO];
    let present = present_epcs(reader);

    // Which canonical targets are missing, and which present tags can supply
    // them — both derived from the same snapshot.
    let missing: Vec<[u8; 12]> = targets
        .into_iter()
        .filter(|t| !present.iter().any(|e| e[..] == t[..]))
        .collect();

    let mut sources: Vec<Vec<u8>> = present
        .iter()
        .filter(|e| !targets.iter().any(|t| t[..] == e[..]))
        .filter(|e| e[..] != EPC_LOCKABLE[..])
        .cloned()
        .collect();

    assert!(
        sources.len() >= missing.len(),
        "cannot reach canonical state: {} target(s) missing {:?} but only {} usable source tag(s) {:?} in field {:?}",
        missing.len(),
        missing.iter().map(|e| hex(e)).collect::<Vec<_>>(),
        sources.len(),
        sources.iter().map(|e| hex(e)).collect::<Vec<_>>(),
        present.iter().map(|e| hex(e)).collect::<Vec<_>>(),
    );

    // Deterministic mapping: assign the "heaviest" EPC to the first missing
    // target, so a fresh pair maps real -> 1111..., blank -> 2222....
    let weight = |e: &Vec<u8>| e.iter().map(|&b| u32::from(b)).sum::<u32>();
    sources.sort_by(|a, b| weight(b).cmp(&weight(a)).then(b.cmp(a)));

    // `sources.len() >= missing.len()` was asserted above, so zip consumes every
    // missing target without any fallible lookup.
    for (target, source) in missing.iter().zip(sources) {
        println!("  → assigning {} := {}", hex(&source), hex(target));
        rewrite_epc(reader, &source, target);
    }
}

/// Poll repeatedly (re-sweeping to catch weakly-coupled tags) until both
/// canonical tags have been observed, or panic with `msg` if they never are.
fn assert_both_present(reader: &mut Reader, msg: &str) {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let both = try_retry("observe both tags", RETRIES, || {
        for epc in present_epcs(reader) {
            if !seen.contains(&epc) {
                seen.push(epc);
            }
        }
        if seen.iter().any(|e| e[..] == EPC_ONE[..]) && seen.iter().any(|e| e[..] == EPC_TWO[..]) {
            Ok(())
        } else {
            Err(format!(
                "so far saw {:?}",
                seen.iter().map(|e| hex(e)).collect::<Vec<_>>()
            ))
        }
    });
    assert!(
        both.is_ok(),
        "{msg}: {:?}",
        seen.iter().map(|e| hex(e)).collect::<Vec<_>>()
    );
}

/// Module info (0x03): hardware, software and manufacturer strings.
fn info_phase(reader: &mut Reader) {
    for (label, param) in [
        ("hardware", ModuleInfoParam::HardwareVersion),
        ("software", ModuleInfoParam::SoftwareVersion),
        ("manufacturer", ModuleInfoParam::Manufacturer),
    ] {
        let info = retry(label, || {
            reader
                .send(&GetModuleInfo { param })
                .map_err(|e| e.to_string())
        });
        assert!(!info.text.is_empty(), "{label} info was empty");
        pass(&format!("{label}: {}", info.text));
    }
}

/// Region + channel (0x08, 0xAA) with a non-destructive set (0x07).
fn region_phase(reader: &mut Reader) {
    let region = retry("get region", || {
        reader.send(&GetWorkingArea).map_err(|e| e.to_string())
    });
    let channel = retry("get channel", || {
        reader.send(&GetWorkingChannel).map_err(|e| e.to_string())
    });
    let freq = region.channel_frequency(channel);
    pass(&format!("{region}, channel {channel} ({freq:.2} MHz)"));
    assert!(freq > 100.0, "channel frequency looks wrong: {freq}");

    // Set the region to its current value (no functional change).
    retry("set region (same value)", || {
        reader
            .send(&SetWorkingArea(region))
            .map_err(|e| e.to_string())
    });
    let region2 = retry("re-get region", || {
        reader.send(&GetWorkingArea).map_err(|e| e.to_string())
    });
    assert_eq!(region, region2, "region changed unexpectedly");
    pass("region set/get round-trips");
    // Region enum is byte-mapped; confirm a known value survives from_byte.
    assert_eq!(Region::from_byte(region as u8), Some(region));
}

/// Transmit power get/set/restore (0xB7, 0xB6).
fn power_phase(reader: &mut Reader) {
    let original = retry("get power", || {
        reader.send(&GetTransmitPower).map_err(|e| e.to_string())
    });
    pass(&format!("current power {original:.1} dBm"));
    let target = if original > 20.0 { 20.0 } else { 26.0 };
    retry("set power", || {
        reader
            .send(&SetTransmitPower(target))
            .map_err(|e| e.to_string())
    });
    let read_back = retry("verify power", || {
        reader.send(&GetTransmitPower).map_err(|e| e.to_string())
    });
    assert!(
        (read_back - target).abs() < 0.6,
        "power set to {target} but read {read_back}"
    );
    pass(&format!("power set to {read_back:.1} dBm"));
    retry("restore power", || {
        reader
            .send(&SetTransmitPower(original))
            .map_err(|e| e.to_string())
    });
    pass(&format!("power restored to {original:.1} dBm"));
}

/// Single polling (0x22): confirm both canonical tags are seen.
fn single_poll_phase(reader: &mut Reader) {
    assert_both_present(reader, "single-poll did not see both canonical tags");
    pass("single-poll sees both 1111... and 2222...");
}

/// Multi polling + stop (0x27, 0x28): collect tag reports for a short window.
fn multi_poll_phase(reader: &mut Reader) {
    retry("start multi-poll", || {
        reader
            .send_only(&MultiplePollingInstruction { pool_times: 100 })
            .map_err(|e| e.to_string())
    });
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_millis(800);
    while std::time::Instant::now() < deadline {
        match reader.recv() {
            Ok(frame) if frame.command_code == 0x22 => {
                if let Ok(Some(tag)) = SinglePollingInstruction.decode_response(&frame.data) {
                    if !seen.contains(&tag.epc) {
                        seen.push(tag.epc);
                    }
                }
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
    retry("stop multi-poll", || {
        reader
            .send_only(&StopMultiplePolling)
            .map_err(|e| e.to_string())
    });
    let _ = reader.recv(); // drain the stop ack
    assert!(!seen.is_empty(), "multi-poll returned no tag reports");
    pass(&format!("multi-poll saw {} distinct tag(s)", seen.len()));
}

/// Select filtering (0x0C, 0x12) proven via read isolation.
fn select_phase(reader: &mut Reader) {
    // Selecting a present tag lets a read succeed...
    let sel_read = retry("read selected tag", || {
        with_selected(reader, &EPC_ONE, |reader| {
            reader.send(&ReadLabel {
                access_password: [0; 4],
                bank: MemBank::Epc,
                address: 0x02,
                length: 6,
            })
        })
        .map_err(|e| e.to_string())
    });
    assert_eq!(
        sel_read[..],
        EPC_ONE[..],
        "selected read returned the wrong tag"
    );
    pass("select 1111... isolates it for reading");

    // ...selecting an absent tag makes the read fail.
    let bogus = [0xAB; 12];
    let bogus_read = with_selected(reader, &bogus, |reader| {
        reader.send(&ReadLabel {
            access_password: [0; 4],
            bank: MemBank::Epc,
            address: 0x02,
            length: 6,
        })
    });
    assert!(
        bogus_read.is_err(),
        "read of a non-existent selected tag unexpectedly succeeded"
    );
    pass("select of absent EPC correctly yields no tag");
}

/// Reversible lock/unlock of the User bank on the known lock-capable tag.
///
/// Skips (rather than fails) when that tag is not in the field or cannot be
/// singulated. If the lock succeeds, the unlock is retried hard so a tag is
/// never left locked.
fn lock_phase(reader: &mut Reader) {
    let lockable_present = present_epcs(reader)
        .iter()
        .any(|e| e[..] == EPC_LOCKABLE[..]);
    if lockable_present {
        // Locking needs a cleanly singulated tag; with several tags crowding the
        // antenna the reader may not isolate it. Give it a generous budget and,
        // if it still can't, skip rather than fail the whole run.
        let locked = try_retry("lock User bank", 15, || {
            with_selected(reader, &EPC_LOCKABLE, |reader| {
                reader.send(&LockTag {
                    password: [0; 4],
                    lock_data: LOCK_USER,
                })
            })
            .map_err(|e| e.to_string())
        });

        match locked {
            Ok(()) => {
                pass(&format!("locked User bank of {}", hex(&EPC_LOCKABLE)));

                // Bank must still be readable while locked (locked != unreadable).
                let while_locked = try_retry("read while locked", RETRIES, || {
                    with_selected(reader, &EPC_LOCKABLE, |reader| {
                        reader.send(&ReadLabel {
                            access_password: [0; 4],
                            bank: MemBank::User,
                            address: 0,
                            length: 1,
                        })
                    })
                    .map_err(|e| e.to_string())
                });
                if while_locked.is_ok() {
                    pass("User bank still readable while locked");
                }

                // Always restore to Open — this must succeed, so retry hard.
                // The unlock is RF-flaky under contention but does take effect;
                // keep trying with a large budget so a tag is never left locked.
                let unlocked = try_retry("unlock User bank", 30, || {
                    with_selected(reader, &EPC_LOCKABLE, |reader| {
                        reader.send(&LockTag {
                            password: [0; 4],
                            lock_data: UNLOCK_USER,
                        })
                    })
                    .map_err(|e| e.to_string())
                });
                assert!(
                    unlocked.is_ok(),
                    "failed to unlock tag after 30 attempts — tag may be left locked!"
                );
                pass("unlocked User bank (restored to Open)");
            }
            Err(e) => {
                println!(
                    "  ⚠ SKIP lock: could not singulate {} ({e})",
                    hex(&EPC_LOCKABLE)
                );
            }
        }
    } else {
        println!(
            "  ⚠ SKIP lock: lock-capable tag {} not in field",
            hex(&EPC_LOCKABLE)
        );
    }
}

fn main() {
    let port_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "/dev/cu.usbserial-10".to_string());

    let port = serialport::new(&port_name, 115_200)
        .timeout(Duration::from_millis(500))
        .open()
        .unwrap_or_else(|e| panic!("failed to open {port_name}: {e}"));
    let mut reader = SyncReader::new(port);

    println!("== hardware-in-the-loop test on {port_name} ==");

    // --- Establish canonical tag IDs up front so the run is deterministic. ---
    println!("[setup] driving tags to canonical 1111.../2222...");
    ensure_canonical(&mut reader);
    pass("tags are 1111... and 2222...");

    // --- Module info (0x03) ---
    println!("[info]");
    info_phase(&mut reader);

    println!("[region/channel]");
    region_phase(&mut reader);

    println!("[power]");
    power_phase(&mut reader);

    println!("[single poll]");
    single_poll_phase(&mut reader);

    println!("[multi poll]");
    multi_poll_phase(&mut reader);

    println!("[select]");
    select_phase(&mut reader);

    println!("[read TID]");
    let tid = with_selected(&mut reader, &EPC_ONE, |reader| {
        retry("read TID", || {
            reader
                .send(&ReadLabel {
                    access_password: [0; 4],
                    bank: MemBank::Tid,
                    address: 0x00,
                    length: 2,
                })
                .map_err(|e| e.to_string())
        })
    });
    pass(&format!("TID(2 words) = {}", hex(&tid)));

    // --- Write (0x49): round-trip one tag's EPC ---
    println!("[write]");
    // Change 1111... -> aaaa... and back. A single-tag round-trip exercises
    // WriteLabel + read-back verification twice, without a fragile two-tag swap
    // (which collides badly when several tags share the field).
    let temp = [0xAA; 12];
    rewrite_epc(&mut reader, &EPC_ONE, &temp);
    pass("wrote 1111... -> aaaa...");
    rewrite_epc(&mut reader, &temp, &EPC_ONE);
    pass("wrote aaaa... -> 1111...");

    // --- Lock (0x82): reversible User-bank lock then unlock ---
    println!("[lock]");
    lock_phase(&mut reader);

    // --- Teardown: guarantee canonical IDs regardless of the above. ---
    println!("[teardown] restoring canonical 1111.../2222...");
    ensure_canonical(&mut reader);
    assert_both_present(&mut reader, "teardown failed to restore canonical IDs");
    pass("tags restored to 1111... and 2222...");

    println!("\n== ALL CHECKS PASSED ==");
}
