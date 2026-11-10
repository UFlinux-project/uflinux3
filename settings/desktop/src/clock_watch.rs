//! Wake the GTK clock when the system's realtime clock is changed.
use std::{
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::net::UnixStream,
    },
    thread,
};

pub struct ClockWatch(UnixStream);

impl ClockWatch {
    pub fn start() -> io::Result<(Self, async_channel::Receiver<()>)> {
        let raw = unsafe { libc::timerfd_create(libc::CLOCK_REALTIME, libc::TFD_CLOEXEC) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        let timer = unsafe { OwnedFd::from_raw_fd(raw) };
        let (wake, reader) = UnixStream::pair()?;
        let (sender, receiver) = async_channel::bounded(1);
        thread::Builder::new()
            .name("desktop-clock-watch".into())
            .spawn(move || loop {
                let mut now = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 0,
                };
                if unsafe { libc::clock_gettime(libc::CLOCK_REALTIME, &mut now) } != 0 {
                    break;
                }
                let alarm = libc::itimerspec {
                    it_interval: libc::timespec {
                        tv_sec: 0,
                        tv_nsec: 0,
                    },
                    it_value: libc::timespec {
                        tv_sec: now.tv_sec.saturating_add(86_400),
                        tv_nsec: now.tv_nsec,
                    },
                };
                if unsafe {
                    libc::timerfd_settime(
                        timer.as_raw_fd(),
                        libc::TFD_TIMER_ABSTIME | libc::TFD_TIMER_CANCEL_ON_SET,
                        &alarm,
                        std::ptr::null_mut(),
                    )
                } != 0
                {
                    break;
                }
                let mut fds = [
                    libc::pollfd {
                        fd: timer.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: reader.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                loop {
                    let ready =
                        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
                    if ready < 0 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted
                    {
                        continue;
                    }
                    if ready < 0 || fds[1].revents != 0 {
                        return;
                    }
                    break;
                }
                let mut count = 0u64;
                let read = unsafe {
                    libc::read(
                        timer.as_raw_fd(),
                        (&mut count as *mut u64).cast(),
                        std::mem::size_of::<u64>(),
                    )
                };
                if read < 0 {
                    if io::Error::last_os_error().raw_os_error() == Some(libc::ECANCELED) {
                        let _ = sender.try_send(());
                    } else {
                        break;
                    }
                }
            })?;
        Ok((Self(wake), receiver))
    }
}
impl Drop for ClockWatch {
    fn drop(&mut self) {
        let _ = self.0.write(&[1]);
    }
}
