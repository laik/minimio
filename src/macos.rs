use std::net;
use std::ptr;
use std::time::Duration;
use std::io::{self, Read, Write, IoSliceMut};
use std::os::unix::io::{AsRawFd, RawFd};
use crate::{ID, Events, interests::Interests};

#[derive(Debug)]
pub struct Selector {
    id: usize,
    kq: RawFd,
}

impl Selector {
    fn new_with_id(id: usize) -> io::Result<Self> {
        Ok(Selector {
            id,
            kq: ffi::queue()?,
        })
    }

    fn new() -> io::Result<Self> {
        Selector::new_with_id(ID.next())
    }

    pub fn id(&self) -> usize {
        self.id
    }

    /// This function blocks and waits until an event has been recieved. It never times out.
    pub fn select(&self, events: &mut Events) -> io::Result<()> {
        // TODO: get n_events from self
        let n_events = events.len() as i32;
        events.clear();
        ffi::syscall_kevent(self.kq, &[], events, n_events, None)
        .map(|n_events| {
            // This is safe because `syscall_kevent` ensures that `n_events` are
            // assigned. We could check for a valid token for each event to verify so this is
            // just a performance optimization used in `mio` and copied here.
            unsafe { events.set_len(n_events as usize) };
        })
    }

    pub fn register(&self, fd: RawFd, id: usize, interests: Interests) -> io::Result<()> {
        let flags = ffi::EV_ADD | ffi::EV_ENABLE |  ffi::EV_ONESHOT;
 
        if interests.is_readable() {
            // We register the id (or most oftenly referred to as a Token) to the `udata` field
            // if the `Kevent`
            let kevent = ffi::Event::new_read_event(fd, id as u64);
            let kevent = [kevent];
            ffi::syscall_kevent(self.kq, &kevent, &mut [], 0, None)?;
        };

        if interests.is_writable() {
            unimplemented!();
        }

        Ok(())
    }
}

pub type Event = ffi::Kevent;

pub struct TcpStream {
    inner: net::TcpStream,
}

impl TcpStream {
    pub fn connect(adr: impl net::ToSocketAddrs) -> io::Result<Self> {
        // actually we should set this to non-blocking before we call connect which is not something
        // we get from the stdlib but could do with a syscall. Let's skip that step in this example. 
        // In other words this will block shortly establishing a connection to the remote server
        let stream = net::TcpStream::connect(adr)?;
        stream.set_nonblocking(true)?;

        Ok(TcpStream {
            inner: stream,
        })
    }
}

impl<'a> Read for &'a TcpStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // If we let the socket operate non-blocking we could get an error of kind `WouldBlock`, 
        // that means there is more data to read but we would block if we waited for it to arrive.
        // The right thing to do is to re-register the event, getting notified once more
        // data is available. We'll not do that in our implementation since we're making an example
        // and instead we make the socket blocking again while we read from it
        self.inner.set_nonblocking(false)?;
        (&self.inner).read(buf)
    }

    /// Copies data to fill each buffer in order, with the final buffer possibly only beeing 
    /// partially filled. Now as we'll see this is like it's made for our use case when abstracting
    /// over IOCP AND epoll/kqueue (since we need to buffer anyways).
    /// 
    /// IoSliceMut is like `&mut [u8]` but it's guaranteed to be ABI compatible with the `iovec` 
    /// type on unix platforms and `WSABUF` on Windows. Perfect for us.
    fn read_vectored(&mut self, bufs: &mut [IoSliceMut]) -> io::Result<usize> {
        (&self.inner).read_vectored(bufs)
    }
}

impl Write for TcpStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl AsRawFd for TcpStream {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.as_raw_fd()
    }
}

mod ffi {
    use super::*;

    pub const EVFILT_READ: i16 = -1;
    pub const EV_ADD: u16 = 0x1;
    pub const EV_ENABLE: u16 = 0x4;
    pub const EV_ONESHOT: u16 = 0x10;

    pub type Event = Kevent;
    impl Event {
        pub fn new_read_event(fd: RawFd, id: u64) -> Self {
            Event {
            ident: fd as u64,
            filter: EVFILT_READ,
            flags: EV_ADD | EV_ENABLE | EV_ONESHOT,
            fflags: 0,
            data: 0,
            udata: id,
        }
        }

        pub fn zero() -> Self {
            Event {
                ident: 0,
                filter: 0,
                flags: 0,
                fflags: 0,
                data: 0,
                udata: 0,
            }
        }
    }

    pub fn queue() -> io::Result<i32> {
        let fd = unsafe { ffi::kqueue() };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }

    pub fn syscall_kevent (
        kq: RawFd,
        cl: &[Kevent],
        el: &mut [Kevent],
        n_events: i32,
        timeout: Option<usize>,
    ) -> io::Result<usize> {
        let res = unsafe {
            let kq = kq as i32;
            // TODO: check if 0 is infinite timeout
            let timeout = timeout.unwrap_or(0);
            let cl_len = cl.len() as i32;
            let el_len = el.len() as i32;
            kevent(kq, cl.as_ptr(), cl_len, el.as_mut_ptr(), n_events, timeout)
        };
        if res < 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(res as usize)
    }

        // https://github.com/rust-lang/libc/blob/c8aa8ec72d631bc35099bcf5d634cf0a0b841be0/src/unix/bsd/apple/mod.rs#L497
        // https://github.com/rust-lang/libc/blob/c8aa8ec72d631bc35099bcf5d634cf0a0b841be0/src/unix/bsd/apple/mod.rs#L207
        #[derive(Debug, Clone, Default)]
        #[repr(C)]
        pub struct Kevent {
            pub ident: u64,
            pub filter: i16,
            pub flags: u16,
            pub fflags: u32,
            pub data: i64,
            pub udata: u64,
        }

        #[link(name = "c")]
        extern "C" {
            /// Returns: positive: file descriptor, negative: error
            pub(super) fn kqueue() -> i32;
            /// Returns: nothing, all non zero return values is an error
            pub(super) fn kevent(
                kq: i32,
                changelist: *const Kevent,
                nchanges: i32,
                eventlist: *mut Kevent,
                nevents: i32,
                timeout: usize,
            ) -> i32;
        }
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::io::AsRawFd;
    use crate::interests::Interests;
    #[test]
    fn create_kevent_works() {
        let selector = Selector::new_with_id(1).unwrap();
        let sock = std::net::TcpStream::connect("www.google.com:80").unwrap();
        sock.set_nonblocking(true).expect("Setting socket to nonblocking.");
        let fd = sock.as_raw_fd();
        selector.register(fd, 1, Interests::readable()).unwrap();
    }

     #[test]
    fn select_kevent_works() {
        let selector = Selector::new_with_id(1).unwrap();
        let mut sock: TcpStream = TcpStream::connect("slowwly.robertomurray.co.uk:80").unwrap();
        let request =
            "GET /delay/1000/url/http://www.google.com HTTP/1.1\r\n\
             Host: slowwly.robertomurray.co.uk\r\n\
             Connection: close\r\n\
             \r\n";
        sock
            .write_all(request.as_bytes())
            .expect("Error writing to stream");

        let fd = sock.as_raw_fd();
        selector.register(fd, 99, Interests::readable()).unwrap();

        let mut events = vec![Event::zero()];

        selector.select(&mut events).expect("waiting for event.");

        assert_eq!(events[0].udata, 99);
    }

       #[test]
    fn read_kevent_works() {
        let selector = Selector::new_with_id(1).unwrap();
        let mut sock: TcpStream = TcpStream::connect("slowwly.robertomurray.co.uk:80").unwrap();
        let request =
            "GET /delay/1000/url/http://www.google.com HTTP/1.1\r\n\
             Host: slowwly.robertomurray.co.uk\r\n\
             Connection: close\r\n\
             \r\n";
        sock
            .write_all(request.as_bytes())
            .expect("Error writing to stream");

        let fd = sock.as_raw_fd();
        selector.register(fd, 100, Interests::readable()).unwrap();

        let mut events = vec![Event::zero()];

        selector.select(&mut events).expect("waiting for event.");

        let mut buff = String::new();
        assert!(buff.is_empty());
        (&sock).read_to_string(&mut buff).expect("Reading to string.");
        
        assert_eq!(events[0].udata, 100);
        println!("{}", &buff);
        assert!(!buff.is_empty());
    }
}