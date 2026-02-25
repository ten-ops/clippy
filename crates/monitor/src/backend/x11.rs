//! X11 clipboard backend implementation.
//!
//! This module provides a concrete implementation of the [`ClipboardBackend`] trait
//! for the X11 window system. It connects to an X server, uses the XFIXES extension
//! to monitor clipboard changes, and retrieves clipboard contents via the standard
//! X11 selection mechanism.
//!
//! # Protocol Details
//! - Communicates with the X server over a UNIX socket (default: `/tmp/.X11-unix/X<display>`).
//! - Authenticates using MIT-MAGIC-COOKIE-1 from `.Xauthority` if available.
//! - Uses the XFIXES extension (version ≥ 1.0) to receive selection change notifications.
//! - Requests clipboard contents in UTF8_STRING or STRING format.
//!
//! # Limitations
//! - Only supports local connections via UNIX sockets; TCP connections are rejected.
//! - Assumes little-endian byte order for X11 protocol (common on x86/x64). On big-endian
//!   systems, this implementation would need adjustment.
//! - Xauthority parsing assumes big-endian byte order as per the X11 standard.
//!
//! # Safety
//! This module uses unsafe syscalls (socket, connect, read, write, close) to interact
//! with the operating system. All calls are checked for errors and wrapped in safe
//! abstractions where possible. The unsafe blocks are necessary for FFI with the kernel.

use std::env;
use std::ffi::CString;
use std::mem;
use std::ptr;
use syscalls::{SockAddrUn, SyscallError, close, connect, read, socket, write};

use crate::ClipboardBackend;

// =============================================================================
// Constants
// =============================================================================

/// X11 protocol major version (X11R7.x uses 11).
const X_PROTO_MAJOR: u8 = 11;

/// X11 protocol minor version (typically 0).
const X_PROTO_MINOR: u8 = 0;

/// X11 reply type opcode.
const X_OP_REPLY: u8 = 1;

/// X11 error type opcode.
const X_OP_ERROR: u8 = 0;

/// X11 CreateWindow request opcode.
const X_OP_CREATE_WINDOW: u8 = 1;

/// X11 QueryExtension request opcode.
const X_OP_QUERY_EXTENSION: u8 = 98;

/// Address family for UNIX domain sockets (from sys/socket.h).
const AF_UNIX: i32 = 1;

/// Socket type for stream-oriented connections.
const SOCK_STREAM: i32 = 1;

/// X11 SelectionNotify event type.
const EVENT_SELECTION_NOTIFY: u8 = 31;

/// X11 generic event (used for XFIXES events).
const EVENT_GENERIC: u8 = 35;

/// X11 InternAtom request opcode.
const X_OP_INTERN_ATOM: u8 = 16;

/// X11 ConvertSelection request opcode.
const X_OP_CONVERT_SELECTION: u8 = 24;

/// X11 GetProperty request opcode.
const X_OP_GET_PROPERTY: u8 = 20;

/// Window class: InputOnly (no visuals, used for receiving events).
const WINDOW_CLASS_INPUT_ONLY: u16 = 2;

/// XFIXES SelectionNotify event type (relative to extension's event base).
const XFIXES_SELECTION_NOTIFY: u8 = 0;

/// Mask to receive SelectionNotify events when selection owner changes.
const XFIXES_SET_SELECTION_OWNER_NOTIFY_MASK: u32 = 1 << 0;

/// XFIXES SelectSelectionInput request sub-opcode.
const XFIXES_SELECT_SELECTION_INPUT: u8 = 2;

/// Special property type value indicating "any type" for GetProperty.
const ANY_PROPERTY_TYPE: u32 = 0;

/// Interrupted system call error number (EINTR) – used for retryable reads.
const EINTR: i32 = 4;

// =============================================================================
// Error Type
// =============================================================================

/// Errors that can occur during X11 clipboard operations.
#[derive(Debug)]
pub enum X11Error {
    /// A system call failed (e.g., socket, connect, read, write, close).
    Syscall(SyscallError),

    /// The connection to the X server was closed unexpectedly.
    ConnectionClosed,

    /// A protocol-level error occurred (e.g., malformed message, handshake failure).
    Protocol(&'static str),

    /// An incomplete message was received (should not happen with proper error handling).
    Incomplete,

    /// Received an event that the implementation does not support.
    UnsupportedEvent,

    /// The DISPLAY environment variable is not set.
    NoDisplay,

    /// The DISPLAY variable is malformed.
    InvalidDisplay(&'static str),

    /// TCP connections are not supported (only local UNIX sockets).
    TcpNotSupported,

    /// Failed to intern an atom (e.g., CLIPBOARD) – possibly server issue.
    AtomInternFailed,

    /// Failed to convert the selection to a supported format.
    SelectionConversionFailed,

    /// X11 error received from the server (opcode indicates error type).
    XError(u8),

    /// XFIXES extension is not available on the connected X server.
    XFixesNotAvailable,

    /// The operation was interrupted by a signal (EINTR). Caller may retry.
    Interrupted,
}

impl From<SyscallError> for X11Error {
    fn from(err: SyscallError) -> Self {
        X11Error::Syscall(err)
    }
}

// =============================================================================
// X11 Event Structure
// =============================================================================

/// A raw X11 event as received from the server.
#[derive(Debug)]
pub struct X11Event {
    /// The event type code (first byte of the event).
    pub event_type: u8,
    /// The full raw event data (including header).
    pub raw_data: Vec<u8>,
}

impl X11Event {
    /// Creates a new X11Event from raw data.
    ///
    /// # Arguments
    /// * `raw_data` - The complete event data as read from the socket.
    ///
    /// # Returns
    /// A new `X11Event` with the event type extracted from the first byte.
    fn new(raw_data: Vec<u8>) -> Self {
        let event_type = raw_data[0];
        X11Event {
            event_type,
            raw_data,
        }
    }
}

// =============================================================================
// X11 Connection
// =============================================================================

/// Represents an active connection to an X11 server.
///
/// This struct holds the socket file descriptor, a read buffer for incoming data,
/// cached atom identifiers, and the window IDs used for clipboard operations.
#[derive(Debug)]
pub struct X11Connection {
    /// Socket file descriptor for the connection.
    fd: i32,

    /// Buffer for accumulating data read from the socket.
    read_buf: Vec<u8>,

    /// Cached atom for the CLIPBOARD selection.
    clipboard_atom: Option<u32>,

    /// Cached atom for UTF8_STRING.
    utf8_string_atom: Option<u32>,

    /// Cached atom for STRING.
    string_atom: Option<u32>,

    /// Cached atom for a temporary property used to retrieve clipboard data.
    property_atom: Option<u32>,

    /// XID of the root window.
    root_window: u32,

    /// XID of our own window (used as requestor for clipboard conversion).
    pub our_window: u32,

    /// Next sequence number to assign to a request (used for matching replies).
    next_seq: u16,

    /// Major opcode of the XFIXES extension, if available.
    xfixes_opcode: Option<u8>,

    /// Event base for XFIXES events (added to event type to get actual XFIXES event).
    xfixes_event_base: Option<u8>,
}

/// Entry in the .Xauthority file.
///
/// See: https://www.x.org/releases/X11R7.7/doc/libX11/Xau/a8.html
struct XAuthEntry {
    /// Authentication family (e.g., FamilyLocal, FamilyWild).
    #[allow(dead_code)]
    family: u16,
    /// Address (hostname, IP, etc.).
    #[allow(dead_code)]
    address: Vec<u8>,
    /// Display number string (e.g., "0").
    display: Vec<u8>,
    /// Authentication method name (e.g., "MIT-MAGIC-COOKIE-1").
    name: Vec<u8>,
    /// Authentication data (cookie).
    data: Vec<u8>,
}

/// Debug print macro that is disabled during tests.
macro_rules! dbg_println {
    ($($arg:tt)*) => {
        if !cfg!(test) {
            println!($($arg)*);
        }
    };
}

// -----------------------------------------------------------------------------
// Xauthority Parsing Helpers
// -----------------------------------------------------------------------------

/// Reads a big-endian u16 from a buffer at the given position.
///
/// # Arguments
/// * `buf` - The byte buffer.
/// * `pos` - Mutable reference to the current position (updated on success).
///
/// # Returns
/// `Some(u16)` if enough bytes are available, otherwise `None`.
fn read_u16_be(buf: &[u8], pos: &mut usize) -> Option<u16> {
    if buf.len() < *pos + 2 {
        return None;
    }
    let v = u16::from_be_bytes([buf[*pos], buf[*pos + 1]]);
    *pos += 2;
    Some(v)
}

/// Reads a counted byte sequence from a buffer (big-endian length prefix).
///
/// The format is a big-endian u16 length followed by that many bytes.
///
/// # Arguments
/// * `buf` - The byte buffer.
/// * `pos` - Mutable reference to the current position (updated on success).
///
/// # Returns
/// `Some(Vec<u8>)` with the bytes, if enough bytes are available.
fn read_counted_bytes(buf: &[u8], pos: &mut usize) -> Option<Vec<u8>> {
    let len = read_u16_be(buf, pos)? as usize;
    if buf.len() < *pos + len {
        return None;
    }
    let v = buf[*pos..*pos + len].to_vec();
    *pos += len;
    Some(v)
}

/// Parses the binary .Xauthority format.
///
/// # Arguments
/// * `data` - Raw contents of the .Xauthority file.
///
/// # Returns
/// A vector of `XAuthEntry` structures found in the data.
fn parse_xauth_entries(data: &[u8]) -> Vec<XAuthEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos < data.len() {
        let Some(family) = read_u16_be(data, &mut pos) else {
            break;
        };
        let Some(address) = read_counted_bytes(data, &mut pos) else {
            break;
        };
        let Some(display) = read_counted_bytes(data, &mut pos) else {
            break;
        };
        let Some(name) = read_counted_bytes(data, &mut pos) else {
            break;
        };
        let Some(auth) = read_counted_bytes(data, &mut pos) else {
            break;
        };
        entries.push(XAuthEntry {
            family,
            address,
            display,
            name,
            data: auth,
        });
    }
    entries
}

/// Reads the Xauthority file from the standard location.
///
/// The file path is taken from the `XAUTHORITY` environment variable, or if that
/// is not set, `$HOME/.Xauthority`. Returns an empty vector if the file cannot be read.
fn read_xauthority() -> Vec<XAuthEntry> {
    let path = env::var("XAUTHORITY")
        .ok()
        .or_else(|| env::var("HOME").ok().map(|h| format!("{}/.Xauthority", h)));
    match path {
        Some(p) => parse_xauth_entries(std::fs::read(&p).unwrap_or_default().as_slice()),
        None => Vec::new(),
    }
}

/// Finds an MIT-MAGIC-COOKIE-1 entry for the given display number.
///
/// # Arguments
/// * `display_num` - The display number (e.g., 0 for :0).
///
/// # Returns
/// `Some((name, data))` where `name` is the authentication method name (always b"MIT-MAGIC-COOKIE-1")
/// and `data` is the cookie. Returns `None` if no matching entry is found.
fn find_cookie(display_num: u32) -> Option<(Vec<u8>, Vec<u8>)> {
    let display_str = display_num.to_string();
    for e in read_xauthority() {
        if e.display == display_str.as_bytes() && e.name == b"MIT-MAGIC-COOKIE-1" {
            return Some((e.name, e.data));
        }
    }
    None
}

// -----------------------------------------------------------------------------
// Socket I/O Helpers
// -----------------------------------------------------------------------------

/// Reads from a file descriptor, retrying on EINTR.
///
/// # Arguments
/// * `fd` - The file descriptor.
/// * `buf` - The buffer to read into.
///
/// # Returns
/// * `Ok(n)` on success (number of bytes read).
/// * `Err(X11Error::ConnectionClosed)` if read returns 0 (EOF).
/// * `Err(X11Error::Interrupted)` if EINTR occurs.
/// * `Err(X11Error::ConnectionClosed)` for any other error (treat as closed).
fn read_with_retry(fd: i32, buf: &mut [u8]) -> Result<usize, X11Error> {
    match unsafe { read(fd, buf) } {
        Ok(0) => Err(X11Error::ConnectionClosed),
        Ok(n) => Ok(n),
        Err(e) if e.0 == EINTR => Err(X11Error::Interrupted),
        Err(_) => Err(X11Error::ConnectionClosed),
    }
}

/// Reads exactly `n` bytes from the socket into a new vector.
///
/// # Arguments
/// * `fd` - The file descriptor.
/// * `n` - Number of bytes to read.
///
/// # Returns
/// * `Ok(Vec<u8>)` containing exactly `n` bytes.
/// * `Err(X11Error)` if the connection closes or an error occurs before reading all bytes.
fn read_exact_fd(fd: i32, n: usize) -> Result<Vec<u8>, X11Error> {
    let mut buf = vec![0u8; n];
    let mut pos = 0;
    while pos < n {
        pos += read_with_retry(fd, &mut buf[pos..])?;
    }
    Ok(buf)
}

// -----------------------------------------------------------------------------
// X11Connection Implementation
// -----------------------------------------------------------------------------

impl X11Connection {
    /// Connects to the X server specified by the DISPLAY environment variable.
    ///
    /// # Steps
    /// 1. Parse DISPLAY (e.g., ":0", "unix:0") – only UNIX sockets are supported.
    /// 2. Open a UNIX socket to `/tmp/.X11-unix/X<display>`.
    /// 3. Perform X11 connection handshake with optional MIT-MAGIC-COOKIE-1 authentication.
    /// 4. Create an InputOnly window to receive clipboard events.
    /// 5. Query the XFIXES extension and verify it is available.
    /// 6. Return an initialized `X11Connection` instance.
    ///
    /// # Returns
    /// * `Ok(Self)` on successful connection.
    /// * `Err(X11Error)` on any failure (connection, authentication, unsupported server).
    pub fn connect() -> Result<Self, X11Error> {
        // Parse DISPLAY environment variable.
        let display_str = env::var("DISPLAY").map_err(|_| X11Error::NoDisplay)?;

        let (host_part, rest) = display_str
            .split_once(':')
            .ok_or(X11Error::InvalidDisplay("missing colon"))?;

        let (display_part, _) = rest.split_once('.').unwrap_or((rest, "0"));

        let display_num: u32 = display_part
            .parse()
            .map_err(|_| X11Error::InvalidDisplay("invalid display number"))?;

        // Reject TCP connections (only local UNIX socket supported).
        if !host_part.is_empty() && host_part != "unix" {
            return Err(X11Error::TcpNotSupported);
        }

        // Construct socket path and connect.
        let socket_path = format!("/tmp/.X11-unix/X{}", display_num);
        let path_cstr = CString::new(socket_path)
            .map_err(|_| X11Error::InvalidDisplay("socket path too long"))?;

        let fd = unsafe { socket(AF_UNIX, SOCK_STREAM, 0) }?;

        let mut addr: SockAddrUn = unsafe { mem::zeroed() };
        addr.sun_family = AF_UNIX as u16;
        let bytes = path_cstr.to_bytes_with_nul();
        let len_to_copy = bytes.len().min(addr.sun_path.len());
        unsafe {
            ptr::copy_nonoverlapping(
                bytes.as_ptr() as *const i8,
                addr.sun_path.as_mut_ptr(),
                len_to_copy,
            );
        }

        let addr_len = (mem::size_of::<u16>() + len_to_copy) as u32;
        unsafe { connect(fd, &addr as *const _, addr_len) }?;

        // Retrieve authentication cookie.
        let cookie = find_cookie(display_num);
        let (auth_name, auth_data) = match &cookie {
            Some((n, d)) => (n.as_slice(), d.as_slice()),
            None => (&b""[..], &b""[..]),
        };

        // Build X11 connection setup request.
        let auth_name_len = auth_name.len() as u16;
        let auth_data_len = auth_data.len() as u16;
        let auth_name_pad = (4 - (auth_name.len() % 4)) % 4;
        let auth_data_pad = (4 - (auth_data.len() % 4)) % 4;

        let mut setup_req: Vec<u8> = Vec::new();
        setup_req.push(b'l'); // byte order (little-endian)
        setup_req.push(0); // unused
        setup_req.extend_from_slice(&(X_PROTO_MAJOR as u16).to_le_bytes());
        setup_req.extend_from_slice(&(X_PROTO_MINOR as u16).to_le_bytes());
        setup_req.extend_from_slice(&auth_name_len.to_le_bytes());
        setup_req.extend_from_slice(&auth_data_len.to_le_bytes());
        setup_req.extend_from_slice(&[0, 0]); // padding
        setup_req.extend_from_slice(auth_name);
        setup_req.extend(std::iter::repeat(0).take(auth_name_pad));
        setup_req.extend_from_slice(auth_data);
        setup_req.extend(std::iter::repeat(0).take(auth_data_pad));

        unsafe { write(fd, &setup_req)? };

        // Read handshake reply (8 bytes header).
        let hdr = read_exact_fd(fd, 8)?;
        let status = hdr[0];

        if status == 0 {
            // Server rejected connection; read and discard the error reason.
            let reason_len = hdr[1] as usize;
            let additional = u16::from_le_bytes([hdr[6], hdr[7]]) as usize * 4;
            if reason_len.max(additional) > 0 {
                let _ = read_exact_fd(fd, reason_len.max(additional));
            }
            dbg_println!("X11 connection rejected (status=0). Check $XAUTHORITY / ~/.Xauthority.");
            return Err(X11Error::Protocol("handshake rejected by server"));
        }
        if status != 1 {
            return Err(X11Error::Protocol("unknown handshake status"));
        }

        // Read the rest of the setup reply (vendor and screen info).
        let additional_bytes = u16::from_le_bytes([hdr[6], hdr[7]]) as usize * 4;
        let setup = read_exact_fd(fd, additional_bytes)?;

        if setup.len() < 32 {
            return Err(X11Error::Protocol("setup reply too short"));
        }

        // Parse resource ID allocation and root window.
        let resource_id_base = u32::from_le_bytes(setup[4..8].try_into().unwrap());
        let resource_id_mask = u32::from_le_bytes(setup[8..12].try_into().unwrap());
        let vendor_len = u16::from_le_bytes([setup[16], setup[17]]) as usize;
        let num_formats = setup[21] as usize;
        let vendor_padded = (vendor_len + 3) & !3;
        let screen_offset = 32 + vendor_padded + num_formats * 8;

        if setup.len() < screen_offset + 4 {
            return Err(X11Error::Protocol(
                "setup reply truncated before screen data",
            ));
        }

        let root_window =
            u32::from_le_bytes(setup[screen_offset..screen_offset + 4].try_into().unwrap());
        // Allocate a new XID for our window: resource_id_base | (1 & resource_id_mask)
        // This is a simple allocation strategy that should not conflict with existing IDs.
        let our_window = resource_id_base | (1 & resource_id_mask);

        dbg_println!(
            "DEBUG connect: root=0x{:08X} our=0x{:08X}",
            root_window,
            our_window
        );

        let mut conn = X11Connection {
            fd,
            read_buf: Vec::new(),
            clipboard_atom: None,
            utf8_string_atom: None,
            string_atom: None,
            property_atom: None,
            root_window,
            our_window,
            next_seq: 1,
            xfixes_opcode: None,
            xfixes_event_base: None,
        };

        // Create an InputOnly window to serve as requestor for clipboard conversion.
        conn.create_window(our_window, root_window)?;

        // Query XFIXES extension.
        let (opcode, event_base, _) = conn.query_extension("XFIXES")?;
        dbg_println!("DEBUG xfixes: opcode={} event_base={}", opcode, event_base);
        if opcode == 0 {
            return Err(X11Error::XFixesNotAvailable);
        }
        conn.xfixes_opcode = Some(opcode);
        conn.xfixes_event_base = Some(event_base);

        // Check XFIXES version (optional, but we require at least 1.0).
        conn.query_xfixes_version(1, 0)?;

        Ok(conn)
    }

    /// Queries the version of the XFIXES extension.
    ///
    /// # Arguments
    /// * `major` - Client's desired major version.
    /// * `minor` - Client's desired minor version.
    ///
    /// # Returns
    /// `Ok(())` if the server supports at least the requested version.
    /// Otherwise, the reply still contains the server's version, but we ignore it.
    fn query_xfixes_version(&mut self, major: u32, minor: u32) -> Result<(), X11Error> {
        let opcode = self.xfixes_opcode.ok_or(X11Error::XFixesNotAvailable)?;
        let mut req = Vec::with_capacity(12);
        req.push(opcode);
        req.push(0); // unused
        req.extend_from_slice(&3u16.to_le_bytes()); // request length in 4-byte units
        req.extend_from_slice(&major.to_le_bytes());
        req.extend_from_slice(&minor.to_le_bytes());

        let reply = self.send_request_with_reply(&req)?;
        if reply.len() < 32 {
            return Err(X11Error::Protocol("XFixesQueryVersion reply too short"));
        }
        let server_major = u32::from_le_bytes(reply[8..12].try_into().unwrap());
        let server_minor = u32::from_le_bytes(reply[12..16].try_into().unwrap());
        dbg_println!("XFixes version: {}.{}", server_major, server_minor);
        Ok(())
    }

    /// Creates an InputOnly window.
    ///
    /// # Arguments
    /// * `wid` - The XID for the new window.
    /// * `parent` - The parent window (usually the root).
    ///
    /// # Returns
    /// `Ok(())` if the request was sent successfully. (No reply is expected.)
    fn create_window(&mut self, wid: u32, parent: u32) -> Result<(), X11Error> {
        let mut req = [0u8; 32];
        req[0] = X_OP_CREATE_WINDOW;
        req[1] = 0; // unused
        req[2..4].copy_from_slice(&8u16.to_le_bytes()); // request length in 4-byte units
        req[4..8].copy_from_slice(&wid.to_le_bytes());
        req[8..12].copy_from_slice(&parent.to_le_bytes());
        req[16..18].copy_from_slice(&1u16.to_le_bytes()); // width=1
        req[18..20].copy_from_slice(&1u16.to_le_bytes()); // height=1
        req[22..24].copy_from_slice(&WINDOW_CLASS_INPUT_ONLY.to_le_bytes());
        unsafe { write(self.fd, &req)? };
        self.next_seq += 1;
        Ok(())
    }

    /// Queries whether an extension is present and returns its opcode and event bases.
    ///
    /// # Arguments
    /// * `name` - The extension name (e.g., "XFIXES").
    ///
    /// # Returns
    /// `(opcode, event_base, error_base)` where `opcode` is zero if not present.
    fn query_extension(&mut self, name: &str) -> Result<(u8, u8, u8), X11Error> {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u16;
        let req_len = 2 + (name_bytes.len() + 3) / 4; // length in 4-byte units
        let mut req = Vec::with_capacity(req_len * 4);
        req.push(X_OP_QUERY_EXTENSION);
        req.push(0);
        req.extend_from_slice(&(req_len as u16).to_le_bytes());
        req.extend_from_slice(&name_len.to_le_bytes());
        req.extend_from_slice(&[0, 0]); // padding
        req.extend_from_slice(name_bytes);
        while req.len() % 4 != 0 {
            req.push(0);
        }

        let reply = self.send_request_with_reply(&req)?;
        if reply.len() < 32 {
            return Err(X11Error::Protocol("QueryExtension reply too short"));
        }
        if reply[8] == 0 {
            Ok((0, 0, 0))
        } else {
            Ok((reply[9], reply[10], reply[11]))
        }
    }

    /// Subscribes to XFIXES selection change events for the CLIPBOARD.
    ///
    /// After this call, the server will send SelectionNotify events whenever the
    /// CLIPBOARD selection owner changes.
    ///
    /// # Returns
    /// `Ok(())` on success.
    pub fn select_clipboard_events(&mut self) -> Result<(), X11Error> {
        let opcode = self.xfixes_opcode.ok_or(X11Error::XFixesNotAvailable)?;
        let clipboard = self.intern_atom_cached("CLIPBOARD", false, |c| &mut c.clipboard_atom)?;
        dbg_println!(
            "DEBUG select_clipboard_events: CLIPBOARD atom={}",
            clipboard
        );
        let req_len: u16 = 4; // length in 4-byte units (16 bytes total)
        let mut req = Vec::with_capacity(16);
        req.push(opcode);
        req.push(XFIXES_SELECT_SELECTION_INPUT);
        req.extend_from_slice(&req_len.to_le_bytes());
        req.extend_from_slice(&self.root_window.to_le_bytes());
        req.extend_from_slice(&clipboard.to_le_bytes());
        req.extend_from_slice(&XFIXES_SET_SELECTION_OWNER_NOTIFY_MASK.to_le_bytes());
        unsafe { write(self.fd, &req)? };
        self.next_seq += 1;
        dbg_println!("DEBUG select_clipboard_events: subscribed OK");
        Ok(())
    }

    /// Ensures that the read buffer contains at least `min_needed` bytes.
    ///
    /// Reads from the socket repeatedly until the buffer has enough data.
    fn fill_buf(&mut self, min_needed: usize) -> Result<(), X11Error> {
        while self.read_buf.len() < min_needed {
            let mut tmp = [0u8; 4096];
            let n = read_with_retry(self.fd, &mut tmp)?;
            self.read_buf.extend_from_slice(&tmp[..n]);
        }
        Ok(())
    }

    /// Removes the first `n` bytes from the read buffer.
    fn consume(&mut self, n: usize) {
        self.read_buf.drain(0..n);
    }

    /// Waits for the next X11 event and returns it.
    ///
    /// This method blocks until an event is available. It handles both regular events
    /// (32 bytes) and generic events (which have a variable length).
    ///
    /// # Returns
    /// * `Ok(X11Event)` containing the event data.
    /// * `Err(X11Error)` if the connection closes or an error occurs.
    pub fn next_event(&mut self) -> Result<X11Event, X11Error> {
        self.fill_buf(32)?;
        let event_type = self.read_buf[0];

        let total_size = if event_type & 0x7F == EVENT_GENERIC {
            // Generic event: the length is in bytes 4-7 (as 32-bit value in units of 4 bytes)
            self.fill_buf(8)?;
            let extra = u32::from_le_bytes(self.read_buf[4..8].try_into().unwrap()) as usize;
            32 + extra * 4
        } else {
            32
        };

        self.fill_buf(total_size)?;
        let raw_data = self.read_buf[..total_size].to_vec();
        self.consume(total_size);
        Ok(X11Event::new(raw_data))
    }

    /// Sends a request and waits for its matching reply.
    ///
    /// This method sends a request (without a sequence number header) and then reads
    /// replies/events until it finds a reply with the expected sequence number.
    /// Any events received while waiting are discarded (they will be processed later
    /// by `next_event` calls). This is standard X11 practice when waiting for a reply.
    ///
    /// # Arguments
    /// * `req` - The full request data (including opcode and length).
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` containing the full reply (including header).
    /// * `Err(X11Error)` if an X11 error reply is received or the connection fails.
    fn send_request_with_reply(&mut self, req: &[u8]) -> Result<Vec<u8>, X11Error> {
        unsafe { write(self.fd, req)? };
        let seq = self.next_seq;
        self.next_seq += 1;

        loop {
            self.fill_buf(32)?;
            let reply_type = self.read_buf[0];
            if reply_type == X_OP_REPLY {
                let reply_seq = u16::from_le_bytes([self.read_buf[2], self.read_buf[3]]);
                let extra_len =
                    u32::from_le_bytes(self.read_buf[4..8].try_into().unwrap()) as usize * 4;
                let total = 32 + extra_len;
                self.fill_buf(total)?;
                if reply_seq == seq {
                    let data = self.read_buf[..total].to_vec();
                    self.consume(total);
                    return Ok(data);
                } else {
                    // This reply belongs to an older request (should not happen if we
                    // sequence correctly). Discard and continue.
                    self.consume(total);
                }
            } else if reply_type == X_OP_ERROR {
                let err_code = self.read_buf[1];
                let bad_seq = u16::from_le_bytes([self.read_buf[2], self.read_buf[3]]);
                dbg_println!("DEBUG: X11 error {} for sequence {}", err_code, bad_seq);
                self.consume(32);
                return Err(X11Error::XError(err_code));
            } else {
                // It's an event. Discard it completely.
                let total_size = if reply_type & 0x7F == EVENT_GENERIC {
                    self.fill_buf(8)?;
                    32 + u32::from_le_bytes(self.read_buf[4..8].try_into().unwrap()) as usize * 4
                } else {
                    32
                };
                self.fill_buf(total_size)?;
                self.consume(total_size);
            }
        }
    }

    /// Interns an atom (string → ID) on the X server.
    ///
    /// # Arguments
    /// * `name` - The atom name (e.g., "CLIPBOARD").
    /// * `only_if_exists` - If true, the atom will not be created if it doesn't exist.
    ///
    /// # Returns
    /// * `Ok(u32)` atom ID on success.
    /// * `Err(X11Error)` if the request fails.
    pub fn intern_atom(&mut self, name: &str, only_if_exists: bool) -> Result<u32, X11Error> {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len() as u16;
        let req_len = 2 + (name_bytes.len() + 3) / 4; // length in 4-byte units
        let mut req = Vec::with_capacity(req_len * 4);
        req.push(X_OP_INTERN_ATOM);
        req.push(if only_if_exists { 1 } else { 0 });
        req.extend_from_slice(&(req_len as u16).to_le_bytes());
        req.extend_from_slice(&name_len.to_le_bytes());
        req.extend_from_slice(&[0, 0]); // padding
        req.extend_from_slice(name_bytes);
        while req.len() % 4 != 0 {
            req.push(0);
        }

        let reply = self.send_request_with_reply(&req)?;
        if reply.len() < 12 {
            return Err(X11Error::Protocol("InternAtom reply too short"));
        }
        Ok(u32::from_le_bytes(reply[8..12].try_into().unwrap()))
    }

    /// Interns an atom and caches the result.
    ///
    /// # Arguments
    /// * `name` - Atom name (must be `'static` because it's used for debugging).
    /// * `only_if_exists` - Passed to `intern_atom`.
    /// * `field` - A closure that returns a mutable reference to the cache field.
    ///
    /// # Returns
    /// The atom ID (either from cache or newly interned).
    fn intern_atom_cached(
        &mut self,
        name: &'static str,
        only_if_exists: bool,
        mut field: impl for<'a> FnMut(&'a mut Self) -> &'a mut Option<u32>,
    ) -> Result<u32, X11Error> {
        if let Some(id) = *field(self) {
            return Ok(id);
        }
        let id = self.intern_atom(name, only_if_exists)?;
        *field(self) = Some(id);
        Ok(id)
    }

    /// Sends a ConvertSelection request to obtain clipboard data in a specific target format.
    ///
    /// # Arguments
    /// * `selection` - Atom of the selection (e.g., CLIPBOARD).
    /// * `target` - Desired target format (e.g., UTF8_STRING).
    /// * `property` - Atom of a temporary property where the data should be stored.
    ///
    /// # Returns
    /// `Ok(())` if the request was sent successfully. The result will arrive as a
    /// SelectionNotify event later.
    fn send_convert_selection(
        &mut self,
        selection: u32,
        target: u32,
        property: u32,
    ) -> Result<(), X11Error> {
        let requestor = self.our_window;
        let mut req = Vec::with_capacity(24);
        req.push(X_OP_CONVERT_SELECTION);
        req.push(0);
        req.extend_from_slice(&6u16.to_le_bytes()); // request length in 4-byte units
        req.extend_from_slice(&requestor.to_le_bytes());
        req.extend_from_slice(&selection.to_le_bytes());
        req.extend_from_slice(&target.to_le_bytes());
        req.extend_from_slice(&property.to_le_bytes());
        req.extend_from_slice(&0u32.to_le_bytes()); // CurrentTime
        unsafe { write(self.fd, &req)? };
        self.next_seq += 1;
        Ok(())
    }

    /// Retrieves the value of a property (used after ConvertSelection).
    ///
    /// # Arguments
    /// * `window` - The window owning the property (our_window).
    /// * `property` - Atom of the property to read.
    /// * `delete` - Whether to delete the property after reading.
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` containing the property data.
    /// * `Err(X11Error)` if the property does not exist or the reply is malformed.
    fn get_property(
        &mut self,
        window: u32,
        property: u32,
        delete: bool,
    ) -> Result<Vec<u8>, X11Error> {
        let mut req = Vec::with_capacity(24);
        req.push(X_OP_GET_PROPERTY);
        req.push(if delete { 1 } else { 0 });
        req.extend_from_slice(&6u16.to_le_bytes()); // request length
        req.extend_from_slice(&window.to_le_bytes());
        req.extend_from_slice(&property.to_le_bytes());
        req.extend_from_slice(&ANY_PROPERTY_TYPE.to_le_bytes());
        req.extend_from_slice(&0u32.to_le_bytes()); // long-offset (start at 0)
        req.extend_from_slice(&0x1fff_ffff_u32.to_le_bytes()); // long-length (max)

        let reply = self.send_request_with_reply(&req)?;
        if reply.len() < 32 {
            return Err(X11Error::Protocol("GetProperty reply too short"));
        }

        let format = reply[1] as usize; // 0, 8, 16, or 32
        let value_length = u32::from_le_bytes(reply[16..20].try_into().unwrap()) as usize;
        let actual_bytes = if format > 0 {
            value_length * (format / 8)
        } else {
            0
        };

        let total_extra = u32::from_le_bytes(reply[4..8].try_into().unwrap()) as usize * 4;
        if reply.len() < 32 + total_extra {
            return Err(X11Error::Protocol("GetProperty data truncated"));
        }
        if 32 + actual_bytes > reply.len() {
            return Err(X11Error::Protocol(
                "GetProperty value_length overflows reply",
            ));
        }
        Ok(reply[32..32 + actual_bytes].to_vec())
    }

    /// Retrieves the current clipboard contents.
    ///
    /// This method performs the following steps:
    /// 1. Interns required atoms (CLIPBOARD, UTF8_STRING, STRING, and a temporary property).
    /// 2. Attempts to convert the selection to UTF8_STRING, then to STRING.
    /// 3. Waits for a SelectionNotify event.
    /// 4. If the conversion succeeded (property non-zero), reads the property and returns the data.
    ///
    /// # Returns
    /// * `Ok(Vec<u8>)` containing the clipboard data (likely UTF-8 text).
    /// * `Err(X11Error)` if no conversion succeeded or the server returned an error.
    pub fn get_clipboard(&mut self) -> Result<Vec<u8>, X11Error> {
        let clipboard = self.intern_atom_cached("CLIPBOARD", false, |c| &mut c.clipboard_atom)?;
        let utf8_string =
            self.intern_atom_cached("UTF8_STRING", false, |c| &mut c.utf8_string_atom)?;
        let string_atom = self.intern_atom_cached("STRING", false, |c| &mut c.string_atom)?;
        let property =
            self.intern_atom_cached("RUST_CLIP_TEMP", false, |c| &mut c.property_atom)?;

        dbg_println!(
            "DEBUG get_clipboard: CLIPBOARD={} UTF8_STRING={} STRING={} PROP={}",
            clipboard,
            utf8_string,
            string_atom,
            property
        );

        for &target in &[utf8_string, string_atom] {
            dbg_println!(
                "DEBUG get_clipboard: trying ConvertSelection target={}",
                target
            );
            self.send_convert_selection(clipboard, target, property)?;

            let notify = loop {
                let event = self.next_event()?;
                let etype = event.event_type & 0x7F;
                dbg_println!(
                    "DEBUG get_clipboard: got event type={} (raw={})",
                    etype,
                    event.event_type
                );
                if etype == EVENT_SELECTION_NOTIFY {
                    break event;
                }
            };

            let data = &notify.raw_data;
            if data.len() < 32 {
                return Err(X11Error::Protocol("SelectionNotify too short"));
            }

            let notify_property = u32::from_le_bytes(data[20..24].try_into().unwrap());
            let notify_target = u32::from_le_bytes(data[16..20].try_into().unwrap());
            dbg_println!(
                "DEBUG get_clipboard: SelectionNotify target={} property={}",
                notify_target,
                notify_property
            );

            if notify_property == 0 {
                dbg_println!(
                    "DEBUG get_clipboard: conversion refused for target={}, trying next",
                    target
                );
                continue;
            }

            return self.get_property(self.our_window, property, true);
        }

        Err(X11Error::SelectionConversionFailed)
    }

    /// Runs the clipboard monitoring loop.
    ///
    /// This method subscribes to XFIXES selection change events and enters an
    /// infinite loop. On each XFIXES event, it fetches the current clipboard
    /// contents and calls the provided handler.
    ///
    /// # Arguments
    /// * `handler` - A closure that will be called with the clipboard data (as `Vec<u8>`)
    ///   whenever it changes.
    ///
    /// # Returns
    /// * `Ok(())` if the loop exits cleanly (e.g., on `Interrupted`).
    /// * `Err(X11Error)` on fatal errors.
    pub fn run_clipboard_monitor<F>(&mut self, mut handler: F) -> Result<(), X11Error>
    where
        F: FnMut(Vec<u8>),
    {
        self.select_clipboard_events()?;
        let event_base = self.xfixes_event_base.ok_or(X11Error::XFixesNotAvailable)?;
        let xfixes_notify_type = event_base + XFIXES_SELECTION_NOTIFY;
        dbg_println!(
            "DEBUG run_clipboard_monitor: listening, xfixes event type={}",
            xfixes_notify_type
        );

        loop {
            let event = match self.next_event() {
                Ok(e) => e,
                Err(X11Error::Interrupted) => return Ok(()),
                Err(e) => return Err(e),
            };

            let raw_code = event.event_type & 0x7F;
            let etype = if raw_code == EVENT_GENERIC {
                if event.raw_data.len() >= 9 {
                    let ext_ev = event.raw_data[8];
                    event_base.wrapping_add(ext_ev)
                } else if raw_code == 0 {
                    let error_code = event.raw_data.get(1).unwrap_or(&0);
                    dbg_println!(
                        "DEBUG: X11 error {} received. Something is very wrong.",
                        error_code
                    );
                    continue;
                } else {
                    dbg_println!("DEBUG run_clipboard_monitor: generic event too short");
                    continue;
                }
            } else {
                raw_code
            };

            dbg_println!(
                "DEBUG run_clipboard_monitor: event type={} (raw={}, generic_base={})",
                etype,
                event.event_type,
                raw_code
            );

            if etype == xfixes_notify_type {
                dbg_println!(
                    "DEBUG run_clipboard_monitor: XFixes clipboard change detected, fetching..."
                );
                match self.get_clipboard() {
                    Ok(data) => {
                        // Increment metrics (assumes telemetry module is available).
                        telemetry::Metrics::get().inc_clipboard_event_count();
                        handler(data);
                    }
                    Err(e) => {
                        telemetry::Metrics::get().inc_fetch_failed_count();
                        dbg_println!("Failed to retrieve clipboard: {:?}", e);
                    }
                }
            }
        }
    }

    /// Closes the X11 connection explicitly.
    ///
    /// # Returns
    /// `Ok(())` if close succeeded.
    pub fn close(self) -> Result<(), X11Error> {
        unsafe { close(self.fd)? };
        Ok(())
    }
}

impl Drop for X11Connection {
    /// Ensures the socket is closed when the connection is dropped.
    fn drop(&mut self) {
        let _ = unsafe { close(self.fd) };
    }
}

// =============================================================================
// ClipboardBackend Trait Implementation
// =============================================================================

impl ClipboardBackend for X11Connection {
    type Error = X11Error;

    /// Connects to the X server and returns a new `X11Connection`.
    ///
    /// This is a wrapper around `X11Connection::connect()`.
    fn connect() -> Result<Self, Self::Error> {
        X11Connection::connect()
    }

    /// Runs the clipboard monitoring loop.
    ///
    /// This is a wrapper around `run_clipboard_monitor`.
    fn run<F>(&mut self, handler: F) -> Result<(), Self::Error>
    where
        F: FnMut(Vec<u8>),
    {
        self.run_clipboard_monitor(handler)
    }

    /// Determines whether an error is fatal and the backend should not be retried.
    ///
    /// # Fatal errors
    /// - `NoDisplay`: DISPLAY environment variable not set.
    /// - `InvalidDisplay`: DISPLAY is malformed.
    /// - `TcpNotSupported`: Only UNIX sockets are supported.
    /// - `XFixesNotAvailable`: XFIXES extension is required.
    /// - `Protocol("handshake rejected by server")`: Authentication failed or server refused.
    ///
    /// All other errors (e.g., `ConnectionClosed`, `Interrupted`) are considered
    /// potentially transient and may be retried.
    fn is_fatal_error(err: &Self::Error) -> bool {
        use X11Error::*;
        match err {
            NoDisplay | InvalidDisplay(_) | TcpNotSupported | XFixesNotAvailable => true,

            Protocol("handshake rejected by server") => true,

            _ => false,
        }
    }
}
