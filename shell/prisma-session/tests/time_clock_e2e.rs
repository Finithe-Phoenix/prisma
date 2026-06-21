//! End-to-end checks for the time / clock syscalls.
//!
//! These drive the public `dispatch` entry point to validate composed
//! behaviour a real guest depends on: `CLOCK_MONOTONIC` never goes backwards
//! across calls, the wall clocks (`clock_gettime(REALTIME)`, `gettimeofday`,
//! `time`) agree on a plausible epoch, `clock_getres` reports the advertised
//! 1 ns resolution, and a zero-duration `nanosleep` returns promptly — none of
//! which the single-call unit tests can observe.

use prisma_orchestrator::address_space::Protection;
use prisma_orchestrator::guest_memory::GuestRegion;
use prisma_runtime::guest_structs::{Timespec, Timeval};
use prisma_session::syscall_dispatch::{dispatch, SyscallContext};

const SYS_NANOSLEEP: u64 = 35;
const SYS_GETTIMEOFDAY: u64 = 96;
const SYS_TIME: u64 = 201;
const SYS_CLOCK_GETTIME: u64 = 228;
const SYS_CLOCK_GETRES: u64 = 229;

const CLOCK_REALTIME: u64 = 0;
const CLOCK_MONOTONIC: u64 = 1;

// A plausible lower bound for "now" in epoch seconds (2020-09-13), the same
// sanity floor the unit tests use.
const EPOCH_FLOOR: i64 = 1_600_000_000;

fn region(buf: &mut [u8]) -> GuestRegion<'_> {
    GuestRegion::new(0x1000, Protection::ReadWrite, buf)
}

fn read_timespec(mem: &GuestRegion, addr: u64) -> Timespec {
    Timespec::from_guest_bytes(mem.read(addr, Timespec::SIZE).unwrap()).unwrap()
}

#[test]
fn monotonic_clock_never_goes_backwards_across_calls() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 16];
    let mut mem = region(&mut buf);

    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [CLOCK_MONOTONIC, 0x1000, 0, 0, 0, 0]
        ),
        0
    );
    let first = read_timespec(&mem, 0x1000);
    // A second sample, later in real time, must not precede the first.
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [CLOCK_MONOTONIC, 0x1000, 0, 0, 0, 0]
        ),
        0
    );
    let second = read_timespec(&mem, 0x1000);
    let ord = (second.sec, second.nsec) >= (first.sec, first.nsec);
    assert!(ord, "monotonic went backwards: {first:?} -> {second:?}");
}

#[test]
fn the_wall_clocks_agree_on_a_plausible_epoch() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 16];
    let mut mem = region(&mut buf);

    // clock_gettime(REALTIME)
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [CLOCK_REALTIME, 0x1000, 0, 0, 0, 0]
        ),
        0
    );
    let rt = read_timespec(&mem, 0x1000);
    assert!(rt.sec > EPOCH_FLOOR);

    // gettimeofday
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_GETTIMEOFDAY,
            [0x1000, 0, 0, 0, 0, 0]
        ),
        0
    );
    let tv = Timeval::from_guest_bytes(mem.read(0x1000, Timeval::SIZE).unwrap()).unwrap();
    assert!(tv.sec > EPOCH_FLOOR);

    // time(NULL) returns the seconds directly.
    let secs = dispatch(&mut ctx, &mut mem, SYS_TIME, [0, 0, 0, 0, 0, 0]);
    assert!(secs > EPOCH_FLOOR);

    // The three wall-clock sources are sampled moments apart, so they agree to
    // within a small window (generous to absorb scheduling jitter).
    assert!((rt.sec - tv.sec).abs() <= 2);
    assert!((rt.sec - secs).abs() <= 2);
}

#[test]
fn clock_getres_reports_one_nanosecond() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 16];
    let mut mem = region(&mut buf);
    for clk in [CLOCK_REALTIME, CLOCK_MONOTONIC] {
        assert_eq!(
            dispatch(
                &mut ctx,
                &mut mem,
                SYS_CLOCK_GETRES,
                [clk, 0x1000, 0, 0, 0, 0]
            ),
            0
        );
        let res = read_timespec(&mem, 0x1000);
        assert_eq!((res.sec, res.nsec), (0, 1));
    }
}

#[test]
fn zero_nanosleep_returns_promptly_and_unknown_clock_is_einval() {
    let mut ctx = SyscallContext::new();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&Timespec { sec: 0, nsec: 0 }.to_guest_bytes());
    let mut mem = region(&mut buf);
    // A zero request returns success without a meaningful wait.
    assert_eq!(
        dispatch(&mut ctx, &mut mem, SYS_NANOSLEEP, [0x1000, 0, 0, 0, 0, 0]),
        0
    );
    // An unmodelled clock id is rejected with -EINVAL (-22).
    assert_eq!(
        dispatch(
            &mut ctx,
            &mut mem,
            SYS_CLOCK_GETTIME,
            [99, 0x1000, 0, 0, 0, 0]
        ),
        -22
    );
}
