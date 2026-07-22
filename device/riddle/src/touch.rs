//! Raw multitouch gestures for takeover mode.
//! One-finger swipe pages, two-finger drag scrolls, two-finger tap undoes,
//! three-finger tap redoes, and five fingers exits.

use std::io;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

const EV_SYN: u16 = 0;
const SYN_REPORT: u16 = 0;
const EV_ABS: u16 = 3;
const ABS_MT_SLOT: u16 = 47;
const ABS_MT_POSITION_Y: u16 = 54;
const ABS_MT_TRACKING_ID: u16 = 57;
const EVIOCGRAB: libc::c_ulong = 0x40044590;
const MAX_SLOTS: usize = 16;
const SCREEN_H: i32 = 2160;
const TOUCH_MAX_Y: i32 = 2832;
const TAP_SLOP: i32 = 45;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesture {
    Quit,
    Undo,
    Redo,
    /// Positive values move down through the document.
    Scroll(i32),
    /// Direction (+1 down, -1 up); caller chooses page size.
    Page(i32),
}

/// Child-safety: five-finger exit must be HELD, not tapped — a toddler's palm
/// slap reaches five contacts for an instant all the time. Hold duration from
/// RIDDLE_QUIT_HOLD_SECS (default 3; 0 restores the legacy instant tap).
struct QuitArm {
    hold: Duration,
    since: Option<Instant>,
    fired: bool,
}

impl QuitArm {
    fn new(hold: Duration) -> Self {
        Self { hold, since: None, fired: false }
    }

    fn from_env() -> Self {
        let secs = std::env::var("RIDDLE_QUIT_HOLD_SECS")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|s| *s >= 0.0)
            .unwrap_or(3.0);
        Self::new(Duration::from_millis((secs * 1000.0) as u64))
    }

    fn update(&mut self, finger_count: usize, now: Instant) -> bool {
        if finger_count < 5 {
            self.since = None;
            self.fired = false;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if !self.fired && now.duration_since(since) >= self.hold {
            self.fired = true;
            return true;
        }
        false
    }
}

/// Check if five-finger hold should fire quit. Counts active slots and updates arm.
/// Extracted for testability — called both from finish_frame (on touch events) and
/// drain's post-read tick (every ~2ms poll) so stationary holds advance the timer.
fn tick_quit(slots: &[Slot; MAX_SLOTS], arm: &mut QuitArm, now: Instant) -> bool {
    let count = slots.iter().filter(|s| s.active).count();
    arm.update(count, now)
}

#[derive(Clone, Copy, Default)]
struct Slot {
    active: bool,
    start_y: i32,
    y: i32,
}

pub struct TouchDevice {
    fd: RawFd,
    slots: [Slot; MAX_SLOTS],
    cur: usize,
    max_fingers: usize,
    frame_y: Option<i32>,
    total_motion: i32,
    quit_arm: QuitArm,
}

impl TouchDevice {
    pub fn open() -> io::Result<Self> {
        for i in 0..8 {
            let name_path = format!("/sys/class/input/event{i}/device/name");
            if let Ok(name) = std::fs::read_to_string(&name_path) {
                if name.to_lowercase().contains("touch") {
                    let path = std::ffi::CString::new(format!("/dev/input/event{i}")).unwrap();
                    let fd =
                        unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NONBLOCK) };
                    if fd < 0 {
                        return Err(io::Error::last_os_error());
                    }
                    unsafe { libc::ioctl(fd, EVIOCGRAB, 1i32) };
                    return Ok(Self {
                        fd,
                        slots: [Slot::default(); MAX_SLOTS],
                        cur: 0,
                        max_fingers: 0,
                        frame_y: None,
                        total_motion: 0,
                        quit_arm: QuitArm::from_env(),
                    });
                }
            }
        }
        Err(io::Error::new(io::ErrorKind::NotFound, "no touch device"))
    }

    /// Drain and discard touch input, then cancel every partial gesture. Used
    /// for palm rejection while the marker is in digitizer proximity.
    pub fn suppress(&mut self) {
        let _ = self.drain();
        self.slots = [Slot::default(); MAX_SLOTS];
        self.max_fingers = 0;
        self.frame_y = None;
        self.total_motion = 0;
        self.quit_arm.since = None;
        self.quit_arm.fired = false;
    }

    /// Compatibility helper for takeover apps that only use five-finger exit.
    pub fn drain_check_quit(&mut self) -> bool {
        self.drain().contains(&Gesture::Quit)
    }

    pub fn drain(&mut self) -> Vec<Gesture> {
        let mut out = Vec::new();
        let mut buf = [0u8; 24 * 64];
        loop {
            let n =
                unsafe { libc::read(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if n <= 0 {
                break;
            }
            for chunk in buf[..n as usize].chunks_exact(24) {
                let etype = u16::from_le_bytes(chunk[16..18].try_into().unwrap());
                let code = u16::from_le_bytes(chunk[18..20].try_into().unwrap());
                let value = i32::from_le_bytes(chunk[20..24].try_into().unwrap());
                if etype == EV_ABS && code == ABS_MT_SLOT {
                    self.cur = (value.max(0) as usize).min(MAX_SLOTS - 1);
                } else if etype == EV_ABS && code == ABS_MT_POSITION_Y {
                    self.slots[self.cur].y = value;
                    if self.slots[self.cur].active && self.slots[self.cur].start_y == i32::MIN {
                        self.slots[self.cur].start_y = value;
                    }
                } else if etype == EV_ABS && code == ABS_MT_TRACKING_ID {
                    if value != -1 {
                        self.slots[self.cur] = Slot {
                            active: true,
                            start_y: i32::MIN,
                            y: self.slots[self.cur].y,
                        };
                    } else {
                        self.slots[self.cur].active = false;
                    }
                } else if etype == EV_SYN && code == SYN_REPORT {
                    self.finish_frame(&mut out);
                }
            }
        }
        // Tick the quit timer even without new touch events. The main loop's
        // ~2ms drain_check_quit() poll advances the hold timer for stationary presses.
        if tick_quit(&self.slots, &mut self.quit_arm, Instant::now()) {
            out.push(Gesture::Quit);
        }
        out
    }

    fn finish_frame(&mut self, out: &mut Vec<Gesture>) {
        let active: Vec<Slot> = self.slots.iter().copied().filter(|s| s.active).collect();
        let count = active.len();
        self.max_fingers = self.max_fingers.max(count);
        if tick_quit(&self.slots, &mut self.quit_arm, Instant::now()) {
            out.push(Gesture::Quit);
        }

        let average_y = (count > 0).then(|| active.iter().map(|s| s.y).sum::<i32>() / count as i32);
        if let (Some(previous), Some(current)) = (self.frame_y, average_y) {
            let raw_delta = previous - current;
            self.total_motion += raw_delta.abs();
            if count == 2 {
                let pixels = raw_delta * SCREEN_H / TOUCH_MAX_Y;
                if pixels != 0 {
                    out.push(Gesture::Scroll(pixels));
                }
            }
        }
        self.frame_y = average_y;

        if count == 0 && self.max_fingers > 0 {
            if self.total_motion < TAP_SLOP {
                match self.max_fingers {
                    2 => out.push(Gesture::Undo),
                    3 => out.push(Gesture::Redo),
                    _ => {}
                }
            } else if self.max_fingers == 1 {
                // Released slots retain their coordinates.
                if let Some(slot) = self
                    .slots
                    .iter()
                    .max_by_key(|slot| (slot.start_y - slot.y).abs())
                {
                    let delta = slot.start_y - slot.y;
                    if delta.abs() >= TAP_SLOP {
                        out.push(Gesture::Page(delta.signum()));
                    }
                }
            }
            self.max_fingers = 0;
            self.frame_y = None;
            self.total_motion = 0;
        }
    }
}

impl Drop for TouchDevice {
    fn drop(&mut self) {
        unsafe {
            libc::ioctl(self.fd, EVIOCGRAB, 0i32);
            libc::close(self.fd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_requires_sustained_five_fingers() {
        let mut arm = QuitArm::new(Duration::from_secs(3));
        let t0 = Instant::now();
        assert!(!arm.update(5, t0)); // 刚满 5 指：不触发
        assert!(!arm.update(5, t0 + Duration::from_millis(2900))); // 未满 3s
        assert!(arm.update(5, t0 + Duration::from_millis(3001))); // 满 3s：触发
        assert!(!arm.update(5, t0 + Duration::from_millis(3200))); // 已触发过，不重复
    }

    #[test]
    fn quit_rearms_only_after_release() {
        let mut arm = QuitArm::new(Duration::from_secs(3));
        let t0 = Instant::now();
        arm.update(5, t0);
        assert!(!arm.update(3, t0 + Duration::from_millis(1000))); // 掉到 3 指：计时清零
        assert!(!arm.update(5, t0 + Duration::from_millis(1500))); // 重新满 5 指从头计
        assert!(!arm.update(5, t0 + Duration::from_millis(4400))); // 距重满仅 2.9s
        assert!(arm.update(5, t0 + Duration::from_millis(4600))); // 3.1s：触发
    }

    #[test]
    fn zero_hold_fires_immediately_for_legacy_mode() {
        let mut arm = QuitArm::new(Duration::ZERO);
        assert!(arm.update(5, Instant::now()));
    }

    #[test]
    fn stationary_hold_fires_via_tick_without_new_frames() {
        let mut slots = [Slot::default(); MAX_SLOTS];
        for s in slots.iter_mut().take(5) {
            s.active = true;
        }
        let mut arm = QuitArm::new(Duration::from_secs(3));
        let t0 = Instant::now();
        assert!(!tick_quit(&slots, &mut arm, t0));
        assert!(tick_quit(&slots, &mut arm, t0 + Duration::from_millis(3100)));
        assert!(
            !tick_quit(&slots, &mut arm, t0 + Duration::from_millis(3200)),
            "fired latch holds"
        );
    }
}
