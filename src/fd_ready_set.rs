//! Descriptor readiness: the descriptors a caller waits on, and whether each is watched for
//! readability or writability.

use libc::c_int;
use std::os::unix::prelude::*;
use std::time::Duration;

#[derive(Clone, Copy)]
pub enum Timeout {
    Duration(Duration),
    Forever,
}

impl Timeout {
    pub const ZERO: Timeout = Timeout::Duration(Duration::ZERO);

    /// Convert from usecs to poll-friendly msecs.
    fn as_poll_msecs(&self) -> c_int {
        match self {
            // Negative values mean wait forever in poll-speak.
            Timeout::Forever => -1 as c_int,
            Timeout::Duration(duration) => {
                assert!(
                    duration.as_millis() < c_int::MAX as _,
                    "Timeout too long but not forever!"
                );
                duration.as_millis() as c_int
            }
        }
    }
}

/// The readiness a descriptor is watched for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Readiness {
    /// Bytes are available, or the peer closed the descriptor.
    Readable,

    /// Space is available, or the peer closed the descriptor.
    Writable,
}

/// Returns `true` if the fd is or becomes readable within the given timeout.
/// This returns `false` if the waiting is interrupted by a signal.
pub fn is_fd_readable(fd: i32, timeout: Timeout) -> bool {
    FdReadySet::is_fd_ready(fd, Readiness::Readable, timeout)
}

/// Returns whether an fd reached the given readiness, without blocking.
pub fn poll_fd_ready(fd: i32, readiness: Readiness) -> bool {
    FdReadySet::poll_fd_ready(fd, readiness)
}

/// Returns whether an fd is readable.
pub fn poll_fd_readable(fd: i32) -> bool {
    FdReadySet::poll_fd_ready(fd, Readiness::Readable)
}

/// Returns whether an fd is writable.
pub fn poll_fd_writable(fd: i32) -> bool {
    FdReadySet::poll_fd_ready(fd, Readiness::Writable)
}

/// A set of descriptors with the readiness each is watched for, checked by `poll` or `select`.
#[cfg(apple)]
pub struct FdReadySet {
    // The underlying fdsets and nfds value to pass to select().
    read_set_: libc::fd_set,
    write_set_: libc::fd_set,
    nfds_: c_int,
}

#[cfg(apple)]
impl FdReadySet {
    /// Construct an empty set.
    pub fn new() -> FdReadySet {
        let mut read_set_ = std::mem::MaybeUninit::uninit();
        let mut write_set_ = std::mem::MaybeUninit::uninit();
        let (read_set_, write_set_) = unsafe {
            libc::FD_ZERO(read_set_.as_mut_ptr());
            libc::FD_ZERO(write_set_.as_mut_ptr());
            (read_set_.assume_init(), write_set_.assume_init())
        };
        FdReadySet {
            read_set_,
            write_set_,
            nfds_: 0,
        }
    }

    /// Reset back to an empty set.
    pub fn clear(&mut self) {
        self.nfds_ = 0;
        unsafe {
            libc::FD_ZERO(&mut self.read_set_);
            libc::FD_ZERO(&mut self.write_set_);
        }
    }

    /// Add an fd to the set. The fd is ignored if negative (for convenience).
    pub fn add(&mut self, fd: RawFd, readiness: Readiness) {
        if fd >= (libc::FD_SETSIZE as RawFd) {
            return;
        }
        if fd >= 0 {
            match readiness {
                Readiness::Readable => unsafe { libc::FD_SET(fd, &mut self.read_set_) },
                Readiness::Writable => unsafe { libc::FD_SET(fd, &mut self.write_set_) },
            }
            self.nfds_ = std::cmp::max(self.nfds_, fd + 1);
        }
    }

    /// Returns `true` if the given `fd` is marked ready for the readiness it was watched for.
    pub fn test(&self, fd: RawFd, readiness: Readiness) -> bool {
        if fd < 0 {
            return false;
        }
        match readiness {
            Readiness::Readable => unsafe { libc::FD_ISSET(fd, &self.read_set_) },
            Readiness::Writable => unsafe { libc::FD_ISSET(fd, &self.write_set_) },
        }
    }

    /// Call `select()`. Note this destructively modifies the sets. Return the result of
    /// `select()`.
    pub fn check(&mut self, timeout: Timeout) -> c_int {
        let mut tvs;
        let timeout = match timeout {
            Timeout::Forever => std::ptr::null_mut(),
            Timeout::Duration(duration) => {
                tvs = libc::timeval {
                    tv_sec: duration.as_secs() as libc::time_t,
                    tv_usec: duration.subsec_micros() as libc::suseconds_t,
                };
                &mut tvs
            }
        };
        unsafe {
            libc::select(
                self.nfds_,
                &mut self.read_set_,
                &mut self.write_set_,
                std::ptr::null_mut(),
                timeout,
            )
        }
    }

    /// Check if a single fd reaches the given readiness, with a given timeout.
    /// Returns `true` if ready, `false` otherwise.
    pub fn is_fd_ready(fd: RawFd, readiness: Readiness, timeout: Timeout) -> bool {
        if fd < 0 {
            return false;
        }
        let mut s = Self::new();
        s.add(fd, readiness);
        let res = s.check(timeout);
        res > 0 && s.test(fd, readiness)
    }

    /// Check if a single fd is ready without blocking.
    pub fn poll_fd_ready(fd: RawFd, readiness: Readiness) -> bool {
        Self::is_fd_ready(fd, readiness, Timeout::ZERO)
    }
}

#[cfg(not(apple))]
pub struct FdReadySet {
    pollfds_: Vec<libc::pollfd>,
}

#[cfg(not(apple))]
impl FdReadySet {
    /// Construct an empty set.
    pub fn new() -> FdReadySet {
        FdReadySet {
            pollfds_: Vec::new(),
        }
    }

    /// Reset back to an empty set.
    pub fn clear(&mut self) {
        self.pollfds_.clear();
    }

    #[inline]
    fn pollfd_get_fd(pollfd: &libc::pollfd) -> RawFd {
        pollfd.fd
    }

    /// Add an fd to the set. The fd is ignored if negative (for convenience). A second readiness
    /// for the same fd extends the events watched for it.
    pub fn add(&mut self, fd: RawFd, readiness: Readiness) {
        if fd < 0 {
            return;
        }
        let events = match readiness {
            Readiness::Readable => libc::POLLIN,
            Readiness::Writable => libc::POLLOUT,
        };
        let pos = match self.pollfds_.binary_search_by_key(&fd, Self::pollfd_get_fd) {
            Ok(pos) => pos,
            Err(pos) => {
                self.pollfds_.insert(
                    pos,
                    libc::pollfd {
                        fd,
                        events,
                        revents: 0,
                    },
                );
                return;
            }
        };
        self.pollfds_[pos].events |= events;
    }

    /// Returns `true` if the given `fd` reached the readiness it was watched for, or was HUP'd.
    pub fn test(&self, fd: RawFd, readiness: Readiness) -> bool {
        let Ok(pos) = self.pollfds_.binary_search_by_key(&fd, Self::pollfd_get_fd) else {
            return false;
        };
        let pollfd = &self.pollfds_[pos];
        debug_assert_eq!(pollfd.fd, fd);
        let ready = match readiness {
            Readiness::Readable => libc::POLLIN,
            Readiness::Writable => libc::POLLOUT,
        };
        pollfd.revents & (ready | libc::POLLHUP | libc::POLLERR) != 0
    }

    fn do_poll(fds: &mut [libc::pollfd], timeout: Timeout) -> c_int {
        let count = fds.len();
        assert!(count <= libc::nfds_t::MAX as usize, "count too big");
        unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                count as libc::nfds_t,
                timeout.as_poll_msecs(),
            )
        }
    }

    /// Call `poll()`. Note this destructively modifies the set. Return the result of `poll()`.
    pub fn check(&mut self, timeout: Timeout) -> c_int {
        if self.pollfds_.is_empty() {
            return 0;
        }
        Self::do_poll(&mut self.pollfds_, timeout)
    }

    /// Check if a single fd reaches the given readiness, with a given timeout.
    /// Return true if the fd is ready, false otherwise.
    pub fn is_fd_ready(fd: RawFd, readiness: Readiness, timeout: Timeout) -> bool {
        if fd < 0 {
            return false;
        }
        let events = match readiness {
            Readiness::Readable => libc::POLLIN,
            Readiness::Writable => libc::POLLOUT,
        };
        let mut pfd = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let ret = Self::do_poll(std::slice::from_mut(&mut pfd), timeout);
        ret > 0 && (pfd.revents & events) != 0
    }

    /// Check if a single fd is ready without blocking.
    pub fn poll_fd_ready(fd: RawFd, readiness: Readiness) -> bool {
        Self::is_fd_ready(fd, readiness, Timeout::ZERO)
    }
}
