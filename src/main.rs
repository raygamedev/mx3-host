use std::fs;
use std::io::{self, Read, Write};
use std::os::raw::{c_int, c_ulong};
use std::os::unix::io::AsRawFd;

// ---------------------------------------------------------------------------
// HID++ 2.0 report IDs and sizes
// ---------------------------------------------------------------------------

const REPORT_ID_SHORT: u8 = 0x10; // 7 bytes  – USB / Unifying receiver
const REPORT_ID_LONG: u8 = 0x11;  // 20 bytes – direct Bluetooth

const REPORT_LEN_SHORT: usize = 7;
const REPORT_LEN_LONG: usize = 20;

const SW_ID: u8 = 0x01;
const FEATURE_CHANGE_HOST_HI: u8 = 0x18;
const FEATURE_CHANGE_HOST_LO: u8 = 0x14; // feature 0x1814
const HIDPP_ERROR: u8 = 0x8F;

// HID bus codes from the HID_ID uevent field
const BUS_USB: u16 = 0x0003; // device on USB Unifying/Bolt receiver
                              // all other buses (0x0005 BT etc.) use the long report / 0xFF index

// ---------------------------------------------------------------------------
// Minimal poll(2) via FFI (no external crates)
// ---------------------------------------------------------------------------

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

extern "C" {
    fn poll(fds: *mut PollFd, nfds: c_ulong, timeout: c_int) -> c_int;
}

fn wait_readable(fd: c_int, timeout_ms: c_int) -> bool {
    let mut pfd = PollFd { fd, events: 0x0001, revents: 0 };
    unsafe { poll(&mut pfd, 1, timeout_ms) > 0 }
}

// ---------------------------------------------------------------------------
// Device discovery
// ---------------------------------------------------------------------------

struct DeviceInfo {
    name: String,
    bus: u16,
}

impl DeviceInfo {
    fn path(&self) -> String {
        format!("/dev/{}", self.name)
    }

    /// HID++ device index:
    ///   0x01 – first device on a USB Unifying/Bolt receiver
    ///   0xFF – direct Bluetooth (or any other direct connection)
    fn device_index(&self) -> u8 {
        if self.bus == BUS_USB { 0x01 } else { 0xFF }
    }

    /// HID++ report ID and total byte length for this connection type.
    /// BT: long report 0x11 (20 bytes) — USB: short report 0x10 (7 bytes).
    fn report_fmt(&self) -> (u8, usize) {
        if self.bus == BUS_USB {
            (REPORT_ID_SHORT, REPORT_LEN_SHORT)
        } else {
            (REPORT_ID_LONG, REPORT_LEN_LONG)
        }
    }
}

/// Parse HID_ID=bus:vendor:product from a uevent file.
/// Returns the bus number if the vendor is Logitech (0x046D), else None.
fn parse_logitech_uevent(content: &str) -> Option<u16> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("HID_ID=") {
            let mut parts = rest.split(':');
            let bus_str = parts.next()?;
            let vendor_str = parts.next()?;
            let bus = u16::from_str_radix(bus_str, 16).ok()?;
            let vendor = u32::from_str_radix(vendor_str, 16).ok()?;
            if vendor == 0x0000_046D {
                return Some(bus);
            }
        }
    }
    None
}

fn find_logitech_devices() -> Vec<DeviceInfo> {
    let mut devices: Vec<DeviceInfo> = fs::read_dir("/sys/class/hidraw")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().into_owned();
                    let uevent_path =
                        format!("/sys/class/hidraw/{}/device/uevent", name);
                    let content = fs::read_to_string(&uevent_path).unwrap_or_default();
                    parse_logitech_uevent(&content).map(|bus| DeviceInfo { name, bus })
                })
                .collect()
        })
        .unwrap_or_default();
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    devices
}

/// Send getFeature(0x0000) and confirm we get a coherent HID++ reply.
/// Skips non-HID++ reports (e.g. mouse movement) that may arrive first.
fn probe_hidpp(dev: &DeviceInfo) -> bool {
    let device_index = dev.device_index();
    let (report_id, report_len) = dev.report_fmt();

    let mut file = match fs::OpenOptions::new().read(true).write(true).open(dev.path()) {
        Ok(f) => f,
        Err(e) => {
            if e.kind() == io::ErrorKind::PermissionDenied {
                eprintln!(
                    "Error: permission denied opening {}.\n\
                     Run install.sh to set up the udev rule:\n\
                     \x20 sudo bash install.sh\n\
                     \x20 sudo usermod -aG <group> $USER  # group printed by install.sh\n\
                     Then log out/in and re-plug the device.",
                    dev.path()
                );
                std::process::exit(1);
            }
            return false;
        }
    };

    let mut req = vec![0u8; report_len];
    req[0] = report_id;
    req[1] = device_index;
    req[2] = 0x00; // root feature index
    req[3] = SW_ID; // function 0 | sw_id

    if file.write_all(&req).is_err() {
        return false;
    }

    let fd = file.as_raw_fd();
    let mut buf = [0u8; 64];
    loop {
        if !wait_readable(fd, 1000) {
            return false;
        }
        match file.read(&mut buf) {
            Ok(n) if n >= 4 && buf[0] == report_id && buf[1] == device_index => return true,
            Ok(n) if n > 0 => continue, // non-HID++ report, keep waiting
            _ => return false,
        }
    }
}

fn find_device() -> Result<DeviceInfo, String> {
    let devices = find_logitech_devices();
    if devices.is_empty() {
        return Err(
            "no Logitech HID device found; is the mouse connected?\n\
             (scanned /sys/class/hidraw/*/device/uevent for vendor 046d)"
                .to_string(),
        );
    }

    for dev in devices {
        if probe_hidpp(&dev) {
            return Ok(dev);
        }
    }

    Err(
        "found Logitech hidraw node(s) but none gave a valid HID++ response.\n\
         Ensure the mouse is awake, then run:\n\
         \x20 sudo bash install.sh\n\
         \x20 sudo usermod -aG <group> $USER  # group printed by install.sh\n\
         Then log out/in and re-plug the device."
            .to_string(),
    )
}

// ---------------------------------------------------------------------------
// HID++ host switching
// ---------------------------------------------------------------------------

fn switch_host(dev: &DeviceInfo, host: u8) -> Result<(), String> {
    let device_index = dev.device_index();
    let (report_id, report_len) = dev.report_fmt();

    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(dev.path())
        .map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                format!(
                    "permission denied opening {}.\n\
                     Run install.sh to set up the udev rule:\n\
                     \x20 sudo bash install.sh\n\
                     \x20 sudo usermod -aG <group> $USER  # group printed by install.sh\n\
                     Then log out/in and re-plug the device.",
                    dev.path()
                )
            } else {
                format!("cannot open {}: {}", dev.path(), e)
            }
        })?;

    let fd = file.as_raw_fd();

    // ------------------------------------------------------------------
    // Step 1: getFeature(0x1814) – get the ChangeHost feature index
    // ------------------------------------------------------------------
    let mut get_feat = vec![0u8; report_len];
    get_feat[0] = report_id;
    get_feat[1] = device_index;
    get_feat[2] = 0x00;                 // root feature is always index 0
    get_feat[3] = (0x00 << 4) | SW_ID; // function 0 | sw_id
    get_feat[4] = FEATURE_CHANGE_HOST_HI;
    get_feat[5] = FEATURE_CHANGE_HOST_LO;

    file.write_all(&get_feat)
        .map_err(|e| format!("write error (getFeature): {}", e))?;

    // Read reports, skipping mouse/keyboard input, until we get the HID++ reply.
    let mut rbuf = [0u8; 64];
    let feat_idx = loop {
        if !wait_readable(fd, 2000) {
            return Err("timeout waiting for getFeature response (is the mouse awake?)".to_string());
        }
        let n = file.read(&mut rbuf)
            .map_err(|e| format!("read error (getFeature): {}", e))?;
        let resp = &rbuf[..n];

        if n < 6 || resp[0] != report_id {
            continue; // non-HID++ report (e.g. mouse movement), skip
        }
        if resp[1] != device_index {
            return Err(format!("unexpected device index in getFeature response: {:02X?}", resp));
        }
        if resp[2] == HIDPP_ERROR {
            return Err(format!("HID++ error 0x{:02X} for getFeature(0x1814)", resp[5]));
        }
        break resp[4];
    };

    if feat_idx == 0 {
        return Err(
            "device does not support ChangeHost (feature 0x1814).\n\
             This may not be an MX Master 3/3S, or the firmware is too old."
                .to_string(),
        );
    }

    // ------------------------------------------------------------------
    // Step 2: setCurrentHost — write_fnid=0x10 (function 1), no_reply=true
    // Source: Solaar settings_templates.py ChangeHost.rw_options
    // The device disconnects immediately; no ACK is expected.
    // ------------------------------------------------------------------
    let mut set_host = vec![0u8; report_len];
    set_host[0] = report_id;
    set_host[1] = device_index;
    set_host[2] = feat_idx;
    set_host[3] = 0x10 | SW_ID; // function 1 | sw_id
    set_host[4] = host - 1;     // 0-indexed host

    file.write_all(&set_host)
        .map_err(|e| format!("write error (setCurrentHost): {}", e))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() != 2 {
        eprintln!("Usage: {} <1|2|3>", args[0]);
        std::process::exit(1);
    }

    let host: u8 = match args[1].trim().parse::<u8>() {
        Ok(n) if (1..=3).contains(&n) => n,
        _ => {
            eprintln!("Error: host must be 1, 2, or 3 (got {:?})", args[1]);
            std::process::exit(1);
        }
    };

    let dev = match find_device() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    if let Err(e) = switch_host(&dev, host) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    println!("Host {} active", host);
}
