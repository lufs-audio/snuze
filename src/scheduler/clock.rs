use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, SystemTime};

/// Injectable clock abstraction for the scheduler.
pub trait Clock: Send + Sync {
    /// Returns current time.
    fn now(&self) -> SystemTime;

    /// Blocks the calling thread until `deadline` or until externally woken.
    fn wait_until(&self, deadline: SystemTime);

    /// Wakes any thread currently blocked in `wait_until`.
    fn wake(&self);
}

/// Production clock implementation backed by `std::time::SystemTime` and condvar sleep.
#[derive(Debug, Default)]
pub struct SystemClock {
    flag: Arc<Mutex<bool>>,
    cvar: Arc<Condvar>,
}

impl SystemClock {
    /// Creates a new `SystemClock`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(Mutex::new(false)),
            cvar: Arc::new(Condvar::new()),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn wait_until(&self, deadline: SystemTime) {
        let mut flag = match self.flag.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };

        if *flag {
            *flag = false;
            return;
        }

        while !*flag {
            let now = self.now();
            if now >= deadline {
                break;
            }

            let Ok(dur) = deadline.duration_since(now) else {
                break;
            };

            if dur.is_zero() {
                break;
            }

            let (new_flag, timeout_res) = match self.cvar.wait_timeout(flag, dur) {
                Ok(res) => res,
                Err(e) => e.into_inner(),
            };
            flag = new_flag;
            if timeout_res.timed_out() {
                break;
            }
        }

        *flag = false;
    }

    fn wake(&self) {
        let mut flag = match self.flag.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *flag = true;
        self.cvar.notify_all();
    }
}

/// Fake clock for deterministic unit testing without real-time delays.
#[derive(Debug, Clone)]
pub struct FakeClock {
    time: Arc<Mutex<SystemTime>>,
    flag: Arc<Mutex<bool>>,
    cvar: Arc<Condvar>,
}

impl FakeClock {
    /// Creates a new `FakeClock` initialized at `start_time`.
    #[must_use]
    pub fn new(start_time: SystemTime) -> Self {
        Self {
            time: Arc::new(Mutex::new(start_time)),
            flag: Arc::new(Mutex::new(false)),
            cvar: Arc::new(Condvar::new()),
        }
    }

    /// Advances the fake clock by `dur` and wakes waiters.
    pub fn advance(&self, dur: Duration) {
        {
            let mut t = match self.time.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            *t += dur;
        }
        self.wake();
    }

    /// Explicitly sets the fake clock time and wakes waiters.
    pub fn set_time(&self, new_time: SystemTime) {
        {
            let mut t = match self.time.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            *t = new_time;
        }
        self.wake();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        match self.time.lock() {
            Ok(g) => *g,
            Err(e) => *e.into_inner(),
        }
    }

    fn wait_until(&self, deadline: SystemTime) {
        let mut flag = match self.flag.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };

        if *flag {
            *flag = false;
            return;
        }

        while !*flag {
            let current = self.now();
            if current >= deadline {
                break;
            }

            flag = match self.cvar.wait(flag) {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
        }

        *flag = false;
    }

    fn wake(&self) {
        let mut flag = match self.flag.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        *flag = true;
        self.cvar.notify_all();
    }
}
