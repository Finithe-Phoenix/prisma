//! Terminal `ioctl` requests.
//!
//! Only the small tty subset programs use to detect a terminal and read its
//! size/attributes is modelled. The standard streams (stdin/stdout/stderr) are
//! presented as a terminal; a regular file is not, so a tty request on it is
//! `ENOTTY` — exactly how `isatty()` (a `TCGETS` probe) tells the two apart.
//! Pointers are checked through a [`GuestRegion`].

use prisma_orchestrator::address_space::RangeError;
use prisma_orchestrator::guest_mem::GuestMem;
use prisma_runtime::fd_table::{FdEntry, FdTable};
use prisma_runtime::guest_structs::{Termios, Winsize};

/// `TCGETS` — read terminal attributes (also how `isatty` probes for a tty).
const TCGETS: u64 = 0x5401;
/// `TCSETS`/`TCSETSW`/`TCSETSF` — set terminal attributes.
const TCSETS: u64 = 0x5402;
const TCSETSW: u64 = 0x5403;
const TCSETSF: u64 = 0x5404;
/// `TIOCGWINSZ` — read the terminal window size.
const TIOCGWINSZ: u64 = 0x5413;

/// The window size reported for a terminal: a conventional 80x24, no pixel size.
const TERM_ROWS: u16 = 24;
const TERM_COLS: u16 = 80;

/// Why an `ioctl` failed (each maps to a guest errno at routing time).
#[derive(Debug)]
pub enum IoctlError {
    /// `fd` is not open — guest `EBADF`.
    BadFd,
    /// The fd is not a terminal, or the request is not modelled — guest `ENOTTY`.
    NotTty,
    /// A pointer argument is not accessible guest memory — guest `EFAULT`.
    Fault(RangeError),
}

/// A conventional cooked-mode `termios` (the attributes a fresh login terminal
/// has): canonical input with echo, CR/NL translation, 8-bit characters.
fn default_termios() -> Termios {
    // c_cc defaults at the standard indices (VINTR, VQUIT, VERASE, ...).
    let mut cc = [0u8; Termios::NCCS];
    cc[0] = 3; // VINTR  (^C)
    cc[1] = 28; // VQUIT  (^\)
    cc[2] = 127; // VERASE (DEL)
    cc[3] = 21; // VKILL  (^U)
    cc[4] = 4; // VEOF   (^D)
    cc[6] = 1; // VMIN
    cc[8] = 17; // VSTART (^Q)
    cc[9] = 19; // VSTOP  (^S)
    cc[10] = 26; // VSUSP  (^Z)
    cc[12] = 18; // VREPRINT (^R)
    cc[13] = 15; // VDISCARD (^O)
    cc[14] = 23; // VWERASE  (^W)
    cc[15] = 22; // VLNEXT   (^V)
    Termios {
        iflag: 0x0000_0500, // ICRNL | IXON
        oflag: 0x0000_0005, // OPOST | ONLCR
        cflag: 0x0000_04bf, // B38400 | CS8 | CREAD | HUPCL
        lflag: 0x0000_8a3b, // ISIG|ICANON|ECHO|ECHOE|ECHOK|ECHOCTL|ECHOKE|IEXTEN
        line: 0,
        cc,
    }
}

/// `ioctl(fd, request, argp)`: the modelled terminal requests.
///
/// - `TIOCGWINSZ` writes an 80x24 [`Winsize`].
/// - `TCGETS` writes the default [`Termios`] (and is how `isatty` succeeds).
/// - `TCSETS`/`TCSETSW`/`TCSETSF` accept (and range-check) a new `termios`
///   without storing it — terminal modes are not modelled.
///
/// Every modelled request requires `fd` to be a terminal (a standard stream);
/// on a regular file, or for an unmodelled request, the result is `ENOTTY`.
///
/// # Errors
/// [`IoctlError::BadFd`] for an unopen fd, [`IoctlError::NotTty`] for a non-tty
/// fd or an unmodelled request, [`IoctlError::Fault`] for a bad pointer.
pub fn ioctl(
    fds: &FdTable,
    mem: &mut impl GuestMem,
    fd: i32,
    request: u64,
    argp: u64,
) -> Result<i64, IoctlError> {
    let entry = fds.get(fd).ok_or(IoctlError::BadFd)?;
    let is_tty = matches!(entry, FdEntry::Stdin | FdEntry::Stdout | FdEntry::Stderr);
    if !is_tty {
        return Err(IoctlError::NotTty);
    }
    match request {
        TIOCGWINSZ => {
            let ws = Winsize {
                row: TERM_ROWS,
                col: TERM_COLS,
                xpixel: 0,
                ypixel: 0,
            };
            mem.write(argp, &ws.to_guest_bytes())
                .map_err(IoctlError::Fault)?;
            Ok(0)
        }
        TCGETS => {
            mem.write(argp, &default_termios().to_guest_bytes())
                .map_err(IoctlError::Fault)?;
            Ok(0)
        }
        TCSETS | TCSETSW | TCSETSF => {
            // Validate the new attributes are readable, then accept them.
            mem.read(argp, Termios::SIZE).map_err(IoctlError::Fault)?;
            Ok(0)
        }
        _ => Err(IoctlError::NotTty),
    }
}

#[cfg(test)]
mod tests {
    use super::{ioctl, IoctlError};
    use prisma_orchestrator::address_space::Protection;
    use prisma_orchestrator::guest_memory::GuestRegion;
    use prisma_runtime::fd_table::{FdEntry, FdTable};
    use prisma_runtime::guest_structs::{Termios, Winsize};

    const TCGETS: u64 = 0x5401;
    const TCSETS: u64 = 0x5402;
    const TIOCGWINSZ: u64 = 0x5413;

    fn region(buf: &mut [u8]) -> GuestRegion<'_> {
        GuestRegion::new(0x1000, Protection::ReadWrite, buf)
    }

    #[test]
    fn tiocgwinsz_reports_eighty_by_twentyfour_on_a_terminal() {
        let fds = FdTable::new(); // stdin/out/err open
        let mut buf = [0u8; Winsize::SIZE];
        let mut mem = region(&mut buf);
        // fd 1 (stdout) is a terminal.
        assert_eq!(ioctl(&fds, &mut mem, 1, TIOCGWINSZ, 0x1000).unwrap(), 0);
        let ws = Winsize::from_guest_bytes(mem.read(0x1000, Winsize::SIZE).unwrap()).unwrap();
        assert_eq!((ws.row, ws.col), (24, 80));
    }

    #[test]
    fn tcgets_succeeds_on_a_tty_and_is_enotty_on_a_file() {
        let mut fds = FdTable::new();
        let mut buf = [0u8; Termios::SIZE];
        let mut mem = region(&mut buf);
        // TCGETS on stdout (a tty) succeeds -> isatty() is true.
        assert_eq!(ioctl(&fds, &mut mem, 1, TCGETS, 0x1000).unwrap(), 0);
        let t = Termios::from_guest_bytes(mem.read(0x1000, Termios::SIZE).unwrap()).unwrap();
        assert_ne!(t.lflag, 0); // a cooked-mode termios, not blank
                                // A regular file is not a terminal -> ENOTTY.
        let path = std::env::temp_dir().join(format!("prisma_ioctl_{}.tmp", std::process::id()));
        let file = std::fs::File::create(&path).unwrap();
        let fd = fds.allocate(FdEntry::File(file)).unwrap();
        assert!(matches!(
            ioctl(&fds, &mut mem, fd, TCGETS, 0x1000),
            Err(IoctlError::NotTty)
        ));
        assert!(fds.close(fd));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tcsets_accepts_attributes_and_unknown_request_is_enotty() {
        let fds = FdTable::new();
        let mut buf = [0u8; Termios::SIZE];
        let mut mem = region(&mut buf);
        // TCSETS reads + accepts the new termios.
        assert_eq!(ioctl(&fds, &mut mem, 0, TCSETS, 0x1000).unwrap(), 0);
        // An unmodelled request on a tty is ENOTTY.
        assert!(matches!(
            ioctl(&fds, &mut mem, 0, 0x1234, 0x1000),
            Err(IoctlError::NotTty)
        ));
    }

    #[test]
    fn ioctl_reports_ebadf_for_an_unopen_fd() {
        let fds = FdTable::new();
        let mut buf = [0u8; 16];
        let mut mem = region(&mut buf);
        assert!(matches!(
            ioctl(&fds, &mut mem, 99, TIOCGWINSZ, 0x1000),
            Err(IoctlError::BadFd)
        ));
    }
}
